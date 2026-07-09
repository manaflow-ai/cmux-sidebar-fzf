mod matcher;
mod tree;
mod ui;

use std::{
    env, io,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Result;
use cmux_client::{ClientConfig, CmuxClient};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tree::{FlatRow, RowKind, flatten_tree};

const REFRESH_EVERY: Duration = Duration::from_secs(2);
const POLL_EVERY: Duration = Duration::from_millis(100);
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_millis(500);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(8);

fn main() -> Result<()> {
    // Restore the terminal before the default panic output so a panic never
    // leaves the host terminal (or the cmux sidebar PTY) stuck in raw mode +
    // alternate screen with the message swallowed.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));
    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();
    app.connect_or_schedule();

    loop {
        terminal.draw(|frame| ui::draw(frame, &app.view()))?;

        if event::poll(POLL_EVERY)?
            && let Event::Key(key) = event::read()?
            && app.handle_key(key)
        {
            break;
        }

        app.tick();
    }

    Ok(())
}

struct App {
    query: String,
    rows: Vec<FlatRow>,
    filtered: Vec<FilteredRow>,
    selected: usize,
    client: Option<CmuxClient>,
    socket_path: Option<PathBuf>,
    status: Status,
    last_refresh: Instant,
    next_reconnect: Instant,
    reconnect_delay: Duration,
}

#[derive(Debug, Clone)]
enum Status {
    Ready,
    Reconnecting { message: String },
}

#[derive(Debug, Clone)]
struct FilteredRow {
    row_index: usize,
    match_positions: Vec<usize>,
    score: i64,
}

impl App {
    fn new() -> Self {
        Self {
            query: String::new(),
            rows: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            client: None,
            socket_path: None,
            status: Status::Reconnecting {
                message: "connecting".to_string(),
            },
            last_refresh: Instant::now(),
            next_reconnect: Instant::now(),
            reconnect_delay: INITIAL_RECONNECT_DELAY,
        }
    }

    fn view(&self) -> ui::View<'_> {
        let rows = self
            .filtered
            .iter()
            .map(|filtered| ui::VisibleRow {
                row: &self.rows[filtered.row_index],
                match_positions: &filtered.match_positions,
            })
            .collect();

        ui::View {
            query: &self.query,
            rows,
            selected: self.selected,
            total_rows: self.rows.len(),
            status: match &self.status {
                Status::Ready => ui::ViewStatus::Ready,
                Status::Reconnecting { message } => ui::ViewStatus::Reconnecting { message },
            },
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }

        match key.code {
            KeyCode::Esc => {
                if !self.query.is_empty() {
                    self.query.clear();
                    self.apply_filter();
                }
            }
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(-1)
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(-1)
            }
            KeyCode::Down => self.move_selection(1),
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(1)
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(1)
            }
            KeyCode::Enter => self.activate_selected(),
            KeyCode::Backspace => {
                self.query.pop();
                self.apply_filter();
            }
            KeyCode::Delete => {
                self.query.clear();
                self.apply_filter();
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.query.push(ch);
                self.apply_filter();
            }
            _ => {}
        }

        false
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.client.is_none() {
            if now >= self.next_reconnect {
                self.connect_or_schedule();
            }
            return;
        }

        if now.duration_since(self.last_refresh) >= REFRESH_EVERY {
            self.refresh_tree();
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.filtered.len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = self.selected.saturating_add_signed(delta).min(len - 1);
    }

    fn activate_selected(&mut self) {
        let Some(filtered) = self.filtered.get(self.selected) else {
            return;
        };
        let row = self.rows[filtered.row_index].clone();
        let result = match self.client.as_mut() {
            Some(client) => activate_row(client, &row),
            None => return,
        };

        match result {
            Ok(()) => self.refresh_tree(),
            Err(err) => self.disconnect(format!("cmux command failed: {err}")),
        }
    }

    fn connect_or_schedule(&mut self) {
        let socket_path = match env::var_os("CMUX_MUX_SOCKET") {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            _ => {
                self.socket_path = None;
                self.disconnect_with_backoff(
                    "CMUX_MUX_SOCKET is not set. Launch this plugin from cmux, or run standalone with CMUX_MUX_SOCKET=/path/to/cmux-mux.sock cargo run.".to_string(),
                );
                return;
            }
        };

        self.socket_path = Some(socket_path.clone());
        match CmuxClient::connect(ClientConfig::from_socket_path(socket_path)) {
            Ok(mut client) => match client.identify().and_then(|_| client.list_workspaces()) {
                Ok(tree) => {
                    self.client = Some(client);
                    self.status = Status::Ready;
                    self.reconnect_delay = INITIAL_RECONNECT_DELAY;
                    self.last_refresh = Instant::now();
                    self.rows = flatten_tree(&tree);
                    self.apply_filter();
                }
                Err(err) => self.disconnect_with_backoff(format!("cmux did not respond: {err}")),
            },
            Err(err) => self.disconnect_with_backoff(format!("cannot connect to cmux: {err}")),
        }
    }

    fn refresh_tree(&mut self) {
        let result = match self.client.as_mut() {
            Some(client) => client.list_workspaces(),
            None => return,
        };

        match result {
            Ok(tree) => {
                self.rows = flatten_tree(&tree);
                self.last_refresh = Instant::now();
                self.apply_filter();
            }
            Err(err) => self.disconnect(format!("cmux socket dropped: {err}")),
        }
    }

    fn disconnect(&mut self, message: String) {
        self.client = None;
        self.disconnect_with_backoff(message);
    }

    fn disconnect_with_backoff(&mut self, message: String) {
        self.status = Status::Reconnecting { message };
        self.next_reconnect = Instant::now() + self.reconnect_delay;
        self.reconnect_delay = (self.reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
    }

    fn apply_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.rows.len())
                .map(|row_index| FilteredRow {
                    row_index,
                    match_positions: Vec::new(),
                    score: 0,
                })
                .collect();
        } else {
            let mut matches = self
                .rows
                .iter()
                .enumerate()
                .filter_map(|(row_index, row)| {
                    matcher::fuzzy_match(&row.label, &self.query).map(|match_result| FilteredRow {
                        row_index,
                        match_positions: match_result.positions,
                        score: match_result.score,
                    })
                })
                .collect::<Vec<_>>();
            matches.sort_by(|a, b| b.score.cmp(&a.score).then(a.row_index.cmp(&b.row_index)));
            self.filtered = matches;
        }

        if self.filtered.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.filtered.len() - 1);
        }
    }
}

fn activate_row(client: &mut CmuxClient, row: &FlatRow) -> cmux_client::Result<()> {
    client.select_workspace(Some(row.workspace_index), None)?;
    match row.kind {
        RowKind::Workspace => {}
        RowKind::Screen => {
            client.select_screen(Some(row.screen_index), None)?;
        }
        RowKind::Pane => {
            client.select_screen(Some(row.screen_index), None)?;
            client.focus_pane(row.id)?;
        }
    }
    Ok(())
}

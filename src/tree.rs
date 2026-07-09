use cmux_client::{Pane, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Workspace,
    Screen,
    Pane,
}

impl RowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Screen => "screen",
            Self::Pane => "pane",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatRow {
    pub kind: RowKind,
    pub id: u64,
    pub workspace_index: usize,
    pub screen_index: usize,
    pub label: String,
    pub active: bool,
}

pub fn flatten_tree(tree: &Tree) -> Vec<FlatRow> {
    let mut rows = Vec::new();

    for (workspace_index, workspace) in tree.workspaces.iter().enumerate() {
        let workspace_name = workspace.name.clone();
        rows.push(FlatRow {
            kind: RowKind::Workspace,
            id: workspace.id,
            workspace_index,
            screen_index: 0,
            label: workspace_name.clone(),
            active: workspace.active,
        });

        for (screen_index, screen) in workspace.screens.iter().enumerate() {
            let screen_name = screen
                .name
                .clone()
                .unwrap_or_else(|| format!("screen {}", screen_index + 1));
            let screen_label = format!("{workspace_name} > {screen_name}");
            rows.push(FlatRow {
                kind: RowKind::Screen,
                id: screen.id,
                workspace_index,
                screen_index,
                label: screen_label.clone(),
                active: workspace.active && screen.active,
            });

            for pane in &screen.panes {
                rows.push(FlatRow {
                    kind: RowKind::Pane,
                    id: pane.id,
                    workspace_index,
                    screen_index,
                    label: format!("{screen_label} > {}", pane_title(pane)),
                    active: workspace.active && screen.active && pane.id == screen.active_pane,
                });
            }
        }
    }

    rows
}

fn pane_title(pane: &Pane) -> String {
    if let Some(name) = pane.name.as_ref().filter(|name| !name.is_empty()) {
        return name.clone();
    }

    if let Some(tab) = pane.tabs.get(pane.active_tab).or_else(|| pane.tabs.first()) {
        if let Some(name) = tab.name.as_ref().filter(|name| !name.is_empty()) {
            return name.clone();
        }
        if !tab.title.is_empty() {
            return tab.title.clone();
        }
        return format!("{} tab", tab.kind);
    }

    format!("pane {}", pane.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmux_client::{Layout, Pane, Screen, Tab, Workspace};

    #[test]
    fn flattens_workspaces_screens_and_panes_in_order() {
        let tree = sample_tree();
        let rows = flatten_tree(&tree);

        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].kind, RowKind::Workspace);
        assert_eq!(rows[0].label, "alpha");
        assert_eq!(rows[1].kind, RowKind::Screen);
        assert_eq!(rows[1].label, "alpha > editor");
        assert_eq!(rows[2].kind, RowKind::Pane);
        assert_eq!(rows[2].label, "alpha > editor > shell");
        assert_eq!(rows[3].label, "alpha > editor > npm test");
        assert_eq!(rows[4].label, "alpha > screen 2");
    }

    #[test]
    fn keeps_workspace_and_screen_indices_for_activation() {
        let tree = sample_tree();
        let rows = flatten_tree(&tree);

        assert_eq!(rows[3].workspace_index, 0);
        assert_eq!(rows[3].screen_index, 0);
        assert_eq!(rows[3].id, 12);
        assert!(rows[2].active);
        assert!(!rows[3].active);
    }

    fn sample_tree() -> Tree {
        Tree {
            workspaces: vec![Workspace {
                id: 1,
                name: "alpha".to_string(),
                active: true,
                screens: vec![
                    Screen {
                        id: 10,
                        name: Some("editor".to_string()),
                        active: true,
                        active_pane: 11,
                        layout: Layout::Leaf { pane: 11 },
                        panes: vec![
                            Pane {
                                id: 11,
                                name: Some("shell".to_string()),
                                active_tab: 0,
                                tabs: vec![],
                                dead: false,
                            },
                            Pane {
                                id: 12,
                                name: None,
                                active_tab: 0,
                                tabs: vec![Tab {
                                    surface: 20,
                                    kind: "terminal".to_string(),
                                    browser_source: None,
                                    name: None,
                                    title: "npm test".to_string(),
                                    size: None,
                                    dead: false,
                                }],
                                dead: false,
                            },
                        ],
                    },
                    Screen {
                        id: 13,
                        name: None,
                        active: false,
                        active_pane: 14,
                        layout: Layout::Leaf { pane: 14 },
                        panes: vec![],
                    },
                ],
            }],
        }
    }
}

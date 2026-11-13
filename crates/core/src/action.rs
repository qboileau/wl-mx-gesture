use serde::Deserialize;

use crate::workspace::Side;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(
    rename_all = "snake_case",
    deny_unknown_fields,
    expecting = "an action: `{ exec = \"…\" }`, `{ hyprland = \"…\" }`, `{ workspace = \"…\" }`, \
                 `{ move_to_workspace = \"…\" }`, `{ adjacent_workspace = \"left|right\" }`, \
                 `{ move_to_adjacent_workspace = \"left|right\" }`, `{ toggle_special = \"…\" }`, \"close_window\", \
                 \"toggle_floating\", \"fullscreen\" or \"maximize\""
)]
pub enum Action {
    /// Shell command run with `sh -c`.
    Exec(String),
    /// Raw Hyprland dispatch, sent verbatim: its syntax must match the running Hyprland version.
    Hyprland(String),
    #[serde(untagged)]
    Compositor(CompositorAction),
}

/// Window management intents, translated by the compositor adapter into its own protocol.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CompositorAction {
    /// Workspace id, name, `special:name` or relative reference such as `e+1`.
    Workspace(String),
    /// Moves the focused window to a workspace and follows it.
    MoveToWorkspace(String),
    /// Workspace beside the current one, following the configured multi-monitor navigation.
    AdjacentWorkspace(Side),
    /// Moves the focused window to the adjacent workspace and follows it.
    MoveToAdjacentWorkspace(Side),
    ToggleSpecial(String),
    CloseWindow,
    ToggleFloating,
    Fullscreen,
    Maximize,
}

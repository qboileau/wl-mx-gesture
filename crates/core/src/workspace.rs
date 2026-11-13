//! Picking the workspace next to the current one, across monitors.

use serde::Deserialize;

/// How `adjacent_workspace` steps through workspaces when several monitors are connected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Navigation {
    /// Every workspace by number, whatever its monitor, wrapping around.
    Global,
    /// Workspaces of the focused monitor only, wrapping around.
    #[default]
    Monitor,
    /// Workspaces of the focused monitor, then the active workspace of the neighbouring monitor on that side.
    Spatial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    /// Left edge in the global layout.
    pub x: i32,
    pub y: i32,
    pub active: Workspace,
    /// Regular workspaces shown on this monitor, the active one included.
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub focused: Monitor,
    pub others: Vec<Monitor>,
}

/// The workspace to go to, or `None` when there is none other than the current one on that side.
pub fn adjacent(layout: &Layout, navigation: Navigation, side: Side) -> Option<&Workspace> {
    let current = &layout.focused.active;
    let target = match navigation {
        Navigation::Global => {
            let all = layout.others.iter().chain([&layout.focused]).flat_map(|monitor| &monitor.workspaces);
            cycle(sorted(all), current, side)
        }
        Navigation::Monitor => cycle(sorted(&layout.focused.workspaces), current, side),
        Navigation::Spatial => step(&sorted(&layout.focused.workspaces), current, side)
            .or_else(|| neighbour(layout, side).map(|monitor| &monitor.active)),
    }?;
    (target != current).then_some(target)
}

fn sorted<'a>(workspaces: impl IntoIterator<Item = &'a Workspace>) -> Vec<&'a Workspace> {
    let mut workspaces: Vec<_> = workspaces.into_iter().collect();
    workspaces.sort_by_key(|workspace| workspace.id);
    workspaces
}

fn step<'a>(ordered: &[&'a Workspace], current: &Workspace, side: Side) -> Option<&'a Workspace> {
    let index = ordered.iter().position(|workspace| *workspace == current)?;
    let index = match side {
        Side::Left => index.checked_sub(1)?,
        Side::Right => index + 1,
    };
    ordered.get(index).copied()
}

fn cycle<'a>(ordered: Vec<&'a Workspace>, current: &Workspace, side: Side) -> Option<&'a Workspace> {
    step(&ordered, current, side).or_else(|| match side {
        Side::Left => ordered.last().copied(),
        Side::Right => ordered.first().copied(),
    })
}

/// Closest monitor whose left edge lies on that side, preferring the most vertically aligned one.
fn neighbour(layout: &Layout, side: Side) -> Option<&Monitor> {
    let focused = &layout.focused;
    layout
        .others
        .iter()
        .filter(|monitor| match side {
            Side::Left => monitor.x < focused.x,
            Side::Right => monitor.x > focused.x,
        })
        .min_by_key(|monitor| (monitor.x.abs_diff(focused.x), monitor.y.abs_diff(focused.y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(id: i64) -> Workspace {
        Workspace { id, name: id.to_string() }
    }

    fn monitor(x: i32, active: i64, ids: &[i64]) -> Monitor {
        Monitor { x, y: 0, active: workspace(active), workspaces: ids.iter().copied().map(workspace).collect() }
    }

    /// Left monitor holds 1 and 4, right monitor holds 2, 3 and 7 with 3 active.
    fn two_monitors(focused_active: i64) -> Layout {
        Layout { focused: monitor(0, focused_active, &[4, 1]), others: vec![monitor(2560, 3, &[2, 3, 7])] }
    }

    fn target(layout: &Layout, navigation: Navigation, side: Side) -> Option<i64> {
        adjacent(layout, navigation, side).map(|workspace| workspace.id)
    }

    #[test]
    fn spatial_walks_the_monitor_then_enters_the_neighbour_on_its_active_workspace() {
        assert_eq!(target(&two_monitors(1), Navigation::Spatial, Side::Right), Some(4));
        assert_eq!(target(&two_monitors(4), Navigation::Spatial, Side::Right), Some(3));
        assert_eq!(target(&two_monitors(4), Navigation::Spatial, Side::Left), Some(1));
    }

    #[test]
    fn spatial_stops_at_the_outer_edge() {
        assert_eq!(target(&two_monitors(1), Navigation::Spatial, Side::Left), None);
    }

    #[test]
    fn monitor_wraps_within_the_focused_monitor() {
        assert_eq!(target(&two_monitors(4), Navigation::Monitor, Side::Right), Some(1));
        assert_eq!(target(&two_monitors(1), Navigation::Monitor, Side::Left), Some(4));
    }

    #[test]
    fn global_orders_every_workspace_by_number() {
        assert_eq!(target(&two_monitors(1), Navigation::Global, Side::Right), Some(2));
        assert_eq!(target(&two_monitors(4), Navigation::Global, Side::Right), Some(7));
        assert_eq!(target(&two_monitors(1), Navigation::Global, Side::Left), Some(7));
    }

    #[test]
    fn a_lone_workspace_has_no_neighbour() {
        let layout = Layout { focused: monitor(0, 1, &[1]), others: vec![] };
        assert_eq!(target(&layout, Navigation::Monitor, Side::Right), None);
        assert_eq!(target(&layout, Navigation::Global, Side::Left), None);
    }

    #[test]
    fn spatial_picks_the_closest_monitor_on_that_side() {
        let layout = Layout {
            focused: monitor(1920, 2, &[2]),
            others: vec![monitor(-1920, 9, &[9]), monitor(0, 1, &[1]), monitor(3840, 3, &[3])],
        };
        assert_eq!(target(&layout, Navigation::Spatial, Side::Left), Some(1));
        assert_eq!(target(&layout, Navigation::Spatial, Side::Right), Some(3));
    }
}

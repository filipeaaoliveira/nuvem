//! Semantic-role style accessors. These read the process-wide active theme
//! (see [`crate::config::theme`]) so the ~55 call sites across the UI layer
//! never need to know about config, themes, or the arc-swap underneath.

use ratatui::style::Style;

use crate::config::theme::active;

pub fn app_title() -> Style {
    active().app_title
}

pub fn view_title() -> Style {
    active().view_title
}

pub fn title_style() -> Style {
    view_title()
}

pub fn border() -> Style {
    active().border
}

pub fn focused_border() -> Style {
    active().focused_border
}

pub fn selected_row() -> Style {
    active().selected_row
}

pub fn inactive_selected_row() -> Style {
    active().inactive_selected_row
}

pub fn active_tab() -> Style {
    active().active_tab
}

pub fn inactive_tab() -> Style {
    active().inactive_tab
}

pub fn status_healthy() -> Style {
    active().status_healthy
}

pub fn status_loading() -> Style {
    active().status_loading
}

pub fn status_stale() -> Style {
    active().status_stale
}

pub fn status_error() -> Style {
    active().status_error
}

pub fn muted() -> Style {
    active().muted
}

pub fn key_hint() -> Style {
    active().key_hint
}

pub fn overlay_border() -> Style {
    active().overlay_border
}

/// A value that has just changed. See [`crate::config::theme::Theme::delta`].
pub fn delta() -> Style {
    active().delta
}

//! Config-selectable themes.
//!
//! [`Theme`] holds one [`Style`] per semantic role used across `ui::theme`.
//! Three built-ins are provided (`default`, `no-color`, `high-contrast`); the
//! active one is chosen via [`ThemeChoice`] and stored in a lock-free
//! [`arc_swap::ArcSwap`] global so the ~55 `ui::theme::*` call sites can read
//! it without taking a lock on every frame.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;
use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

use crate::config::problems::{ConfigProblemSource, ConfigProblems};

/// The set of built-in themes selectable from config (`[general].theme`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeChoice {
    #[default]
    Default,
    NoColor,
    HighContrast,
}

impl ThemeChoice {
    /// Parse a theme name leniently: case-insensitive, `-`/`_` interchangeable.
    /// Returns `None` for unrecognized values so callers can warn and fall
    /// back to the default theme.
    pub fn from_str_lenient(raw: &str) -> Option<Self> {
        let normalized = raw.to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "default" => Some(ThemeChoice::Default),
            "no-color" | "nocolor" => Some(ThemeChoice::NoColor),
            "high-contrast" | "highcontrast" => Some(ThemeChoice::HighContrast),
            _ => None,
        }
    }

    /// The canonical, stable name for this theme (used by `nuvem info`).
    pub fn name(&self) -> &'static str {
        match self {
            ThemeChoice::Default => "default",
            ThemeChoice::NoColor => "no-color",
            ThemeChoice::HighContrast => "high-contrast",
        }
    }
}

/// One [`Style`] per semantic role used across `ui::theme`. Pure data, no
/// global state — unit-testable on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub app_title: Style,
    pub view_title: Style,
    pub border: Style,
    pub focused_border: Style,
    pub selected_row: Style,
    pub inactive_selected_row: Style,
    pub active_tab: Style,
    pub inactive_tab: Style,
    pub status_healthy: Style,
    pub status_loading: Style,
    pub status_stale: Style,
    pub status_error: Style,
    pub muted: Style,
    pub key_hint: Style,
    pub overlay_border: Style,
    /// A value that has just changed. Its own role rather than a
    /// reuse of `status_loading`: "in flight" and "just moved" are different
    /// claims, and a theme that wanted to mute one without muting the other
    /// could not.
    pub delta: Style,
}

impl Theme {
    /// The current `ui::theme` colors, copied verbatim so default rendering
    /// stays pixel-identical.
    pub fn default_theme() -> Self {
        Self {
            app_title: Style::default().fg(Color::Cyan),
            view_title: Style::default().fg(Color::Cyan),
            border: Style::default().fg(Color::DarkGray),
            focused_border: Style::default().fg(Color::Cyan),
            selected_row: Style::default().bg(Color::Blue),
            inactive_selected_row: Style::default().bg(Color::DarkGray),
            active_tab: Style::default().fg(Color::Yellow),
            inactive_tab: Style::default().fg(Color::DarkGray),
            status_healthy: Style::default().fg(Color::Green),
            status_loading: Style::default().fg(Color::Yellow),
            status_stale: Style::default().fg(Color::LightYellow),
            status_error: Style::default().fg(Color::Red),
            muted: Style::default().fg(Color::DarkGray),
            key_hint: Style::default().fg(Color::Cyan),
            overlay_border: Style::default().fg(Color::Magenta),
            delta: Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        }
    }

    /// No fg/bg colors on any field; relies on `Modifier` (reverse video,
    /// bold, underline) plus the app's `[OK]`/`[ERR]` text markers for
    /// emphasis. This is the accessibility / no-color option.
    pub fn no_color() -> Self {
        let plain = Style::default();
        let emphasis = Style::default().add_modifier(Modifier::BOLD);
        let underline = Style::default().add_modifier(Modifier::UNDERLINED);
        let reversed = Style::default().add_modifier(Modifier::REVERSED);

        Self {
            app_title: emphasis,
            view_title: emphasis,
            border: plain,
            focused_border: underline,
            selected_row: reversed,
            inactive_selected_row: plain,
            active_tab: underline,
            inactive_tab: plain,
            status_healthy: emphasis,
            status_loading: plain,
            status_stale: plain,
            status_error: emphasis,
            muted: plain,
            key_hint: underline,
            overlay_border: emphasis,
            delta: emphasis,
        }
    }

    /// Bright, maximally-distinct fg colors + bold on statuses.
    pub fn high_contrast() -> Self {
        Self {
            app_title: Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            view_title: Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            border: Style::default().fg(Color::White),
            focused_border: Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            selected_row: Style::default()
                .bg(Color::LightMagenta)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
            inactive_selected_row: Style::default().bg(Color::White).fg(Color::Black),
            active_tab: Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
            inactive_tab: Style::default().fg(Color::White),
            status_healthy: Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
            status_loading: Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
            status_stale: Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
            status_error: Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
            muted: Style::default().fg(Color::White),
            key_hint: Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
            overlay_border: Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
            delta: Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        }
    }

    /// This theme with both border roles recolored to `color`.
    ///
    /// Only the foreground is replaced: `Modifier`s survive, so the
    /// `no-color` theme keeps its underlined focus marker and `high-contrast`
    /// keeps its bold, and the override changes the hue without undoing the
    /// base theme's non-color emphasis.
    ///
    /// Both roles move together on purpose. Recoloring only `border` would
    /// leave the focused pane on the base theme's accent, so the frame would
    /// read as overridden only while unfocused — a guardrail that is on half
    /// the time teaches the user it cannot be trusted.
    pub fn with_border_color(self, color: Color) -> Self {
        Self {
            border: self.border.fg(color),
            focused_border: self.focused_border.fg(color),
            ..self
        }
    }

    /// Resolve a built-in [`Theme`] for the given [`ThemeChoice`].
    pub fn for_choice(choice: ThemeChoice) -> Self {
        match choice {
            ThemeChoice::Default => Self::default_theme(),
            ThemeChoice::NoColor => Self::no_color(),
            ThemeChoice::HighContrast => Self::high_contrast(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::default_theme()
    }
}

/// The validated frame overrides: a border color per gcloud configuration
/// name from `[theme.overrides]`, and per project id from
/// `[theme.project_overrides]`. Together the "prod is unmistakable"
/// guardrail — the project key is the one that holds when the operator's prod
/// is reached through a configuration named `default`.
///
/// Empty when the file has no `[theme]` table, which is what keeps an
/// unconfigured user's frame byte-for-byte what it is today.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemeOverrides {
    /// Configuration name -> border color. A `BTreeMap` so the warnings the
    /// validator emits come out in a deterministic order, matching the
    /// ordering promise `ConfigProblems` documents.
    entries: BTreeMap<String, Color>,
    /// Project id -> border color, from `[theme.project_overrides]`.
    /// A second map rather than a second kind of key in `entries`: a project
    /// id and a configuration name can legitimately be the same string, so a
    /// heuristic over one namespace could not tell them apart.
    projects: BTreeMap<String, Color>,
}

/// The keys `[theme]` accepts. Named as a set so an unknown key warns with
/// the same wording the other sections use.
pub const THEME_KEYS: [&str; 2] = ["overrides", "project_overrides"];

/// The `[theme]` key holding project-id overrides. Named as a constant
/// because both the parser and the warning that redirects the nested
/// spelling have to agree on it.
const PROJECT_OVERRIDES_KEY: &str = "project_overrides";

/// The nested spelling once proposed, kept only to redirect whoever writes
/// it. See the branch in [`ThemeOverrides::from_file`] for why it is not the
/// spelling that exists.
const PROJECT_OVERRIDES_NESTED_GUESS: &str = "projects";

impl ThemeOverrides {
    // No `empty()` constructor, unlike its `[favorites]` and `[columns]`
    // siblings. Theirs predate this file; a wrapper whose body is exactly
    // `Self::default()` is indistinguishable from the derived `Default`, so
    // mutation testing reports replacing it as a surviving mutant that no
    // test can ever kill. `ThemeOverrides::default()` says the same thing and
    // leaves nothing equivalent to mutate.

    /// Parse and validate the raw `[theme]` table. Each invalid entry is
    /// dropped and produces a warning; the remaining valid entries are kept.
    ///
    /// Validation rules:
    /// - the section itself must be a table; anything else drops the whole
    ///   section with one warning.
    /// - each key must be one of [`THEME_KEYS`]; an unknown key is dropped
    ///   with a warning, so writing `[theme] active = ...` (the base theme's
    ///   key, which lives at `[general].theme`) is told rather than ignored.
    /// - `overrides` must be a table of `configuration name -> color string`.
    /// - a name that is blank once trimmed, a non-string value, or a color
    ///   ratatui cannot parse drops that entry alone; its siblings are kept.
    ///
    /// A well-formed name that matches no *existing* gcloud configuration
    /// cannot be judged here: the configuration list is fetched from `gcloud`
    /// long after the config is resolved. That check is deferred to
    /// `RuntimeSession`, which re-runs it once the configurations are known.
    pub fn from_file(raw: &toml::Value, problems: &mut ConfigProblems) -> Self {
        let mut overrides = Self::default();

        let Some(table) = raw.as_table() else {
            // Points at `[general].theme` for the same reason the unknown-key
            // branch below does: `theme = "dark"` at top level is the likeliest
            // confusion now that a `[theme]` section exists.
            problems.push(
                ConfigProblemSource::Theme,
                format!(
                    "theme: expected a table, found {}; ignoring (the base theme \
                     is set by general.theme)",
                    raw.type_str()
                ),
            );
            return overrides;
        };

        for (key, value) in table {
            if !THEME_KEYS.contains(&key.as_str()) {
                // Names `[general].theme` rather than only reporting the key
                // as wrong: `[theme]` sits next to the base-theme setting and
                // picking the wrong one of the two is the likeliest mistake
                // here, so the warning points at the key the user wanted.
                problems.push(
                    ConfigProblemSource::Theme,
                    format!(
                        "theme.{key}: unknown key; skipping (the base theme is \
                         set by general.theme)"
                    ),
                );
                continue;
            }

            // Both tables share this loop, so the noun comes from the key:
            // a warning about a project table that says "configuration name"
            // sends the reader to the other table to fix a key that is not
            // there.
            let vocabulary = if key == PROJECT_OVERRIDES_KEY {
                "project ids"
            } else {
                "configuration names"
            };

            let Some(entries) = value.as_table() else {
                problems.push(
                    ConfigProblemSource::Theme,
                    format!(
                        "theme.{key}: expected a table of {vocabulary} to colors, found {}; skipping",
                        value.type_str()
                    ),
                );
                continue;
            };

            for (name, raw_color) in entries {
                // `[theme.overrides.projects]` is the natural guess for the
                // project table and is not what exists: reserving `projects`
                // inside a table of user-chosen configuration names would
                // make a configuration of that name uncolourable. Reaching
                // here means the user wrote the nested spelling, so point at
                // the sibling table rather than only reporting a table where
                // a color string was expected.
                if key != PROJECT_OVERRIDES_KEY
                    && name == PROJECT_OVERRIDES_NESTED_GUESS
                    && raw_color.is_table()
                {
                    problems.push(
                        ConfigProblemSource::Theme,
                        format!(
                            "theme.{key}.{name}: project overrides live in \
                             [theme.{PROJECT_OVERRIDES_KEY}], not nested here; skipping"
                        ),
                    );
                    continue;
                }

                let Some(text) = raw_color.as_str() else {
                    problems.push(
                        ConfigProblemSource::Theme,
                        format!(
                            "theme.{key}.{name}: color must be a string, found {}; skipping",
                            raw_color.type_str()
                        ),
                    );
                    continue;
                };

                // Trimmed but never case-folded: gcloud configuration names
                // are matched verbatim against what `gcloud` reports.
                let name = name.trim();
                if name.is_empty() {
                    let noun = if key == PROJECT_OVERRIDES_KEY {
                        "project id"
                    } else {
                        "configuration name"
                    };
                    problems.push(
                        ConfigProblemSource::Theme,
                        format!("theme.{key}: {noun} is empty; skipping"),
                    );
                    continue;
                }

                // ratatui's own `FromStr` is the parser, so the spellings a
                // user may write here are exactly the ones the rest of the
                // ecosystem accepts (`red`, `light-red`, `#ff0000`, `9`)
                // rather than a narrower hand-rolled list.
                let Ok(color) = text.trim().parse::<Color>() else {
                    problems.push(
                        ConfigProblemSource::Theme,
                        format!("theme.{key}.{name}: unknown color {text:?}; skipping"),
                    );
                    continue;
                };

                if key == PROJECT_OVERRIDES_KEY {
                    overrides.projects.insert(name.to_string(), color);
                } else {
                    overrides.entries.insert(name.to_string(), color);
                }
            }
        }

        overrides
    }

    /// The override color for `configuration`, when one is configured.
    fn border_color(&self, configuration: &str) -> Option<Color> {
        self.entries.get(configuration).copied()
    }

    /// The frame color for the active context: the configuration's own name,
    /// falling back to the project it points at.
    ///
    /// The configuration name wins because it is the narrower key — several
    /// configurations can point at one project, so naming one of them is the
    /// more specific statement, and the project override is the catch-all for
    /// every configuration the operator never named. Stated here rather than
    /// left to fall out of a lookup order, which two `BTreeMap`s do not have
    /// between them.
    pub fn border_color_for(&self, configuration: &str, project: Option<&str>) -> Option<Color> {
        self.border_color(configuration)
            .or_else(|| project.and_then(|project| self.projects.get(project).copied()))
    }

    /// Every configuration name an override is written for, in sorted order.
    /// The vocabulary the deferred "names nothing" check reads.
    pub fn configurations(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Every project id an override is written for, in sorted order. The
    /// vocabulary the deferred "targets nothing" check reads, beside
    /// [`Self::configurations`].
    pub fn projects(&self) -> impl Iterator<Item = &str> {
        self.projects.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.projects.is_empty()
    }
}

static ACTIVE: LazyLock<ArcSwap<Theme>> =
    LazyLock::new(|| ArcSwap::from_pointee(Theme::default_theme()));

/// The currently active theme. Lock-free: an atomic load per frame, no
/// contention with concurrent readers or a reload's [`set_active`] store.
pub fn active() -> arc_swap::Guard<Arc<Theme>> {
    ACTIVE.load()
}

/// Atomically swap in a new active theme (e.g. at startup, or on a config
/// reload command).
pub fn set_active(theme: Theme) {
    ACTIVE.store(Arc::new(theme));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_choice_default_is_default_variant() {
        assert_eq!(ThemeChoice::default(), ThemeChoice::Default);
    }

    #[test]
    fn theme_choice_name_round_trips_through_parse() {
        for choice in [
            ThemeChoice::Default,
            ThemeChoice::NoColor,
            ThemeChoice::HighContrast,
        ] {
            assert_eq!(ThemeChoice::from_str_lenient(choice.name()), Some(choice));
        }
    }

    #[test]
    fn active_defaults_to_default_theme() {
        // Other tests in this binary may have swapped the active theme; only
        // assert the invariant that holds right after a fresh `set_active`.
        set_active(Theme::default_theme());
        assert_eq!(**active(), Theme::default_theme());
    }
}

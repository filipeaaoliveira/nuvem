//! User-favorited GCP resource-scope identifiers: project ids, region names,
//! zone names. Parsed from the `[favorites]` table of the config file and
//! used by the configuration-context picker to order its rows ahead of the
//! rest.

use crate::config::problems::{ConfigProblemSource, ConfigProblems};

/// Which scope kind a favorited value belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FavoriteKind {
    Project,
    Region,
    Zone,
}

/// The `[favorites]` table's keys, paired with the kind each one feeds. The
/// single vocabulary the parser, the "unknown key" check, and documentation
/// tests outside this crate all read from.
pub const KEYS: [(&str, FavoriteKind); 3] = [
    ("projects", FavoriteKind::Project),
    ("regions", FavoriteKind::Region),
    ("zones", FavoriteKind::Zone),
];

/// The validated `[favorites]` table: the user's favorited project ids,
/// region names and zone names, each in the order they were written in the
/// config file. This is the order `values` returns; it is a property of this
/// struct only. Once the picker uses these values to pick favorites out of
/// its own rows, the favorite group keeps the *rows'* order, not this one —
/// see `ConfigurationContextPicker::visible_items`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FavoriteScopes {
    projects: Vec<String>,
    regions: Vec<String>,
    zones: Vec<String>,
}

impl FavoriteScopes {
    /// No favorites at all — the default when there is no config file, and
    /// when the file has no `[favorites]` table. An unconfigured user's
    /// picker order must be byte-for-byte what it is today.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The validated favorites for `kind`, in source order. Values are GCP
    /// resource identifiers (project ids, region and zone names), which are
    /// already lowercase by construction, so they are trimmed but never
    /// case-folded — unlike `[aliases]`, which folds human-typed names.
    pub fn values(&self, kind: FavoriteKind) -> &[String] {
        match kind {
            FavoriteKind::Project => &self.projects,
            FavoriteKind::Region => &self.regions,
            FavoriteKind::Zone => &self.zones,
        }
    }

    fn values_mut(&mut self, kind: FavoriteKind) -> &mut Vec<String> {
        match kind {
            FavoriteKind::Project => &mut self.projects,
            FavoriteKind::Region => &mut self.regions,
            FavoriteKind::Zone => &mut self.zones,
        }
    }

    /// Parse and validate the raw `[favorites]` table. Each invalid entry is
    /// dropped and produces a warning; the remaining valid entries are kept.
    ///
    /// Validation rules:
    /// - the section itself must be a table; anything else drops the whole
    ///   section with one warning.
    /// - each key must be one of `projects`, `regions`, `zones`; an unknown
    ///   key is dropped with a warning.
    /// - each key's value must be an array; a non-array value drops that key
    ///   (its siblings are unaffected).
    /// - each array element must be a non-empty (once trimmed) string; an
    ///   invalid element is dropped and its siblings are kept.
    pub fn from_file(raw: &toml::Value, problems: &mut ConfigProblems) -> Self {
        let mut scopes = Self::empty();

        let Some(table) = raw.as_table() else {
            problems.push(
                ConfigProblemSource::Favorites,
                format!(
                    "favorites: expected a table, found {}; ignoring",
                    raw.type_str()
                ),
            );
            return scopes;
        };

        for (key, value) in table {
            let Some((_, kind)) = KEYS.iter().find(|(name, _)| *name == key) else {
                problems.push(
                    ConfigProblemSource::Favorites,
                    format!("favorites.{key}: unknown key; skipping"),
                );
                continue;
            };

            let Some(items) = value.as_array() else {
                problems.push(
                    ConfigProblemSource::Favorites,
                    format!(
                        "favorites.{key}: expected a list of strings, found {}; skipping",
                        value.type_str()
                    ),
                );
                continue;
            };

            let values = scopes.values_mut(*kind);
            for item in items {
                let Some(text) = item.as_str() else {
                    problems.push(
                        ConfigProblemSource::Favorites,
                        format!(
                            "favorites.{key}: value must be a string, found {}; skipping",
                            item.type_str()
                        ),
                    );
                    continue;
                };

                let trimmed = text.trim();
                if trimmed.is_empty() {
                    problems.push(
                        ConfigProblemSource::Favorites,
                        format!("favorites.{key}: empty value; skipping"),
                    );
                    continue;
                }

                values.push(trimmed.to_string());
            }
        }

        scopes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(body: &str) -> (FavoriteScopes, Vec<String>) {
        let value: toml::Value = toml::from_str(body).expect("test fixture must be valid TOML");
        let mut problems = ConfigProblems::default();
        let scopes = FavoriteScopes::from_file(&value, &mut problems);
        (scopes, problems.into_messages())
    }

    #[test]
    fn valid_section_parses_all_three_kinds() {
        let (favorites, warnings) = scopes(
            r#"
projects = ["acme-prod", "acme-staging"]
regions  = ["europe-west1"]
zones    = ["europe-west1-b"]
"#,
        );
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(
            favorites.values(FavoriteKind::Project),
            &["acme-prod".to_string(), "acme-staging".to_string()]
        );
        assert_eq!(
            favorites.values(FavoriteKind::Region),
            &["europe-west1".to_string()]
        );
        assert_eq!(
            favorites.values(FavoriteKind::Zone),
            &["europe-west1-b".to_string()]
        );
    }

    #[test]
    fn empty_favorites_table_yields_empty() {
        // `FavoriteScopes::default()` (what `ResolvedConfig` starts from when
        // there is no config file at all) must match what parsing an empty
        // `[favorites]` table produces — an unconfigured user's picker order
        // must be byte-for-byte the same however "unconfigured" arose.
        assert_eq!(FavoriteScopes::default(), FavoriteScopes::empty());

        let (favorites, warnings) = scopes("");
        assert!(warnings.is_empty());
        assert_eq!(favorites, FavoriteScopes::empty());
        assert!(favorites.values(FavoriteKind::Project).is_empty());
    }

    #[test]
    fn a_non_array_value_warns_and_drops_that_key_while_others_survive() {
        let (favorites, warnings) = scopes(
            r#"
projects = "acme-prod"
regions  = ["europe-west1"]
"#,
        );
        assert_eq!(warnings.len(), 1, "unexpected warnings: {warnings:?}");
        assert!(
            warnings[0].starts_with("favorites.projects:"),
            "{warnings:?}"
        );
        assert!(favorites.values(FavoriteKind::Project).is_empty());
        assert_eq!(
            favorites.values(FavoriteKind::Region),
            &["europe-west1".to_string()]
        );
    }

    #[test]
    fn a_non_string_array_element_warns_and_drops_that_element_while_siblings_survive() {
        let (favorites, warnings) = scopes(r#"projects = ["acme-prod", 7]"#);
        assert_eq!(warnings.len(), 1, "unexpected warnings: {warnings:?}");
        assert!(
            warnings[0].starts_with("favorites.projects:"),
            "{warnings:?}"
        );
        assert_eq!(
            favorites.values(FavoriteKind::Project),
            &["acme-prod".to_string()]
        );
    }

    #[test]
    fn an_unknown_key_inside_the_table_warns() {
        let (favorites, warnings) = scopes(
            r#"
projects = ["acme-prod"]
bogus = ["nope"]
"#,
        );
        assert_eq!(warnings.len(), 1, "unexpected warnings: {warnings:?}");
        assert!(warnings[0].starts_with("favorites.bogus:"), "{warnings:?}");
        assert_eq!(
            favorites.values(FavoriteKind::Project),
            &["acme-prod".to_string()]
        );
    }

    #[test]
    fn a_whitespace_only_entry_warns_and_drops() {
        let (favorites, warnings) = scopes(r#"regions = ["europe-west1", "   "]"#);
        assert_eq!(warnings.len(), 1, "unexpected warnings: {warnings:?}");
        assert!(
            warnings[0].starts_with("favorites.regions:"),
            "{warnings:?}"
        );
        assert_eq!(
            favorites.values(FavoriteKind::Region),
            &["europe-west1".to_string()]
        );
    }

    #[test]
    fn the_section_not_being_a_table_warns_and_yields_empty() {
        let value = toml::Value::String("nope".to_string());
        let mut problems = ConfigProblems::default();
        let favorites = FavoriteScopes::from_file(&value, &mut problems);
        assert_eq!(problems.len(), 1);
        assert!(problems.messages()[0].starts_with("favorites:"));
        assert_eq!(favorites, FavoriteScopes::empty());
    }

    #[test]
    fn entries_are_trimmed_but_not_case_folded() {
        let (favorites, warnings) = scopes(r#"zones = ["  Europe-West1-B  "]"#);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(
            favorites.values(FavoriteKind::Zone),
            &["Europe-West1-B".to_string()]
        );
    }
}

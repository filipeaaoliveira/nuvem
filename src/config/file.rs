use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// All-optional mirror of the on-disk TOML schema (v1). Every field is
/// optional so a partial file only overrides the fields it mentions; the
/// rest fall back to built-in defaults.
#[derive(Debug, Default, Deserialize)]
pub struct FileConfig {
    pub version: Option<u64>,
    pub general: Option<GeneralSection>,
    /// `[aliases]` table: extra service aliases, `alias -> service id`.
    pub aliases: Option<BTreeMap<String, String>>,
    /// `[hotkeys]` table: rebindable action -> key spec.
    pub hotkeys: Option<BTreeMap<String, String>>,
    /// `[columns.<service>]` tables: per-service column selection and default
    /// sort, keyed by the same lenient service vocabulary `[aliases]` uses.
    ///
    /// Deliberately kept as raw TOML values rather than a typed section
    /// struct: every problem inside a block — an unknown key *and* a key of
    /// the wrong type — is then validated per service by
    /// [`crate::config::columns`], so one bad block warns and falls back on
    /// its own instead of failing deserialization and discarding the whole
    /// file (theme, aliases, hotkeys and all).
    pub columns: Option<BTreeMap<String, toml::Value>>,
    /// `[[jumps]]` rules: user-configured cross-resource jumps, validated by
    /// [`crate::config::jumps`].
    ///
    /// Raw for the same reason `columns` is: a shape slip inside one rule is
    /// then a per-rule warning rather than a deserialization failure that
    /// would discard the whole file.
    ///
    /// Untyped all the way out to the section itself — not `Vec<toml::Value>`
    /// — because `[[jumps]]` is the schema's only array of tables, so writing
    /// `[jumps]` is the likeliest mistake in the file. As a `Vec` that typo is
    /// a deserialization failure, which costs the user their theme, aliases,
    /// hotkeys and columns as well as their jumps.
    pub jumps: Option<toml::Value>,
    /// `[favorites]` table: favorited project ids, region names and zone
    /// names, validated by [`crate::config::favorites`].
    ///
    /// Raw for the same reason `columns` and `jumps` are: a shape slip inside
    /// one key (a string instead of a list, say) is then a per-key warning
    /// rather than a deserialization failure that discards the whole file.
    pub favorites: Option<toml::Value>,
    /// `[[plugins]]` written *here* rather than in the separate plugin file.
    /// Read only so the mistake can be reported: plugins are never
    /// loaded from this file, because it is the file that may be synced,
    /// checked in, or handed over, and that is the whole reason the two are
    /// separate. Without the field serde drops the key silently and the user
    /// gets a config nuvem accepted and a feature that never appears.
    pub plugins: Option<toml::Value>,
    /// `[theme]` table: frame overrides per gcloud configuration under
    /// `[theme.overrides]` and per project id under
    /// `[theme.project_overrides]`, validated by [`crate::config::theme`].
    ///
    /// Distinct from `general.theme`, which picks the *base* theme; this
    /// section only recolors that theme's frame, per gcloud configuration or
    /// per project id.
    ///
    /// Raw for the same reason `columns`, `jumps` and `favorites` are: a shape
    /// slip inside one entry is then a per-entry warning rather than a
    /// deserialization failure that discards the whole file.
    pub theme: Option<toml::Value>,
    /// Every root key the schema does not name.
    ///
    /// Captured rather than denied. `#[serde(deny_unknown_fields)]` would make
    /// an unknown key a *deserialization failure*, which discards the whole
    /// file — theme, aliases, hotkeys and all — and that is exactly the
    /// outcome every raw-`toml::Value` field above exists to avoid. Collecting
    /// them instead lets the file keep working while
    /// [`crate::config::ResolvedConfig::resolve`] warns per key, the way
    /// `[theme]`, `[favorites]` and `[columns]` already warn about theirs.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Default, Deserialize)]
pub struct GeneralSection {
    pub poll_interval_secs: Option<u64>,
    pub default_service: Option<String>,
    pub sidebar_visible: Option<bool>,
    pub theme: Option<String>,
    pub read_only: Option<bool>,
    /// Unknown `[general]` keys, captured for the same reason as
    /// [`FileConfig::unknown`]. This is where a near miss like
    /// `poll_interval_sec` lands — one character from a real setting, and
    /// silently ignored until now.
    #[serde(flatten)]
    pub unknown: BTreeMap<String, toml::Value>,
}

/// A structured, file/line/field-scoped config problem. Used for both read
/// errors (missing permissions, etc.) and TOML parse errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub field: Option<String>,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path.display())?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        write!(f, ":")?;
        if let Some(field) = &self.field {
            write!(f, " {field}:")?;
        }
        write!(f, " {}", self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Load and parse the config file at `path`.
///
/// - `Ok(None)` — the file does not exist; callers should proceed with
///   built-in defaults.
/// - `Ok(Some(_))` — the file was read and parsed successfully.
/// - `Err(_)` — the file exists but could not be read or parsed; callers
///   should surface the error as a warning and fall back to defaults.
pub fn load(path: &Path) -> Result<Option<FileConfig>, ConfigError> {
    let contents = match crate::config::read::read_config_file(path) {
        Ok(Some(contents)) => contents,
        Ok(None) => return Ok(None),
        Err(message) => {
            return Err(ConfigError {
                path: path.to_path_buf(),
                line: None,
                field: None,
                message,
            });
        }
    };

    toml::from_str::<FileConfig>(&contents)
        .map(Some)
        .map_err(|err| {
            let line = err
                .span()
                .map(|span| contents[..span.start].matches('\n').count() + 1);
            ConfigError {
                path: path.to_path_buf(),
                line,
                field: None,
                message: err.message().to_string(),
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn missing_file_returns_ok_none() {
        let result = load(Path::new("/nonexistent/nuvem-config-does-not-exist.toml"));
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn valid_file_parses_general_section() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(
            b"version = 1\n[general]\npoll_interval_secs = 12\nsidebar_visible = false\n",
        )
        .unwrap();

        let file_config = load(file.path()).unwrap().unwrap();
        assert_eq!(file_config.version, Some(1));
        let general = file_config.general.unwrap();
        assert_eq!(general.poll_interval_secs, Some(12));
        assert_eq!(general.sidebar_visible, Some(false));
    }

    #[test]
    fn malformed_file_returns_config_error_with_line() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"this is : not = toml [[[").unwrap();

        let err = load(file.path()).unwrap_err();
        assert_eq!(err.path, file.path());
        assert!(err.line.is_some());
        assert!(!err.message.is_empty());
    }

    #[test]
    fn config_error_display_includes_path_and_line() {
        let err = ConfigError {
            path: PathBuf::from("/tmp/nuvem.toml"),
            line: Some(3),
            field: Some("general.poll_interval_secs".to_string()),
            message: "invalid type".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "/tmp/nuvem.toml:3: general.poll_interval_secs: invalid type"
        );
    }
}

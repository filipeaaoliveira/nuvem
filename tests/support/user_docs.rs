//! The user documentation, read the way the documentation site reads it.
//!
//! Every page under `docs/` is a user page except the contributor documents
//! named in [`CONTRIBUTOR_DOCS`]. The set is found by walking the directory
//! rather than written down, so a page added to `docs/` is read by every pin
//! and completeness check the moment it exists, and a page that is removed
//! cannot leave a check passing by never running.
//!
//! A user page opens with YAML frontmatter carrying its `title` and
//! `description`, and has no level-1 heading of its own: the site renders the
//! title as the page heading. [`page`] puts that title back as a `# ` line, so
//! a section that ends one page is closed by the next page's title when the
//! corpus is read as one document, and so a check can scope itself to a whole
//! page by its title.

use std::path::Path;

/// Paths under `docs/` that are contributor documentation, not user pages.
pub(crate) const CONTRIBUTOR_DOCS: &[&str] = &["docs/decisions/", "docs/performance-read-paths.md"];

/// The README, then every user page under `docs/`, in path order.
pub(crate) fn paths() -> Vec<String> {
    let mut pages = Vec::new();
    collect(Path::new("docs"), &mut pages);
    pages.sort();
    let mut all = vec!["README.md".to_string()];
    all.extend(pages);
    all
}

fn collect(dir: &Path, out: &mut Vec<String>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|error| panic!("{dir:?}: {error}"));
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        let shown = path.to_string_lossy().replace('\\', "/");
        let shown_dir = format!("{shown}/");
        if CONTRIBUTOR_DOCS
            .iter()
            .any(|excluded| *excluded == shown || *excluded == shown_dir)
        {
            continue;
        }
        if path.is_dir() {
            collect(&path, out);
        } else if shown.ends_with(".md") {
            out.push(shown);
        }
    }
}

/// The frontmatter fields a user page must carry.
pub(crate) struct Frontmatter {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) order: i64,
}

/// Splits a user page into its frontmatter and its body, or panics naming
/// what is missing.
///
/// The frontmatter is parsed as YAML, the way the documentation site parses
/// it. A check by line prefix would accept a `description` holding an
/// unquoted `: `, which YAML rejects and which fails the site build.
pub(crate) fn split(path: &str, text: &str) -> (Frontmatter, String) {
    use yaml_rust2::{Yaml, YamlLoader};

    let rest = text
        .strip_prefix("---\n")
        .unwrap_or_else(|| panic!("{path} opens with YAML frontmatter"));
    let close = rest
        .find("\n---\n")
        .unwrap_or_else(|| panic!("{path} closes its frontmatter"));
    let (yaml, body) = (&rest[..close], &rest[close + "\n---\n".len()..]);
    let documents = YamlLoader::load_from_str(yaml)
        .unwrap_or_else(|error| panic!("{path} has frontmatter that is not valid YAML: {error}"));
    let document = documents
        .first()
        .unwrap_or_else(|| panic!("{path} has an empty frontmatter"));
    let text_field = |name: &str| -> String {
        match &document[name] {
            Yaml::String(value) if !value.trim().is_empty() => value.clone(),
            other => {
                panic!("{path} has a non-empty string `{name}` in its frontmatter, not {other:?}")
            }
        }
    };
    let order = match &document["sidebar"]["order"] {
        Yaml::Integer(order) => *order,
        other => panic!("{path} has an integer `sidebar.order` in its frontmatter, not {other:?}"),
    };
    let frontmatter = Frontmatter {
        title: text_field("title"),
        description: text_field("description"),
        order,
    };
    (frontmatter, body.to_string())
}

/// One page as the corpus reads it: the README as it stands, a `docs/` page
/// with its frontmatter replaced by a `# <title>` line.
pub(crate) fn page(path: &str) -> String {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{path} is readable: {error}"));
    if path == "README.md" {
        return text;
    }
    let (frontmatter, body) = split(path, &text);
    assert!(
        frontmatter.order >= 0 && !frontmatter.description.trim().is_empty(),
        "{path} has a sidebar order and a description"
    );
    let mut fenced = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        } else if !fenced && line.starts_with("# ") {
            panic!(
                "{path} has its own level-1 heading {line:?}; the title belongs in the \
                 frontmatter, and the site would show both"
            );
        }
    }
    format!("# {}\n{body}", frontmatter.title)
}

/// Every user page, concatenated in [`paths`] order.
pub(crate) fn corpus() -> String {
    paths()
        .iter()
        .map(|path| format!("{}\n", page(path)))
        .collect()
}

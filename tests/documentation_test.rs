//! Documentation pins — two kinds of assertion, deliberately mixed.
//!
//! A **pinned** assertion derives its expected value from the code, or extracts
//! the claim out of the document and compares it to the code. It fails three
//! ways: the document is wrong today, the code moves without the document, or
//! the paragraph is deleted. The filter-grammar table, the label-pushdown
//! attributions, the mark keys, the `Marked:` indicator, the batch-action lists
//! and their acknowledgements, the batch-concurrency default, the `L`
//! entry-point enumeration, the default-jumps table, the destination-tab
//! vocabulary, the `{field}` vocabulary and the `[columns]` catalog table are
//! all pinned.
//!
//! A **presence** assertion is a substring check. It proves only that a
//! paragraph has not been silently deleted. It cannot detect the paragraph
//! turning false: the claim can be reversed word for word with the substring
//! intact, and a short substring can be satisfied by unrelated text elsewhere
//! in the same file. Deletion detection is all it buys.
//!
//! The mix is on purpose, not an unfinished migration. A claim with a
//! machine-derivable fact behind it is pinned to that fact. A claim that is
//! design rationale — why `Drop` cannot await, why the port probe is advisory,
//! why `SIGINT` is excluded from the signal backstop — has no such fact, and
//! keeping the paragraph alive is the most that is available.
//!
//! Every presence-only test, and every presence-only block inside an otherwise
//! pinned test, carries a `// PRESENCE-ONLY:` marker line giving the reason the
//! claim is not derivable. No marker means the assertions are pinned. Do not
//! read a marked test and an unmarked one as equal evidence.
//!
//! **Every presence assertion in this file goes through `Doc`, and the section
//! it is scoped to is a required argument.** Scope is worth enforcing because
//! it is the one property here that is decidable: whether a paragraph is *true*
//! is not checkable by any test, but whether an assertion names the section it
//! applies to is. An audit measured what the unscoped form was worth — nine of
//! nine assertions stayed green after the claim they guard was deleted — and
//! fixed those nine by hand. This is the constraint that stops them being
//! rewritten: the two shapes the convention used to permit no longer compile.
//!
//! ```text
//! error[E0599]: no method named `contains` found for struct `Doc`
//! error[E0061]: this method takes 2 arguments but 1 argument was supplied
//!                       argument #1 of type `&str` is missing
//! ```

use nuvem::app::projection::columns::catalog_meta;
use nuvem::app::state::navigation::{DetailTab, JumpTarget};
use nuvem::app::state::resources::{CloudSqlDetailTab, ComputeDetailTab};
use nuvem::config::favorites::{FavoriteKind, FavoriteScopes, KEYS};
use nuvem::config::jumps::{JumpRules, detail_backed_fields, fields_for, location_field};
use nuvem::config::problems::ConfigProblems;
use nuvem_domain::gcp::filter::label_filter_field;
use nuvem_domain::gcp::resource::ResourceServiceKind;
use std::collections::BTreeSet;

/// The last fenced `toml` block before `marker`, so a documented example can
/// be fed to the parser that has to accept it rather than read by eye.
fn toml_block_before(document: &str, marker: &str) -> String {
    let up_to_marker = &document[..document.find(marker).unwrap_or_else(|| {
        panic!("the document says {marker:?}");
    })];
    let open = up_to_marker
        .rfind("```toml\n")
        .expect("the marker is preceded by a toml example");
    let body = &up_to_marker[open + "```toml\n".len()..];
    let close = body.find("```").expect("the toml example is closed");
    body[..close].to_string()
}

// PRESENCE-ONLY: the registry boundary is a rule about where service branching
// is allowed to live. No symbol reports "Home does not branch on service", so
// there is nothing to compare the prose against.
#[test]
fn architecture_documents_service_registry_boundaries() {
    let architecture = Doc::architecture();

    // `service registry` occurs four times document-wide and
    // `Home or switcher branching` twice, so unscoped none of these could fail.
    architecture.presence(
        "### Service registry and navigator flow",
        &[
            "service registry",
            "service-specific resource behavior",
            "Home or switcher branching",
        ],
    );
}

/// Two promises the layout section makes that nothing else can keep.
///
/// The section heading is a constant in `tests/architecture_layout_test.rs`,
/// printed in every failure it reports as the place to go and fix the drift.
/// The checker reads the markers, never the heading, so renaming the heading
/// back to the old "Recommended Repository Layout" would leave the checker
/// green while it directed authors to a section that no longer exists.
///
/// The prose also promises a checker by name. Deleting or renaming that file
/// would silently retire the check while the document went on claiming it, so
/// the named path is asserted to exist rather than merely to be mentioned.
#[test]
fn architecture_layout_section_promises_a_checker_that_exists() {
    let architecture = Doc::architecture();

    // The heading the checker's failure message sends authors to.
    // PRESENCE-ONLY: the heading the checker points at is a string constant in
    // another test crate, not something this crate can import and compare.
    architecture.heading_exists("## Repository Layout");
    architecture.absent(&["## Recommended Repository Layout"]);

    // Described as checked, not recommended: the block is a record of what is
    // on disk, and adding a module means adding a line to it.
    // PRESENCE-ONLY: how the layout block is meant to be read is editorial
    // framing; the checker exposes no fact that says "both directions".
    //
    // This is the section whose fenced block opens `# VERIFIED - checked in
    // both directions against the filesystem by ...`, and that line is a
    // comment inside the block rather than a heading — `section_body` is
    // fence-aware for exactly this, since a scope that ended at it would cut
    // the section off above its own prose.
    //
    // It is also a heading echo in another form: the comment restates the
    // prose claim, so `contains("checked in both directions")` is satisfied by
    // the code block alone and survives the paragraph being deleted. The
    // needle carries the following word, which the comment does not have.
    architecture.presence(
        "## Repository Layout",
        &[
            "checked in both directions by",
            "means adding a line here",
            // And the checker it names is really there.
            "`tests/architecture_layout_test.rs`",
        ],
    );
    assert!(std::path::Path::new("tests/architecture_layout_test.rs").exists());
}

/// A document, and the only way a presence check in this file may read one.
///
/// An audit measured what an unscoped presence assertion is worth: nine of nine
/// stayed green after the claim they guard was deleted, because every needle
/// also occurred somewhere else in the document. Scoping each one to its own
/// section is what made all nine fail. That fix left "scope your presence
/// checks" as a convention an author has to remember and a reviewer has to
/// catch — and that audit is the evidence that both failed nine times, in a file
/// whose own module doc already explains what a presence check is worth.
///
/// So the convention is a constraint here instead. A `Doc` has no `contains`
/// and does not deref to `str`, and every presence entry point takes the
/// section as a **required argument**. `architecture.contains("...")` inside a
/// test that holds a `Doc` is a compile error, not a lapse.
///
/// Scope is enforceable precisely because it is decidable. Whether a paragraph
/// is *true* is not, and no helper here pretends otherwise: a presence check
/// buys deletion detection and nothing more.
// PRESENCE-ONLY: the GKE slice's boundary is a scope decision — read-only, no
// mutation — and no symbol reports "GKE has no mutating action" in a form a
// test can compare prose against. The argv lines beside it are the load-bearing
// half, and they are the ones a drift in the service would falsify.
//
// This pin and the one below it lived in tests/ui_projection_test.rs as bare
// `include_str!("../README.md")` + `contains`, which is the unscoped shape this
// file's header describes and the audit measured the worth of. Moving the prose
// out of the README forced them to move; they go through `Doc` now, so the
// section each one applies to is stated and checked.
#[test]
fn docs_document_the_gke_read_only_workflow() {
    let docs = Doc::user_docs();

    docs.presence(
        "# GKE clusters",
        &[
            "GKE clusters",
            "container clusters list --format=json",
            "container clusters describe",
            "GKE mutation is not included",
        ],
    );
}

// PRESENCE-ONLY: that `:` opens a *global* mode is a claim about every view
// admitting the palette, and no single symbol carries it. The aliases the
// paragraph lists are pinned against the real alias table by
// `docs_list_every_configurable_column_of_every_service`'s neighbours; what is
// left here is the paragraph's existence.
#[test]
fn docs_document_the_global_command_palette() {
    let docs = Doc::user_docs();

    docs.presence(
        "# Command palette",
        &["global command mode", "`gce`", "`gke`"],
    );
}

/// The corpus and the README's own table of contents are the same set.
///
/// The corpus is every page under `docs/` except the contributor documents,
/// found by walking the directory (`support::user_docs::paths`). Holding the
/// README's links to that set means a page added to `docs/` and not linked
/// from the README fails here, and so does a README link to a page that is
/// gone.
#[test]
fn readme_links_every_page_the_pins_are_read_from() {
    let readme = std::fs::read_to_string("README.md").expect("README.md is readable");
    let linked: BTreeSet<String> = readme
        .match_indices("](docs/")
        .map(|(at, _)| {
            let from = at + "](".len();
            let rest = &readme[from..];
            let close = rest.find(')').expect("a markdown link closes");
            rest[..close].to_string()
        })
        .collect();

    let read: BTreeSet<String> = crate::support::user_docs::paths()
        .into_iter()
        .filter(|path| path.starts_with("docs/"))
        .collect();

    assert_eq!(
        linked, read,
        "every docs/ page the README links is read by the pins, and every page \
         the pins read is reachable from the README",
    );
}

struct Doc {
    name: &'static str,
    text: String,
}

impl Doc {
    /// The user-facing documentation, read as one document.
    ///
    /// Every pin below asks "is this fact written down where a reader will
    /// find it", and until the reference moved out of a 2,700-line README
    /// that question had exactly one file to ask of. It now spans README.md
    /// and the pages README links, so the corpus is the set, and the pins
    /// mean today what they meant before the split: a fact cannot be
    /// retired by moving it into whichever file no test happens to name.
    ///
    /// Concatenation is safe for what these assertions do. `section_body`
    /// terminates a span on the next heading at the same or a shallower
    /// level, and every page contributes a level-1 title (a `docs/` page's
    /// comes from its frontmatter), so a section at the end of one page is
    /// closed by the next page's title rather than bleeding into it. The
    /// same title is what lets a check scope itself to one whole page.
    fn user_docs() -> Self {
        let mut text = String::new();
        for path in crate::support::user_docs::paths() {
            let page = crate::support::user_docs::page(&path);
            assert_eq!(
                page.lines().filter(|line| is_fence(line)).count() % 2,
                0,
                "{path} closes every fence it opens; an odd count would leak \
                 the fenced state into the next page and hide its headings",
            );
            text.push_str(&page);
            text.push('\n');
        }
        Self::of("the user documentation", &text)
    }

    fn architecture() -> Self {
        Self::of(
            "ARCHITECTURE.md",
            &std::fs::read_to_string("ARCHITECTURE.md").unwrap(),
        )
    }

    fn of(name: &'static str, text: &str) -> Self {
        Self {
            name,
            text: text.to_string(),
        }
    }

    /// The body of `heading`: everything after its heading line, up to the next
    /// heading at the same or a shallower level.
    ///
    /// The heading line is **excluded**, and that is load-bearing rather than
    /// incidental. Four ARCHITECTURE.md headings restate the exact words their
    /// body states the claim in — `### Why the kill targets the process group`
    /// against `contains("process group")` is the clearest — so a haystack that
    /// kept its heading would satisfy those assertions from the heading alone
    /// and never notice the prose disappearing. That is the defect this file
    /// exists to keep fixed, and the audit's own first commit re-created it in its
    /// own cure. A claim that really is only stated in a heading goes through
    /// `heading_exists`, which says so.
    ///
    /// A `#` at the start of a line inside a fenced block is a comment, not a
    /// heading: ARCHITECTURE.md's repository-layout block opens with
    /// `# VERIFIED - checked in both directions ...`, and a terminator scan
    /// that counted it would cut that section off above its own prose and
    /// silently shrink every check scoped to it.
    fn section_body(&self, heading: &str) -> String {
        let level = heading
            .chars()
            .take_while(|character| *character == '#')
            .count();
        assert!(level > 0, "{heading:?} is not a markdown heading");

        let mut fenced = false;
        let mut lines = self.text.lines();
        // The whole heading line has to match, so a heading that is a prefix of
        // a longer one (`## Beta` against `## Betamax`) cannot select the
        // longer one's span.
        let found = lines.by_ref().any(|line| {
            if is_fence(line) {
                fenced = !fenced;
                return false;
            }
            !fenced && line == heading
        });
        assert!(found, "{} has a {heading:?} section", self.name);

        let mut body: Vec<&str> = Vec::new();
        for line in lines {
            if is_fence(line) {
                fenced = !fenced;
            } else if !fenced {
                let depth = line
                    .chars()
                    .take_while(|character| *character == '#')
                    .count();
                if depth > 0 && depth <= level && line[depth..].starts_with(' ') {
                    break;
                }
            }
            body.push(line);
        }
        body.join("\n")
    }

    /// Every needle must appear in the body of `heading`.
    ///
    /// The section is a **required argument**: that is the whole point of this
    /// type. What this buys is deletion detection inside one section and
    /// nothing else — a claim can be reversed word for word with the needle
    /// intact, and no argument to this function can change that.
    ///
    /// The body is read reflowed to one line, so a claim that wraps across a
    /// line break is still one string to match and a reflow is not reported as
    /// a deletion.
    ///
    /// What the section argument buys is granularity, and it stops at the
    /// section: a needle written down more than once inside its own section is
    /// still satisfied after the paragraph it guards is deleted. Several
    /// needles below are like that on purpose — a key name documented in seven
    /// places is a section documenting it thoroughly — but where a needle is
    /// meant to guard one paragraph, it has to be narrow enough to name it.
    fn presence<S: AsRef<str>>(&self, heading: &str, needles: &[S]) {
        let body = unwrapped(&self.section_body(heading));
        for needle in needles {
            let needle = unwrapped(needle.as_ref());
            assert!(
                body.contains(&needle),
                "{}: the {heading:?} section must say {needle:?}",
                self.name
            );
        }
    }

    /// A claim that must **not** be anywhere in the document.
    ///
    /// Deliberately unscoped, and the one entry point here that takes no
    /// section. Narrowing an absence check weakens it: a superseded claim
    /// re-added one section over would pass a scoped `absent` while being
    /// exactly the regression the assertion exists to catch. Scope makes a
    /// presence check stronger and an absence check weaker, so the two take
    /// opposite defaults.
    fn absent<S: AsRef<str>>(&self, needles: &[S]) {
        let text = unwrapped(&self.text);
        for needle in needles {
            let needle = unwrapped(needle.as_ref());
            assert!(
                !text.contains(&needle),
                "{}: {needle:?} is a superseded claim and must stay out of the document",
                self.name
            );
        }
    }

    /// A claim stated only in a heading, asserted as the heading-existence
    /// check it is.
    ///
    /// Two claims in this file really are written down nowhere but a heading —
    /// the `SIGTERM`/`SIGHUP` pair and the `## Tunnels` section itself — and
    /// neither has prose for `presence` to guard. Going through a separate,
    /// named entry point is what stops the next reader mistaking one for a weak
    /// content check.
    ///
    /// Fence-aware for the same reason `section_body` is: both documents show
    /// heading-shaped lines inside code blocks, and a fenced example of a
    /// heading must not stand in for the heading itself.
    fn heading_exists(&self, heading: &str) {
        let mut fenced = false;
        let found = self.text.lines().any(|line| {
            if is_fence(line) {
                fenced = !fenced;
                return false;
            }
            !fenced && line == heading
        });
        assert!(found, "{} must have a {heading:?} heading", self.name);
    }

    /// PINNED PARSING ONLY — never for a presence check.
    ///
    /// A pinned assertion walks a table or extracts a claim and compares it
    /// with the code, and that needs the raw text. A presence check that
    /// reached for this would be back to `document.contains(...)` across a
    /// whole document, which is the exact defect this type exists to make
    /// unwriteable. Use `presence`, `absent` or `heading_exists`.
    fn text_for_pinned_parsing(&self) -> &str {
        &self.text
    }

    /// PINNED PARSING ONLY, scoped — never for a presence check. Same rule as
    /// `text_for_pinned_parsing`: this hands out a `String` a `contains` call
    /// would compile against.
    fn section_for_pinned_parsing(&self, heading: &str) -> String {
        self.section_body(heading)
    }
}

/// A fenced-block delimiter, in either of the two spellings CommonMark allows.
fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// The four properties `Doc::section_body` has to have for a presence check
/// scoped through it to be worth anything. Each is a way the scoping has
/// already been got wrong once, here or in that audit, so each is checked against
/// a fixture rather than left to the reading of the implementation.
///
/// These are assertions about the helper, not about the documents. They run
/// against a literal fixture so that editing README.md or ARCHITECTURE.md can
/// never make them pass or fail.
#[test]
fn section_body_is_scoped_body_only_and_fence_aware() {
    const FIXTURE: &str = "\
# Title

## Alpha

Alpha says process group.

## Beta

### Why the kill targets the process group

The claim is stated
across two lines here.

### Sibling

Sibling says nothing.

## Layout

Prose before the block.

```text
# VERIFIED - a comment inside a fence, not a heading
```

Prose after the block.

## Betamax

Betamax is a longer heading that starts with Beta.
";

    let doc = Doc::of("FIXTURE.md", FIXTURE);

    // 1. Scoped: a claim stated in another section is not in this one.
    assert!(
        !doc.section_body("### Sibling").contains("process group"),
        "a section body must not carry a neighbouring section's prose"
    );

    // 2. Body-only: three headings in ARCHITECTURE.md restate the exact words
    //    their body states the claim in, so a haystack that kept its heading
    //    line would satisfy those assertions from the heading alone — the
    //    defect the audit's first commit re-created in its own cure.
    let kill = doc.section_body("### Why the kill targets the process group");
    assert!(
        !kill.contains("targets the process group"),
        "the heading line must not be part of the body it heads"
    );
    assert!(
        kill.contains("The claim is stated"),
        "the body itself must be in the body"
    );

    // 3. Fence-aware: a `#` comment inside a fenced block is not a heading, and
    //    ARCHITECTURE.md's layout block opens with one. A terminator scan that
    //    counted it would cut the section off above its own prose.
    let layout = doc.section_body("## Layout");
    assert!(
        layout.contains("Prose after the block"),
        "a `#` inside a fenced block must not terminate the section"
    );

    // 4. Anchored on the whole heading line: a heading that is a prefix of a
    //    longer one must not select the longer one's span, or every check
    //    scoped to it silently moves.
    let beta = doc.section_body("## Beta");
    assert!(
        !beta.contains("Betamax is a longer heading"),
        "a prefix heading must not select the section it is a prefix of"
    );
    assert!(
        beta.contains("Sibling says nothing"),
        "a section body includes its own subsections"
    );
}

/// `heading_exists` has to be fence-aware for the same reason `section_body`
/// is. A heading shown as sample markdown inside a code block must not stand in
/// for the heading it illustrates, or a section can be deleted outright while
/// its only guard stays green.
///
/// Assertions about the helper, against a literal fixture, so editing
/// README.md or ARCHITECTURE.md can never make them pass or fail.
#[test]
fn heading_exists_is_fence_aware() {
    const FENCED_ONLY: &str = "\
# Title

```text
## Managed Tunnels
```
";
    const REAL: &str = "\
# Title

## Managed Tunnels

Prose.
";

    Doc::of("FIXTURE.md", REAL).heading_exists("## Managed Tunnels");

    let fenced = std::panic::catch_unwind(|| {
        Doc::of("FIXTURE.md", FENCED_ONLY).heading_exists("## Managed Tunnels")
    });
    assert!(
        fenced.is_err(),
        "a heading inside a fenced block must not satisfy `heading_exists`"
    );
}

/// `presence` reads the section reflowed to one line, so a claim that wraps
/// across a line break is still one string to match — and a reflow of the
/// document is not reported as the claim having been deleted.
#[test]
fn presence_matches_a_claim_that_wraps_across_a_line_break() {
    let doc = Doc::of(
        "FIXTURE.md",
        "## Alpha\n\nThe claim is stated\nacross two lines here.\n",
    );

    doc.presence("## Alpha", &["The claim is stated across two lines here."]);
}

/// Reflowed to one line, so a claim that wraps across a line break is still one
/// string to match. Every prose pin below works on this form.
fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

const BATCH_SECTION: &str = "# Batch actions on marked rows";
/// The key reference, which is a page rather than a section: its level-1
/// title, taken from the page's frontmatter, is what scopes to it.
const KEYS_PAGE: &str = "# Keys";
/// Each service has a page of its own under `docs/reference/services/`, titled
/// with the service's registry label, so a pin about one service is scoped to
/// that service's page and nothing else.
const IAM_SECTION: &str = "# IAM policy";
const SECRETS_PAGE: &str = "# Secret Manager";

/// The BigQuery page.
const BIGQUERY_SECTION: &str = "# BigQuery datasets";

/// ARCHITECTURE.md's BigQuery section, which restates two of the numbers the
/// README states — the enrichment width and the listing cap — in the same
/// idioms. A pin scoped to the README alone leaves this document silently
/// false the day either number is retuned, which is the failure the BigQuery
/// pins exist to prevent; so both documents go through the same parse.
const BIGQUERY_SECTION_ARCHITECTURE: &str = "### BigQuery: two drill-down levels, no detail pane";

/// The VPC networks page.
const VPC_SECTION: &str = "# VPC networks";

/// The firewall rules page. Scoped separately from the networks page because
/// the two make different claims about the same fixture family, and a needle
/// that matched either would let one of them be deleted.
const FIREWALL_SECTION: &str = "# Firewall rules";

/// The Cloud Logging page, where the logging entry points are documented.
const CLOUD_LOGGING: &str = "# Cloud Logging";

/// The filter guide, where the resource-list filter grammar is documented.
const FILTER_PAGE: &str = "# Filter a list";

/// The config file page, which carries the full example and the key table.
const CONFIG_FILE_PAGE: &str = "# Config file";

/// The two sections that both talk about jumps: the rule syntax under
/// Configuration, and the picker the rules feed.
const JUMP_RULES: &str = "# Jump rules";
const JUMPS: &str = "# Related-resource jumps";

/// The two sections describing the Home inventory, one per document. Both
/// enumerate the registry's services, and both are pinned to it together.
const HOME_README: &str = "# Home overview";
const HOME_ARCHITECTURE: &str = "## Home Overview Dashboard";

/// The four mark keys, each pinned to the bullet that describes *its* action.
///
/// The old assertions were `contains("`Space`")` and friends: four keys and
/// four bullets, with nothing tying a key to the bullet it belongs to. The
/// README could have offered `\` for range-marking and every substring would
/// still have been present. Here the bullet is found by its own wording and
/// the key it carries has to be the one the binding catalog binds.
///
/// The catalog stores the bare uppercase letter (`V`, `U`) because that is the
/// `KeyTrigger::Char`; the README spells the same key `Shift-V`, which is how
/// it reads on a keyboard. That rendering is the one place the two spellings
/// are reconciled.
#[test]
fn docs_document_the_mark_keys_the_binding_catalog_binds() {
    use nuvem::app::projection::binding::{
        Binding, MARK_CLEAR, MARK_RANGE, MARK_TOGGLE, MARK_UNMARK_VISIBLE,
    };

    let readme = Doc::user_docs();
    // Pinned parsing: the bullets are walked and each key compared with the
    // binding catalog, so this needs the section's raw text.
    let section = readme.section_for_pinned_parsing(BATCH_SECTION);

    // The marking bullets: `- Press ...` lines plus their continuations, so a
    // bullet that wraps is matched as the one sentence it is.
    let mut bullets: Vec<String> = Vec::new();
    for line in section.lines() {
        if let Some(rest) = line.strip_prefix("- Press ") {
            bullets.push(rest.to_string());
        } else if !line.trim().is_empty() && line.starts_with("  ") {
            if let Some(last) = bullets.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            }
        } else if line.trim().is_empty() {
            // A blank line ends the list; later paragraphs are not bullets.
            if !bullets.is_empty() {
                break;
            }
        }
    }

    let documented_key = |bullet: &str| -> String {
        bullet
            .split('`')
            .nth(1)
            .unwrap_or_else(|| panic!("marking bullet {bullet:?} backticks its key"))
            .to_string()
    };
    // How the README spells what the catalog stores as a `KeyTrigger::Char`.
    let readme_spelling = |binding: &Binding| -> String {
        let key = binding.key;
        let mut chars = key.chars();
        match (chars.next(), chars.next()) {
            (Some(letter), None) if letter.is_ascii_uppercase() => format!("Shift-{letter}"),
            _ => key.to_string(),
        }
    };

    for (binding, subject) in [
        (MARK_TOGGLE, "mark or unmark the selected instance"),
        (MARK_RANGE, "range-mark every instance"),
        (MARK_CLEAR, "clear every mark"),
        (MARK_UNMARK_VISIBLE, "unmark only the currently visible"),
    ] {
        let bullet = bullets
            .iter()
            .find(|bullet| bullet.contains(subject))
            .unwrap_or_else(|| panic!("README documents {subject:?} as a marking bullet"));
        assert_eq!(
            documented_key(bullet),
            readme_spelling(&binding),
            "README binds {subject:?} to a key the catalog does not"
        );
    }

    // Both directions: the list documents exactly the four mark bindings, so a
    // fifth bullet inventing a key, or a retired binding left documented, fails.
    let mut documented: Vec<String> = bullets.iter().map(|b| documented_key(b)).collect();
    documented.sort();
    let mut real: Vec<String> = [MARK_TOGGLE, MARK_RANGE, MARK_CLEAR, MARK_UNMARK_VISIBLE]
        .iter()
        .map(readme_spelling)
        .collect();
    real.sort();
    assert_eq!(
        documented, real,
        "the marking list must document exactly the catalog's mark bindings"
    );
}

/// The two `Marked:` forms, taken from titles the projection really builds.
///
/// `readme.contains("Marked:")` was true of both forms and of neither: it
/// survived any change to what follows the colon. Here the app is driven to
/// produce a title with and without hidden marks, the counts are replaced by
/// the README's own placeholders, and the result has to be documented verbatim.
/// Renaming the segment, dropping the parenthesis, or reordering the counts
/// all fail.
#[test]
fn docs_document_the_marked_indicator_the_frame_builds() {
    use nuvem::app::projection::project_app_frame;
    use nuvem::app::reducer::root::reduce;
    use nuvem::app::{Action, AppState, EffectResult, ResourcesAction, Workspace};
    use nuvem_domain::gcp::configuration::{GcloudConfiguration, GcloudConfigurationName};
    use nuvem_domain::gcp::resource::{ComputeInstance, ResourceStatus};

    // Distinct counts (7 marked, 3 of them hidden) so replacing each number
    // with its placeholder is unambiguous.
    let instances: Vec<ComputeInstance> = (0..7)
        .map(|index| ComputeInstance {
            id: format!("i-{index}-id"),
            // Four `api-*` rows survive the `api` filter; three `web-*` do not.
            name: if index < 4 {
                format!("api-{index}")
            } else {
                format!("web-{index}")
            },
            zone: "europe-west1-b".to_string(),
            status: ResourceStatus::Running,
            machine_type: "e2-medium".to_string(),
            internal_ip: None,
            external_ip: None,
        })
        .collect();

    let mut app = AppState::new();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ConfigurationsLoaded {
            configurations: vec![GcloudConfiguration {
                name: GcloudConfigurationName::new("dev"),
                is_active: true,
                account: Some("dev@example.com".to_string()),
                project: Some("dev-project".to_string()),
                region: Some("europe-west1".to_string()),
                zone: Some("europe-west1-b".to_string()),
            }],
        }),
    );
    let context = app
        .root
        .current_context()
        .expect("the loaded configuration is active")
        .clone();
    let count = instances.len();
    reduce(
        &mut app,
        &Action::EffectResult(EffectResult::ComputeInstancesLoaded { context, instances }),
    );
    app.root.workspace = Workspace::Resources;
    reduce(&mut app, &Action::Resources(ResourcesAction::ToggleMark));
    for _ in 1..count {
        reduce(&mut app, &Action::Resources(ResourcesAction::SelectNext));
        reduce(&mut app, &Action::Resources(ResourcesAction::ToggleMark));
    }

    let marked_segment = |app: &AppState| -> String {
        let title = project_app_frame(&app.root).view_title;
        let at = title
            .find("Marked")
            .unwrap_or_else(|| panic!("marked rows put a marked segment in {title:?}"));
        title[at..].to_string()
    };

    let plain = marked_segment(&app);

    reduce(&mut app, &Action::Resources(ResourcesAction::StartFilter));
    for value in "api".chars() {
        reduce(
            &mut app,
            &Action::Resources(ResourcesAction::AppendFilterChar(value)),
        );
    }
    reduce(&mut app, &Action::Resources(ResourcesAction::ApplyFilter));
    let with_hidden = marked_segment(&app);

    // The real segments, with the counts swapped for the placeholders the
    // README documents them under.
    let documented_plain = plain.replace('7', "<n>");
    let documented_hidden = with_hidden.replace('7', "<n>").replace('3', "<hidden>");

    let readme = Doc::user_docs();
    // Pinned parsing: the documented forms are compared with the ones the
    // frame really builds.
    let section = unwrapped(&readme.section_for_pinned_parsing(BATCH_SECTION));
    for form in [&documented_plain, &documented_hidden] {
        assert!(
            section.contains(&format!("`{form}`")),
            "README must document the `{form}` indicator the frame builds"
        );
    }

    // Both directions: no other `Marked: ...` form is documented, so a stale
    // shape left behind after the builder changed fails too.
    let mut documented: Vec<String> = section
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|token| token.starts_with("Marked"))
        .map(str::to_string)
        .collect();
    documented.sort();
    documented.dedup();
    let mut real = vec![documented_plain, documented_hidden];
    real.sort();
    assert_eq!(
        documented, real,
        "the batch section must document exactly the marked-indicator forms"
    );
}

/// Which batch actions demand a typed count, and which acknowledgement each
/// half of the partition is claimed to use.
///
/// `contains("Type") && contains("to confirm")` was satisfied by the palette's
/// "Type to filter" and by the single-resource "Press `Enter` to confirm" —
/// two unrelated sentences, neither about batches.
///
/// Two claims live in these two sentences and both are pinned, because pinning
/// only one leaves the other free: *which* actions are on each list, and *what
/// each list says happens*. Swapping the two trailing clauses while leaving both
/// action lists alone would otherwise state the exact inverse of the code and
/// pass. So each list is compared as a set against
/// `BatchAction::is_destructive`, and the clause that follows it has to carry
/// the acknowledgement the app really renders for that half — taken from
/// `resource_batch_confirmation_lines`, not retyped here.
#[test]
fn docs_document_the_batch_actions_that_require_a_typed_count() {
    use nuvem::app::state::resources::{
        BatchAck, BatchAction, BatchActionConfirmation, CloudSql, CloudSqlInstanceAction, Compute,
        ComputeInstanceAction, ResourcesState,
    };
    use nuvem::ui::resources::resource_batch_confirmation_lines;
    use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
    use nuvem_domain::gcp::resource::{
        CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance,
        EndpointVisibility, ResourceServiceKind, ResourceStatus,
    };

    let context = GcloudContext {
        configuration: GcloudConfigurationName::new("dev"),
        account: None,
        project: Some("dev-project".to_string()),
        region: None,
        zone: None,
    };

    // One instance per status, all marked, so every action has an eligible
    // target and every action really opens a confirmation.
    let compute_state = || {
        let mut state = ResourcesState::default();
        state.set_instances(
            context.clone(),
            [
                ResourceStatus::Running,
                ResourceStatus::Stopped,
                ResourceStatus::Suspended,
            ]
            .into_iter()
            .enumerate()
            .map(|(index, status)| ComputeInstance {
                id: format!("i-{index}-id"),
                name: format!("i-{index}"),
                zone: "europe-west1-b".to_string(),
                status,
                machine_type: "e2-medium".to_string(),
                internal_ip: None,
                external_ip: None,
            })
            .collect(),
        );
        state.toggle_mark();
        state.select_next();
        state.toggle_mark();
        state.select_next();
        state.toggle_mark();
        state
    };
    let cloud_sql_state = || {
        let mut state = ResourcesState::default();
        state.select_service(ResourceServiceKind::CloudSql);
        state.set_cloud_sql_instances(
            context.clone(),
            [
                (CloudSqlStatus::Runnable, CloudSqlAvailability::Regional),
                (CloudSqlStatus::Stopped, CloudSqlAvailability::Zonal),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (status, availability))| CloudSqlInstance {
                name: format!("sql-{index}"),
                database_version: "POSTGRES_15".to_string(),
                engine: "PostgreSQL".to_string(),
                region: "europe-west1".to_string(),
                tier: "db-custom-2-7680".to_string(),
                availability,
                status,
                connectivity: EndpointVisibility::Private,
            })
            .collect(),
        );
        state.toggle_mark();
        state.select_next();
        state.toggle_mark();
        state
    };

    let mut real_count_required: Vec<String> = Vec::new();
    let mut real_simple: Vec<String> = Vec::new();
    // The acknowledgement prompts the two halves really render, collected as
    // sets so a half that disagrees with itself is caught before either is
    // compared with the prose.
    let mut count_required_prompts: BTreeSet<String> = BTreeSet::new();
    let mut simple_prompts: BTreeSet<String> = BTreeSet::new();

    // How the README names an action: the service it targets plus the action's
    // own label without the resource noun ("Stop instance" -> "Stop").
    let documented_name = |action: BatchAction| {
        format!(
            "{} {}",
            action.service_label(),
            action.label().trim_end_matches(" instance")
        )
    };
    // The acknowledgement line the confirmation modal really renders, with the
    // eligible count swapped for the placeholder the README documents it under.
    let documented_prompt = |confirmation: &BatchActionConfirmation| -> String {
        let count = confirmation.eligible().len().to_string();
        resource_batch_confirmation_lines(confirmation)
            .into_iter()
            .find(|line| line.ends_with("to confirm"))
            .expect("the confirmation modal ends with an acknowledgement line")
            .replace(&count, "<n>")
    };

    {
        let mut record = |wrapped: BatchAction, confirmation: &BatchActionConfirmation| {
            // The wire from destructiveness to acknowledgement. `is_destructive`
            // is what `start_batch_confirmation` switches on, so this restates
            // two adjacent lines rather than deriving the link independently —
            // it is here to fail loudly if that switch is ever rewired, not as
            // proof the partition is meaningful. The prose pins below are what
            // make it so.
            assert_eq!(
                matches!(confirmation.ack(), BatchAck::CountRequired { .. }),
                wrapped.is_destructive(),
                "{wrapped:?} opens an acknowledgement its destructiveness does not explain"
            );
            if wrapped.is_destructive() {
                real_count_required.push(documented_name(wrapped));
                count_required_prompts.insert(documented_prompt(confirmation));
            } else {
                real_simple.push(documented_name(wrapped));
                simple_prompts.insert(documented_prompt(confirmation));
            }
        };

        for action in ComputeInstanceAction::ALL {
            let mut state = compute_state();
            state.start_batch_confirmation::<Compute>(context.clone(), action);
            let confirmation = state
                .batch_confirmation()
                .unwrap_or_else(|| panic!("{action:?} has an eligible marked target"));
            record(BatchAction::Compute(action), confirmation);
        }
        for action in CloudSqlInstanceAction::ALL {
            let mut state = cloud_sql_state();
            state.start_batch_confirmation::<CloudSql>(context.clone(), action);
            let confirmation = state
                .batch_confirmation()
                .unwrap_or_else(|| panic!("{action:?} has an eligible marked target"));
            record(BatchAction::CloudSql(action), confirmation);
        }
    }

    let one = |prompts: BTreeSet<String>, half: &str| -> String {
        assert_eq!(
            prompts.len(),
            1,
            "the {half} actions must all render one acknowledgement, not {prompts:?}"
        );
        prompts.into_iter().next().expect("checked non-empty")
    };
    let count_required_prompt = one(count_required_prompts, "destructive");
    let simple_prompt = one(simple_prompts, "non-destructive");

    // The README's own lists: the parenthesis after each sentence's opening,
    // `;`-separated per service, `,`/`and`-separated within one.
    let readme = Doc::user_docs();
    // Pinned parsing: both action lists and both acknowledgement clauses are
    // read out of the section and compared with the code.
    let section = unwrapped(&readme.section_for_pinned_parsing(BATCH_SECTION));
    // The service names the README may attribute an action to, from the code
    // that renders them, so a renamed service is not silently accepted.
    let services = [
        BatchAction::Compute(ComputeInstanceAction::Start).service_label(),
        BatchAction::CloudSql(CloudSqlInstanceAction::Start).service_label(),
    ];
    // The list, and the clause that follows it to the end of its sentence —
    // what this half of the partition is claimed to *do*, which is a second
    // claim the list alone does not pin.
    let documented_claim = |opening: &str| -> (Vec<String>, String) {
        let at = section
            .find(opening)
            .unwrap_or_else(|| panic!("README says {opening:?}"));
        let body = &section[at + opening.len()..];
        let close = body
            .find(')')
            .unwrap_or_else(|| panic!("README closes the list after {opening:?}"));
        let mut names = Vec::new();
        for segment in body[..close].split(';') {
            let segment = segment.trim();
            let service = services
                .iter()
                .find(|service| segment.starts_with(*service))
                .unwrap_or_else(|| panic!("README attributes {segment:?} to a real service"));
            for name in segment[service.len()..]
                .split(',')
                .flat_map(|part| part.split(" and "))
            {
                let name = name.trim();
                if !name.is_empty() {
                    names.push(format!("{service} {name}"));
                }
            }
        }
        let rest = &body[close + 1..];
        let end = rest.find(". ").map_or(rest.len(), |at| at + 1);
        (names, rest[..end].to_string())
    };

    let (mut documented_count_required, count_required_clause) =
        documented_claim("Destructive batch actions (");
    let (mut documented_simple, simple_clause) =
        documented_claim("Non-destructive batch actions (");

    // Both directions, twice: every action the code treats as destructive is
    // named in the typed-count sentence and nowhere else, and vice versa. A
    // README that moved Reset to the `Enter` list fails on both sets. Sorted
    // `Vec`s rather than sets, so a name listed twice is a mismatch too.
    documented_count_required.sort();
    documented_simple.sort();
    real_count_required.sort();
    real_simple.sort();
    assert_eq!(
        documented_count_required, real_count_required,
        "the typed-count sentence must name exactly the destructive batch actions"
    );
    assert_eq!(
        documented_simple, real_simple,
        "the `Enter` sentence must name exactly the non-destructive batch actions"
    );

    // And each list is pinned to *its own* acknowledgement, so swapping the two
    // trailing clauses while leaving the lists alone — a README stating the
    // exact inverse of the code — fails. Each clause must quote the prompt its
    // half renders and must not quote the other half's.
    for (half, clause, own, other) in [
        (
            "destructive",
            &count_required_clause,
            &count_required_prompt,
            &simple_prompt,
        ),
        (
            "non-destructive",
            &simple_clause,
            &simple_prompt,
            &count_required_prompt,
        ),
    ] {
        assert!(
            clause.contains(&format!("`{own}`")),
            "the {half} sentence must document the `{own}` prompt it renders, not {clause:?}"
        );
        assert!(
            !clause.contains(&format!("`{other}`")),
            "the {half} sentence claims the `{other}` prompt, which is the other half's"
        );
    }
}

/// The documented batch-concurrency default, compared against the real one.
///
/// `readme.contains('4')` was satisfied by any `4` in the document — a port
/// number, a line width, a year. The number is now taken out of the README's
/// own `(default `N`)` and compared with `ResolvedConfig`'s, so a README that
/// says 8 fails, and a default that moves to 8 fails until the README moves
/// with it.
#[test]
fn docs_document_the_real_batch_concurrency_default() {
    use nuvem::config::ResolvedConfig;

    let readme = Doc::user_docs();
    // Pinned parsing: the documented default is extracted and compared with
    // `ResolvedConfig`'s.
    let section = unwrapped(&readme.section_for_pinned_parsing(BATCH_SECTION));

    let flag = "`--batch-concurrency`";
    let at = section
        .find(flag)
        .unwrap_or_else(|| panic!("README documents {flag}"));
    let opening = "(default `";
    let body = &section[at + flag.len()..];
    let start = body
        .find(opening)
        .expect("the flag is followed by its default");
    let value = &body[start + opening.len()..];
    let end = value.find('`').expect("the default is backticked");
    let documented: usize = value[..end]
        .parse()
        .unwrap_or_else(|_| panic!("the documented default {:?} is a number", &value[..end]));

    assert_eq!(
        documented,
        ResolvedConfig::default().batch_concurrency,
        "README documents a --batch-concurrency default the config does not have"
    );
}

/// The grammar table is the published copy of `filter_mode`, and a pair of
/// `contains` on the two cells of a row proves nothing about the row: the
/// README could pair `-f` with `label` and both substrings would still be
/// present somewhere in a 1258-line document. So each row is fed through the
/// real grammar instead — the Input cell goes into `filter_mode` and the Mode
/// cell has to be exactly what comes back.
#[test]
fn docs_document_the_four_filter_modes() {
    use nuvem_domain::gcp::filter::{FilterMode, filter_mode};

    let readme = Doc::user_docs();

    // The table's own rows: the `|` lines following its header, minus the
    // `| --- |` separator. The header is matched by its columns rather than by
    // a padded literal, so reflowing the table's whitespace does not report
    // itself as the table having been deleted.
    let mut lines = readme
        .text_for_pinned_parsing()
        .lines()
        .skip_while(|line| !(line.starts_with("| Input") && line.contains("| Mode")));
    lines
        .next()
        .expect("README documents the filter grammar as a table");
    let rows: Vec<&str> = lines
        .skip(1)
        .take_while(|line| line.starts_with('|'))
        .collect();

    // The first backticked token of a cell. The Meaning column is backticked
    // too, so cells are taken by position rather than by scanning the line.
    let backticked = |row: &str, column: usize| -> String {
        row.split('|')
            .nth(column)
            .unwrap_or_else(|| panic!("grammar row {row:?} has a column {column}"))
            .split('`')
            .nth(1)
            .unwrap_or_else(|| panic!("grammar row {row:?} backticks its column {column}"))
            .to_string()
    };

    let mut documented = Vec::new();
    for row in &rows {
        let input = backticked(row, 1);
        let mode = backticked(row, 2);
        assert_eq!(
            filter_mode(&input).label(),
            mode,
            "the grammar table claims {input:?} selects {mode:?}"
        );
        documented.push(mode);
    }

    // Both directions: every mode that exists is documented, and no row names
    // a mode that does not exist.
    let real: Vec<&str> = FilterMode::ALL.iter().map(|mode| mode.label()).collect();
    let mut sorted = documented.clone();
    sorted.sort();
    let mut real_sorted = real.clone();
    real_sorted.sort_unstable();
    assert_eq!(
        sorted, real_sorted,
        "the grammar table must document exactly the FilterMode variants"
    );

    // `regex` is the default: the mode of an input with no prefix at all is the
    // mode the table's first row names, so the sentence cannot outlive a change
    // to `split_prefix`'s fallback arm.
    // PRESENCE-ONLY (this line): the sentence's wording is prose. The two
    // `assert_eq` below are the pin, and they are what actually holds the
    // fallback arm; this line only keeps the sentence from disappearing.
    readme.presence(FILTER_PAGE, &["`regex` is the default"]);
    let default = documented
        .first()
        .expect("the grammar table has rows")
        .as_str();
    assert_eq!(filter_mode("").label(), default);
    assert_eq!(filter_mode("plain word").label(), default);

    // Label mode is server-side only, and reports "not supported" on exactly
    // one service: Cloud Run. Cloud Storage is out of the grammar entirely, so
    // it reports nothing at all — pairing the two services in that sentence is
    // the claim this assertion exists to keep out.
    // PRESENCE-ONLY from here to the end of this test: server-side-only-ness,
    // the malformed-filter behaviour and Cloud Storage's substring fallback are
    // statements about runtime behaviour behind private seams — nothing reports
    // "label filtering is unsupported for Cloud Run" or "the table is never
    // emptied" as a value. The negative assertions are the load-bearing ones:
    // holding a previously false pairing out is work a substring really can do.
    readme.presence(
        FILTER_PAGE,
        &[
            "Label mode is server-side only",
            "label filtering is not supported for Cloud Run",
            // A malformed filter shows every row plus an inline error.
            "never empties the table",
            "invalid regex: unclosed group",
            // Cloud Storage keeps the plain substring filter and shows no chip.
            // Its real behaviour under a grammar prefix — literal substring
            // match, so normally an empty table with no chip and no error —
            // must be stated, not left to be inferred from a sentence about
            // Cloud Run.
            "Cloud Storage keeps the plain",
            "no mode chip",
            "matched as literal substrings",
            "Cloud Storage reports no filter error",
        ],
    );
    // Document-wide, as every absence check here is: the pairing this holds out
    // would be just as false one section over.
    readme.absent(&["Cloud Run and Cloud Storage"]);
}

/// The pushed label paths, each read together with the service it is
/// attributed to.
///
/// Asserting the paths alone is not enough, and this test carried that defect:
/// the three paths were checked as bare substrings, so swapping the
/// attributions — `labels` given to GKE and `resourceLabels` to Compute — left
/// the suite green while the document told a reader to expect a field gcloud
/// would reject. That is the same unpaired-substring shape the grammar table
/// above was converted away from, so the pairing is pinned in both directions
/// here too.
#[test]
fn architecture_documents_the_label_filter_pushdown() {
    let architecture = Doc::architecture();
    const PUSHDOWN: &str = "## Effects And Services";

    // Where the grammar lives.
    // PRESENCE-ONLY: module and function names as prose. The pairing walk below
    // is the pin; these two only keep the section pointing somewhere.
    architecture.presence(
        PUSHDOWN,
        &["nuvem_domain::gcp::filter", "label_filter_field"],
    );

    // Pinned parsing: the pairing walk below reads every `X.k = "v"` path in
    // the document together with the service beside it, and the *reverse*
    // direction has to see the whole document — a path attributed to the wrong
    // service in some other chapter is exactly what it exists to catch.
    // The attribution wraps across a line break, and a reflow is not a change
    // of meaning.
    let prose = unwrapped(architecture.text_for_pinned_parsing());

    // The path token, closing backtick included, so the field name can be read
    // back to its opening backtick.
    const TOKEN: &str = r#".k = "v"`"#;
    let claim =
        |field: &str, kind: ResourceServiceKind| format!("`{field}{TOKEN} for {}", kind.label());

    // Forward: every service that has a pushdown, with its path attributed to
    // that service by name. The name is `ResourceServiceKind::label()`, so a
    // renamed service has to be renamed here too.
    let mut attributed: BTreeSet<String> = BTreeSet::new();
    for kind in ResourceServiceKind::ALL {
        if let Some(field) = label_filter_field(kind) {
            let claim = claim(field, kind);
            assert!(
                prose.contains(&claim),
                "{kind:?} pushes down {field}, so the section must say {claim:?}"
            );
            attributed.insert(claim);
        }
    }

    // Reverse: every `X.k = "v"` path anywhere in the document, read with the
    // service beside it. Each token is located by a bounded search rather than
    // by splitting the whole file on backticks, so one unpaired backtick 700
    // lines away cannot quietly empty this scan and blame the label paths.
    let mut scanned: BTreeSet<String> = BTreeSet::new();
    let mut cursor = 0;
    while let Some(offset) = prose[cursor..].find(TOKEN) {
        let path_end = cursor + offset;
        let open = prose[..path_end]
            .rfind('`')
            .expect("a documented label path opens with a backtick");
        let field = &prose[open + 1..path_end];
        let tail = &prose[path_end + TOKEN.len()..];

        // No service label is a prefix of another, so this match is unambiguous.
        let kind = ResourceServiceKind::ALL
            .into_iter()
            .find(|kind| tail.starts_with(&format!(" for {}", kind.label())))
            .unwrap_or_else(|| panic!("the label path `{field}{TOKEN} names no service after it"));

        // The pairing itself: a path is only correct beside the service that
        // really pushes it, and a service with no pushdown may carry none.
        assert_eq!(
            label_filter_field(kind),
            Some(field),
            "the document gives `{field}{TOKEN} to {kind:?}"
        );
        scanned.insert(claim(field, kind));
        cursor = path_end + TOKEN.len();
    }

    // Sets, not multisets: two services sharing one raw path would be two
    // distinct claims, and neither has to be printed twice.
    assert_eq!(
        scanned, attributed,
        "the documented label paths must be exactly `label_filter_field`'s, service by service"
    );

    // The server-authoritative decision and the reason for it.
    // PRESENCE-ONLY from here to the end of this test: why the decision was
    // made server-side, and the never-drop-a-row invariant it protects, are
    // design rationale — the invariant is not represented by any value. The
    // negatives keep a retired false pairing out.
    architecture.presence(
        PUSHDOWN,
        &[
            "Label mode is server-authoritative",
            "carry labels",
            "never-drop-a-row invariant",
            // Cloud Storage is out of the grammar, so it reports no label error
            // at all.
            "`filter_error` is always `None`",
            "matched *literally*",
        ],
    );
    // The document must not simultaneously claim Cloud Storage reports a label
    // error and that `filter_error` is always `None` for it, and a scoped
    // absence check would miss the claim reappearing in the other section —
    // which is where it was.
    architecture.absent(&["Cloud Run and Cloud Storage"]);
}

/// The numbered first-match list is the precedence contract, so the order the
/// document records has to be the order `render_active_overlay` actually
/// checks. A new overlay inserted into the code without being inserted here
/// would make the contract a lie.
// PRESENCE-ONLY: the expected order is retyped here, not read out of
// `render_active_overlay` — its arms are a `match` inside a private renderer
// with no list to walk. The relative-order checks on top of the substrings
// catch the *document* being reordered or an entry deleted, and never catch the
// *code* being reordered. Same shape as the forward-navigation list below.
#[test]
fn architecture_documents_the_real_overlay_precedence_order() {
    let architecture = Doc::architecture();

    // Pinned parsing, not a presence check: what is asserted below is the
    // *order* the names appear in, read out of the list itself. The list is cut
    // at the paragraph that follows it, which is tighter than the section and
    // is the boundary the ordering claim is about.
    let list = architecture.section_for_pinned_parsing("## Overlay precedence");
    let end = list
        .find("These are full-frame overlay responsibilities")
        .expect("the precedence list is followed by the full-frame paragraph");
    let list = &list[..end];

    let position = |needle: &str| {
        list.find(needle)
            .unwrap_or_else(|| panic!("the overlay precedence list mentions {needle}"))
    };

    // Every arm of `render_active_overlay`, in the order it checks them.
    let help = position("help");
    let picker = position("context picker");
    let action = position("resource-action confirmation");
    let batch = position("batch-action confirmation");
    let activation = position("configuration-activation confirmation");
    // A plugin's `Enter` runs the command, so its confirmation outranks the
    // copy-only preview. The list entry deliberately says "copy-only preview"
    // rather than naming the entry below it: "command preview" is the needle
    // this test reads the preview's own position from, and a second occurrence
    // above it would make that position meaningless.
    let plugin_confirmation = position("plugin confirmation");
    // Not "GCE SSH command preview": a needle naming one caller would go stale
    // the next time a second one is added rather than failing honestly here.
    let command_preview = position("command preview");
    let ssh_preview = position("GCE SSH launch preview");
    let k9s = position("k9s launch confirmation");
    let tunnel = position("tunnel launch preview");
    let jump_picker = position("jump picker");
    let palette = position("command palette");
    let batch_progress = position("batch progress");

    assert!(help < picker);
    assert!(picker < action);
    assert!(action < batch);
    assert!(batch < activation);
    assert!(activation < plugin_confirmation);
    assert!(plugin_confirmation < command_preview);
    assert!(command_preview < ssh_preview);
    assert!(ssh_preview < k9s);
    // The tunnel launch preview was placed deliberately: it is a launch
    // confirmation like the ones above it, and it must outrank the palette
    // because the palette is one of the ways it gets opened.
    assert!(k9s < tunnel);
    // The related-resource jump picker is ranked with the launch modals above
    // it and ahead of the palette, matching `services::runtime::input_context`:
    // whichever overlay renders is the one that owns the keys.
    assert!(tunnel < jump_picker);
    assert!(jump_picker < palette);
    assert!(palette < batch_progress);
}

// PRESENCE-ONLY: which module renders which workspace body is a wiring fact
// with no accessor — `ui::tunnels` is chosen by a private `match` on
// `Workspace`, so the attribution can only be read as prose.
#[test]
fn architecture_documents_the_tunnels_workspace_body_renderer() {
    let architecture = Doc::architecture();

    // Both words also appear in the tunnel chapter's layer split 700 lines
    // later, and the hand-rolled scope this replaces ran from the heading to
    // the end of the document, so both were reachable from there.
    architecture.presence(
        "## Workspace ownership",
        &["`ui::tunnels`", "tunnels projection"],
    );
}

// PRESENCE-ONLY: design rationale. "Never owns the terminal" and "outlives the
// keystroke that started it" are the reasons a separate seam was built; a seam
// that exists cannot report why it was not folded into another one.
#[test]
fn architecture_documents_why_a_tunnel_is_not_an_interactive_process() {
    let architecture = Doc::architecture();

    architecture.heading_exists("## Managed Tunnels");

    // The two properties that forced a separate seam rather than an extension
    // of the interactive-process runner.
    architecture.presence(
        "### Why a tunnel is not an interactive process",
        &[
            "never owns the terminal",
            "outlives the keystroke that started it",
            "InteractiveProcessRunner",
        ],
    );

    // The layer split and what each layer may not do. A subsection of its own,
    // next door to the one above: this test asserts claims from two sections
    // and now says which is which, rather than searching a 3000-line document
    // for both.
    architecture.presence(
        "### The four layers, and what each may not do",
        &[
            "nuvem_domain::gcp::tunnel",
            "services::runtime::tunnel",
            "app::state::tunnels",
        ],
    );
}

// PRESENCE-ONLY: design rationale. "`Drop` cannot await" is a language fact
// explaining a signature choice, not a value the signature exposes.
//
// STAYS A PRESENCE CHECK, deliberately — this is the first of the four
// rationale claims the audit recorded that decision for, and moving it onto
// `Doc::presence` changes where it looks, not what it claims. There is no
// machine-derivable fact behind "`Drop` cannot await": a pin over it would
// assert the implementation matches itself, and in the audit's words it *would
// look like evidence*, which is strictly worse than the presence check it
// replaced. A presence check is honest about buying only deletion detection;
// a self-asserting pin is not. Do not "finish" these into pins.
#[test]
fn architecture_documents_the_synchronous_tunnel_shutdown_contract() {
    let architecture = Doc::architecture();

    architecture.presence(
        "### Why `TunnelControl::shutdown` is synchronous",
        &[
            // Why `shutdown` is not async: a destructor cannot await one.
            "`Drop` cannot await",
            // The heading restates this one, which is why the body excludes it.
            "TunnelControl::shutdown",
            // Two paths, one synchronous primitive.
            "spawn_blocking",
            "TunnelRegistry`'s `Drop`",
            // Bare word, 4 occurrences document-wide: the neighbouring
            // signal-backstop subsection carries three of them, so unscoped
            // this could not fail.
            "backstop",
        ],
    );
}

// PRESENCE-ONLY: the kill strategy and its non-unix fallback live behind
// `#[cfg(unix)]` in a private module; there is no value that says "a process
// group is signalled here, a direct child there".
#[test]
fn architecture_documents_the_process_group_kill_and_its_non_unix_fallback() {
    let architecture = Doc::architecture();

    architecture.presence(
        "### Why the kill targets the process group",
        &[
            // The clearest of the three heading echoes: this heading states the
            // claim in the body's own words, so a haystack including it would
            // satisfy this needle after the paragraph was deleted.
            "process group",
            "wrapper script",
            "direct child",
            // The fallback's one distinguishing word. Three of its four
            // document-wide occurrences are unrelated — a `kubectl` probe, an
            // object-browser cache, the `Drop` backstop — so unscoped this
            // survived deleting the paragraph.
            "best-effort",
        ],
    );
}

// PRESENCE-ONLY: design rationale. That the probe is advisory rather than a
// reservation, and that the race between probe and bind is accepted, are
// statements about what the code deliberately does *not* guarantee — an absent
// guarantee has nothing to derive an expected value from.
//
// STAYS A PRESENCE CHECK for the reason recorded at the shutdown-contract test
// above: an accepted race is the absence of a guarantee, and a pin over an
// absence would only assert the implementation matches itself.
#[test]
fn architecture_documents_the_advisory_port_probe_and_its_race() {
    let architecture = Doc::architecture();

    architecture.presence(
        "### The port probe is advisory, not a reservation",
        &[
            // Heading echo, again: the heading is these exact words.
            "advisory, not a reservation",
            "between the probe and the child's",
            // What a child that cannot bind is left holding. Both words are
            // common enough document-wide to be meaningless unscoped: `Failed`
            // matches 31 lines, ~20 of them effect-enum variant names, and
            // `stderr` matches 17, most of them the diagnostics classifier.
            // Neither could fail before.
            "Failed",
            "stderr",
        ],
    );
}

/// The document used to claim a child that cannot bind "writes an error line
/// and exits" for every kind. `ssh` does neither on its own — it reports the
/// bind failure and keeps running — so the two things that make the claim true
/// have to be named.
// PRESENCE-ONLY: `ExitOnForwardFailure=yes` is an argument this crate passes to
// an external binary, and what that binary then does is not observable from
// here. The corrected claim can only be held as prose.
#[test]
fn architecture_documents_what_a_child_that_cannot_bind_really_does() {
    let architecture = Doc::architecture();

    // The corrected claim lives in the port-probe subsection, beside the race
    // it qualifies, which is why this test and the one above scope to the same
    // section.
    architecture.presence(
        "### The port probe is advisory, not a reservation",
        &[
            "ExitOnForwardFailure=yes",
            "keep running",
            "carries nothing",
        ],
    );
}

// PRESENCE-ONLY: the two tiers are a classification applied inside a private
// log-scanning routine. It exposes no table of patterns to walk, so which line
// is always-fatal and which is fatal only before readiness stays prose.
//
// STAYS A PRESENCE CHECK — the third of the four claims covered by the audit's
// recorded decision. The classification lives inside a private routine that
// exposes no table, so a pin would be transcribed from the same `match` it
// claims to check.
#[test]
fn architecture_documents_the_two_tier_post_readiness_error_policy() {
    let architecture = Doc::architecture();

    architecture.presence(
        "### Two tiers of error line",
        &[
            // Tier one: fatal whenever it appears, and the reason that tier
            // exists at all — an `OnSpawn` kind is ready from birth, so nothing
            // else can fail it. `always-fatal` also appears in the port-probe
            // subsection, which is what kept this green unscoped.
            "always-fatal",
            "cannot carry traffic",
            "ready from the instant it spawns",
            // Tier two: fatal only before readiness, because a healthy child
            // prints it.
            "channel 3: open failed",
            "only before readiness",
            // And what happens to a tier-two line on a ready tunnel instead of
            // nothing. `MESSAGE column` is also written down in the
            // failed-stop subsection.
            "notice",
            "MESSAGE column",
        ],
    );
}

// PRESENCE-ONLY: design rationale. Which signals are handled could in principle
// be derived, but *why* `SIGINT` is deliberately excluded — raw mode and the
// keymap already own it — is a decision, and an exclusion has no value to read.
//
// STAYS A PRESENCE CHECK — the last of the four. Why a signal is *excluded* is
// a decision, and an exclusion has no value to read: a pin would compare the
// handler list with itself and report agreement as evidence.
#[test]
fn architecture_documents_the_signal_backstop_and_why_sigint_is_excluded() {
    let architecture = Doc::architecture();
    const BACKSTOP: &str = "### The `SIGTERM`/`SIGHUP` backstop, and why `SIGINT` is not in it";

    // A heading-existence check, not a weak content check: that exact pair is
    // written down only in the heading, so there is no prose for a scoped
    // assertion to guard. `section_body` already panics when the heading is
    // gone; this states the claim rather than leaving it implied, and goes
    // through the entry point that says which of the two it is.
    architecture.heading_exists(BACKSTOP);
    architecture.presence(
        BACKSTOP,
        &[
            // Why it was needed: a tunnel child is in a session of its own.
            "never reaches them",
            // Why `SIGINT` is not in it.
            "`SIGINT` is deliberately excluded",
            "raw mode",
            // The thing that already owns `Ctrl-C`. Seven of its eight
            // document-wide occurrences are the input layer describing itself.
            "keymap",
        ],
    );
}

// PRESENCE-ONLY: "terminal is terminal" is an invariant of a private
// `live_entry_mut` lookup, and the two harms it prevents are consequences
// argued in prose. Neither is a value the state machine reports.
#[test]
fn architecture_documents_that_terminal_is_terminal_for_late_results() {
    let architecture = Doc::architecture();

    architecture.presence(
        "### Late results, and why terminal is terminal",
        &[
            // A fourth heading echo, found while scoping this one: the heading
            // is the claim, so this needle is satisfied by the heading alone
            // unless the body is read without it.
            "terminal is terminal",
            "live_entry_mut",
            // The two concrete harms a resurrected row causes.
            "promised to the allocator",
            "undismissable",
        ],
    );
}

// PRESENCE-ONLY: an ordering claim about two statements inside one private
// effect handler. Ordering is not a value, and the handler exposes no trace.
#[test]
fn architecture_documents_that_the_allocated_port_reaches_state_before_the_launch() {
    let architecture = Doc::architecture();

    // The ordering claim is stated in the port-probe subsection, where the
    // allocation it orders is described. `claimed_ports` is also named in the
    // kill-off-the-run-loop subsection.
    architecture.presence(
        "### The port probe is advisory, not a reservation",
        &["before the child is launched", "claimed_ports"],
    );
}

// PRESENCE-ONLY: the claim is about what a failed stop must *not* do — drop the
// control handle. Non-removal from a private registry leaves nothing to read.
#[test]
fn architecture_documents_that_a_failed_stop_keeps_its_control_and_reports_failed() {
    let architecture = Doc::architecture();

    // `stays registered` is written down three times in this chapter alone, so
    // deleting this subsection's copy changed nothing before it was scoped.
    architecture.presence(
        "### A stop that fails keeps its handle",
        &["stays registered", "a stop that did not work"],
    );
}

// PRESENCE-ONLY: design rationale. The deadline exists so a drifted readiness
// banner fails the tunnel instead of parking it forever; that reason is not a
// value, and the deadline itself is a private constant in the runtime.
#[test]
fn architecture_documents_the_bounded_readiness_deadline() {
    let architecture = Doc::architecture();

    architecture.presence(
        "### Why readiness detection is bounded",
        &[
            "readiness deadline",
            // The string the supervisor is waiting on. Two of its five
            // document-wide occurrences sit in neighbouring subsections of this
            // same chapter, so the word survived this subsection being emptied
            // of it.
            "banner",
            // A banner that drifted away must fail the tunnel, never park it
            // forever.
            "never waits forever",
            "bounded-gate",
        ],
    );
}

// PRESENCE-ONLY: design rationale for a scoping decision. Tunnel state sitting
// outside the per-context store is the *absence* of a scope key; the word
// "deliberately" is the part that matters and no type carries it.
#[test]
fn architecture_documents_that_tunnel_state_is_not_context_scoped() {
    let architecture = Doc::architecture();

    // The heading is "Tunnel state is not context-scoped", so the body has to
    // be read without it for the first needle to mean anything.
    architecture.presence(
        "### Tunnel state is not context-scoped",
        &["deliberately **not** context-scoped", "hide a live process"],
    );
}

// PRESENCE-ONLY throughout: the keys are asserted as bare substrings with
// nothing tying a key to the action it performs (unlike the mark keys above,
// which go through the binding catalog), and the three tunnel kinds are
// described by external tooling this crate only shells out to. The CLI flags
// could be pinned to the argument parser and are not.
#[test]
fn docs_document_the_managed_tunnel_workflow() {
    let readme = Doc::user_docs();

    // A section of its own explaining what the three tunnel kinds are. Scoping
    // to it is what gives the checks below any force: `external IP` matches
    // five lines README-wide, three of them Compute filter and detail
    // documentation, and `dismiss` matches four, one the batch progress panel.
    readme.presence(
        "# Tunnels",
        &[
            // Keys. The `|| readme.contains("`K`")` disjunct that used to be
            // beside `Shift-K` was dead: a bare `` `K` `` appears zero times in
            // the README, so it never once contributed a match.
            "`Ctrl-t`",
            "`Shift-K`",
            "dismiss",
            "start-iap-tunnel",
            "Cloud SQL Auth Proxy",
            "external IP",
            "loopback",
        ],
    );

    // The key reference, which is the only place this wording appears — the
    // Tunnels page says "Press `t` on a row [...] to start one" instead. The
    // key reference is a page of its own, so the whole page is the scope.
    readme.presence(KEYS_PAGE, &["start a tunnel"]);

    // Binary override, alongside the existing interactive-launch overrides.
    // These are documented where the other CLI flags are, not under Tunnels.
    readme.presence(
        "# Command line",
        &[
            "--cloud-sql-proxy-binary",
            "--k9s-binary",
            "--kubectl-binary",
        ],
    );

    // Heading-existence, same as the signal-backstop pair above: the section
    // having its own heading is the claim, and no prose inside it repeats one.
    readme.heading_exists("# Tunnels");
}

/// The README has to enumerate every way into Cloud Logging, because they
/// differ in scope and the two keyboard ones differ only in case.
///
/// The negative assertion is the load-bearing one: the paragraph used to claim
/// that `l` on a Cloud Run service opened resource-scoped logs and that the
/// viewer "arrives with #30". Both were corrected once the viewer shipped, and
/// a later edit that re-added either would be a regression a prose diff makes
/// easy to miss.
#[test]
fn docs_enumerate_every_cloud_logging_entry_point() {
    let readme = Doc::user_docs();

    // The three entry points, each with what it is scoped to.
    // PRESENCE-ONLY (this block): the scoping phrases are asserted as bare
    // substrings, with nothing pairing an entry point to its scope. The `L`
    // enumeration below is the pinned part.
    //
    // `all recent project logs` appears twice more in the README and
    // `Open logs for selected` once more, in the keybinding reference list, so
    // unscoped two of these four could not fail on deletion.
    readme.presence(
        CLOUD_LOGGING,
        &[
            "all recent project logs",
            "one resource the current",
            // Both palette titles carry the words around them, and that is not
            // decoration. Each title is written down two or three more times
            // inside this same section — the Cloud Run passage names both — so
            // scoped but bare, these two survived their own bullet being
            // deleted: the deletion probe over this diff caught them still
            // passing. A needle has to be as narrow as the claim it guards,
            // and the section argument cannot supply that on its own.
            "`Open project logs` (aliases",
            "`Open logs for selected ...` entries",
        ],
    );

    // `L` reaches every service, from both the list and the detail pane. The
    // count word and the enumeration are both derived from the registry: a
    // further service added without touching this sentence fails on the word,
    // and a renamed or invented noun fails on the set.
    // Derived from the services that *have* logs, not from every service.
    // Persistent disks do not: nothing writes logs as a disk, so a
    // resource-scoped query for one could only ever return zero rows. Counting
    // it here would make the README claim `L` reaches a service it reports an
    // error for. Service accounts are the same refusal: an account is an
    // identity that appears *in* log entries as the caller, not a resource
    // with a log stream of its own. VPC networks are the sharpest case of the
    // three: flow logs and firewall hit logs do exist, and Cloud Logging
    // attributes both to the *subnetwork*, so a network-scoped query returns
    // zero rows while looking to an operator like it should return many.
    // Firewall rules are that same case one step further and the most
    // dangerous of the four: a rule's hits *are* logged, against the
    // subnetwork the traffic crossed and with the rule named in the payload,
    // so a rule-scoped query returns zero rows — which on a DENY rule reads as
    // "nothing was blocked".
    let with_logs: Vec<&ResourceServiceKind> = ResourceServiceKind::ALL
        .iter()
        .filter(|kind| {
            !matches!(
                kind,
                ResourceServiceKind::PersistentDisks
                    | ResourceServiceKind::IamServiceAccounts
                    | ResourceServiceKind::IamPolicy
                    | ResourceServiceKind::VpcNetworks
                    | ResourceServiceKind::FirewallRules
            )
        })
        .collect();
    let count = match with_logs.len() {
        4 => "four",
        5 => "five",
        6 => "six",
        7 => "seven",
        8 => "eight",
        9 => "nine",
        10 => "ten",
        11 => "eleven",
        other => panic!("no count word for {other} services; add one"),
    };
    let opening = format!("`L` covers all {count} services: the selected ");
    // Pinned parsing: the enumeration is compared with the registry.
    let logging = unwrapped(&readme.section_for_pinned_parsing(CLOUD_LOGGING));
    let at = logging
        .find(&opening)
        .unwrap_or_else(|| panic!("README says {opening:?}"));
    let body = &logging[at + opening.len()..];
    let close = body
        .find(". ")
        .expect("the enumeration is closed by the end of its sentence");
    // Sorted `Vec`s rather than sets: a set would swallow a noun listed twice,
    // leaving six items documented under the word "five".
    let mut documented: Vec<&str> = body[..close]
        .split(',')
        .map(|noun| noun.trim().trim_start_matches("or ").trim())
        .collect();
    documented.sort_unstable();
    let mut real: Vec<&str> = with_logs.iter().map(|kind| kind.selection_noun()).collect();
    real.sort_unstable();
    assert_eq!(
        documented, real,
        "the `L` sentence must name exactly the services that produce logs"
    );
    // PRESENCE-ONLY: that `L` also works from the detail pane is an input-routing
    // fact with no accessor. Seven README lines say `resource detail pane`, one
    // of them in this section, so this is the clearest case in the file of an
    // assertion that could not fail before it was scoped.
    readme.presence(CLOUD_LOGGING, &["resource detail pane"]);

    // The removed seam text and the misattributed trigger must stay removed.
    // PRESENCE-ONLY: two negatives holding corrected claims out. Neither the
    // retired seam text nor the misattributed `l` trigger corresponds to a fact
    // that still exists, so there is nothing left to derive them from.
    readme.absent(&[
        "arrives with #30",
        "Press `l` on a selected Cloud Run service",
    ]);
}

/// `J`'s contract, in the two places it is easy to get wrong: which jumps
/// exist per service, and that an inapplicable selection reports rather than
/// opening an empty modal.
///
/// The per-service table is now the *default* rule set rather than the only
/// one there can be, and the empty-picker message says "configured" for the
/// same reason: a user who wrote a GKE rule is not told GKE has no jumps. The
/// old wording is asserted absent, since a prose edit that put it back would
/// document a message the app no longer emits.
#[test]
fn docs_document_the_related_resource_jump_picker() {
    let readme = Doc::user_docs();

    // PRESENCE-ONLY (this block): the heading, and the key with nothing tying
    // it to the jump action. The tables below are pinned.
    readme.heading_exists(JUMPS);
    // The `readme.contains("`Shift-J`") ||` disjunct this replaces was carried
    // by the keybinding reference list at the end of the README, not by this
    // section. Scoped, only `Press `J`` is here, and this section documenting
    // its own key is what the assertion is for.
    readme.presence(JUMPS, &["Press `J`"]);

    // Per-service targets, walked against `JumpRules::defaults()`.
    //
    // A `contains("Network, Disks")` proved only that the pair appears
    // somewhere in a 1258-line document: crediting Cloud SQL with the disks
    // jump and GCE with `Replicas` would have kept every one of those calls
    // green. Each row is now matched to a service by its `selection_noun` —
    // those are exactly the Selection labels — and its Jumps cell has to be
    // the `DetailTab::label`s of that service's default rules, in order.
    // Pinned parsing: each row is matched to a service and its cell compared
    // with that service's default rules.
    let jumps_section = readme.section_for_pinned_parsing(JUMPS);
    let mut lines = jumps_section
        .lines()
        .skip_while(|line| !(line.starts_with("| Selection") && line.contains("| Jumps offered")));
    lines
        .next()
        .expect("README documents the default jumps as a table");
    let rows: Vec<&str> = lines
        .skip(1)
        .take_while(|line| line.starts_with('|'))
        .collect();
    let cell = |row: &str, column: usize| -> String {
        row.split('|')
            .nth(column)
            .unwrap_or_else(|| panic!("jumps row {row:?} has a column {column}"))
            .trim()
            .to_string()
    };

    let defaults = JumpRules::defaults();
    let mut documented_selections: Vec<String> = Vec::new();
    let mut saw_revision_row = false;
    for row in &rows {
        let selection = cell(row, 1);
        let offered = cell(row, 2);
        let Some(kind) = ResourceServiceKind::ALL
            .iter()
            .copied()
            .find(|kind| kind.selection_noun() == selection)
        else {
            // The one row that names no service: a Cloud Run revision is a
            // sub-view of a Cloud Run service, not a `ResourceServiceKind`, so
            // there is no rule set to walk for it. Its "none" is a hard-coded
            // refusal in `no_jumps_reason`, whose message the README quotes and
            // this test checks below. Any *other* unknown selection is a row
            // for a service that does not exist.
            assert_eq!(
                selection, "Cloud Run revision",
                "the jumps table row {selection:?} names no known selection"
            );
            assert_eq!(
                offered, "none",
                "a Cloud Run revision can offer no jumps at all"
            );
            saw_revision_row = true;
            continue;
        };
        documented_selections.push(selection.clone());

        let real: Vec<&str> = defaults
            .for_service(kind)
            .map(|rule| rule.to.label())
            .collect();
        let expected = if real.is_empty() {
            "none".to_string()
        } else {
            real.join(", ")
        };
        assert_eq!(
            offered, expected,
            "the jumps table credits {selection:?} with {offered:?}"
        );
    }

    // Both directions: every service has a row, and no row names a selection
    // twice. Sorted `Vec`s rather than sets, so a duplicated row is a mismatch
    // and not something the comparison swallows.
    documented_selections.sort();
    let mut real_selections: Vec<String> = ResourceServiceKind::ALL
        .iter()
        .map(|kind| kind.selection_noun().to_string())
        .collect();
    real_selections.sort();
    assert_eq!(
        documented_selections, real_selections,
        "the jumps table must have a row for exactly the services that exist"
    );

    // The round-trip above covers the five real services; the revision row is
    // the one row it cannot speak for, so its *presence* is required here.
    // Without this the branch above only says what the row must contain if it
    // is there at all, and deleting it would drop the documentation of the
    // refusal without a single test noticing.
    assert!(
        saw_revision_row,
        "the jumps table must keep its `Cloud Run revision` row: the refusal is \
         hard-coded and this table is the only place it is published"
    );

    // No empty picker: the status line explains which of the reasons it is.
    // PRESENCE-ONLY (this line): a claim about what the picker never does. The
    // quoted messages below are pinned to the nouns they are built from.
    readme.presence(JUMPS, &["never opens an empty picker"]);

    // The two quoted "no rule for this service" messages, rebuilt from the
    // nouns rather than retyped, and each paired with the fact that makes it
    // true — the service really has no default rule. A service that gained one
    // would fail here rather than leaving the README quoting a message the app
    // no longer emits for it.
    //
    // One-way on the wording: `no_jumps_reason` is private to
    // `src/app/reducer/jumps.rs`, so the format string is reproduced here and
    // is *not* pinned — only the noun in it is. `tests/resource_jump_picker_test.rs`
    // is what holds the wording itself against the real producer.
    for kind in [
        ResourceServiceKind::GkeClusters,
        ResourceServiceKind::CloudStorage,
    ] {
        assert!(
            defaults.for_service(kind).next().is_none(),
            "{kind:?} must offer no default jump for the quoted message to be its message"
        );
        let message = format!(
            "No related-resource jumps configured for {}s",
            kind.selection_noun()
        );
        readme.presence(JUMPS, &[message]);
    }
    // PRESENCE-ONLY: the superseded wording, retyped for the same reason —
    // there is no producer to derive a message the code no longer emits from.
    // This catches a prose edit that puts the earlier sentence back.
    readme.absent(&["No related-resource jumps for GKE clusters"]);

    // The Cloud Run revision refusal, which the `none` row above defers to.
    // Also a literal for the reason given above.
    // PRESENCE-ONLY: `no_jumps_reason` is private, so this message is retyped
    // rather than produced. `tests/resource_jump_picker_test.rs` holds the
    // wording against the real producer.
    readme.presence(
        JUMPS,
        &["No related-resource jumps from a Cloud Run revision"],
    );

    // The "nothing selected" form comes off the same noun table, and is one-way
    // in exactly the same place as the two messages above: the noun is derived
    // from `selection_noun`, the format string around it is reproduced from the
    // private `no_jumps_reason` and is *not* pinned.
    // `tests/resource_jump_picker_test.rs` holds that wording.
    let nothing_selected = format!(
        "No {} selected to jump from",
        ResourceServiceKind::ComputeInstances.selection_noun()
    );
    readme.presence(JUMPS, &[nothing_selected]);

    // The table is a default, and the section says where to change it.
    // PRESENCE-ONLY (this line): a cross-reference to another section, not a
    // claim with a value behind it. `[[jumps]]` is written down five times in
    // the README, four of them in the section it cross-references *to*, so
    // unscoped this could not notice the cross-reference going away.
    readme.presence(JUMPS, &["`[[jumps]]`"]);
}

/// `[[jumps]]` is the second config section whose vocabulary is derived from
/// enums rather than written out (the first being `[columns]`), so the README
/// is the only place a user can read that vocabulary. Three of these claims
/// are the ones a wrong document turns into wasted debugging: which key does
/// what, that a present array *replaces* the defaults, and that one bad rule
/// costs only itself.
#[test]
fn docs_document_configurable_jump_rules() {
    let readme = Doc::user_docs();

    // PRESENCE-ONLY (this block): the five key names are retyped here rather
    // than derived — `from_file` hands each entry to a private `validate` that
    // reads the TOML keys by hand, so there is no key list to walk. The two
    // sample `service:tab` ids are examples, and the Cloud Storage sentence
    // explains an absence. The `{field}` table below is the pinned part.
    readme.heading_exists(JUMP_RULES);

    // The five keys, and the `service:tab` vocabulary, and the service that
    // cannot be a destination because it has no detail tabs at all. Every one
    // of these is a bare word documented in several other sections too —
    // `` `name` `` alone matches seventeen README lines — so the section
    // argument is what makes any of them able to fail.
    readme.presence(
        JUMP_RULES,
        &[
            "`from`",
            "`to`",
            "`name`",
            "`location`",
            "`label`",
            "`compute:network`",
            "`gke:node-pools`",
            "Cloud Storage has no detail tabs",
        ],
    );

    // The two claims about which services a *list* `to` reaches. Both are
    // prose the code can contradict silently: the count moves the day a
    // seventeenth service lands, and `to = "iam-policy"` being refused is a
    // decision a later slice could reverse without anyone reading this section.
    // Derived rather than retyped, so the sentence and the predicate cannot
    // drift apart.
    let unlandable: Vec<ResourceServiceKind> = ResourceServiceKind::ALL
        .into_iter()
        .filter(|kind| !JumpTarget::List(*kind).is_landable())
        .collect();
    assert_eq!(
        unlandable,
        vec![ResourceServiceKind::IamPolicy],
        "the README names the IAM policy table as the only service a list `to` \
         cannot reach, and says fifteen of the sixteen can"
    );
    readme.presence(
        JUMP_RULES,
        &[
            "Fifteen of the sixteen services are legal list destinations",
            "`to = \"iam-policy\"` is refused at load",
        ],
    );

    // The claims about the two detail-backed fields. Which fields
    // those are is derived, not retyped, so the sentence cannot outlive the
    // table; the wording of the diagnostic it promises is quoted from the
    // section, because that string is the whole user-visible deliverable of the
    // slice and prose is the only place it is written down twice.
    let compute = ResourceServiceKind::ComputeInstances;
    let detail_backed: Vec<&str> = detail_backed_fields(compute)
        .iter()
        .map(|field| {
            fields_for(compute)
                .iter()
                .find_map(|(name, known)| (known == field).then_some(*name))
                .unwrap_or_else(|| panic!("{field:?} is detail-backed but is not a compute field"))
        })
        .collect();
    assert_eq!(
        detail_backed,
        vec!["network", "subnetwork", "subnetwork_region"],
        "the README names exactly these compute fields as detail-backed"
    );
    readme.presence(
        JUMP_RULES,
        &[
            "`{network}`, `{subnetwork}` and `{subnetwork_region}` on `compute` are the only fields that do not come from the row",
            "needs network, which is on this GCE instance's detail — open it and try again",
            // The two claims about the region: how it is obtained, and how it is
            // deliberately *not* obtained. The second is the one worth pinning
            // — the zone-stripping guess is right on most topologies, so a
            // README that quietly started recommending it would read fine.
            "from the segment following `regions`",
            "`regions`, not by stripping the last part off the instance's `{zone}`",
            "There is no `{network_region}` beside it",
        ],
    );

    // The `{field}` vocabulary is per source service, so the fields that exist
    // for one service and not another have to be listed per service — and the
    // five cherry-picked braces this replaces never said which service any of
    // them belonged to. Each row's brace list is now compared with
    // `fields_for` for that row's own service, so documenting `{engine}` under
    // `compute` fails as loudly as omitting it under `cloudsql`.
    //
    // The row's `from` id is resolved by running it through the real parser:
    // `config::resolve_service_kind` is `pub(crate)`, and going through
    // `JumpRules::from_file` is the stronger pin anyway — it proves the id the
    // README publishes is an id a user's file may really use.
    {
        // Pinned parsing: each row's brace list is compared with `fields_for`.
        let section = readme.section_for_pinned_parsing(JUMP_RULES);
        let mut lines = section
            .lines()
            .skip_while(|line| !(line.starts_with("| `from`") && line.contains("| fields")));
        lines
            .next()
            .expect("README documents the template fields as a table");
        let rows: Vec<&str> = lines
            .skip(1)
            .take_while(|line| line.starts_with('|'))
            .collect();

        let mut documented_kinds: Vec<ResourceServiceKind> = Vec::new();
        for row in &rows {
            let mut cells = row.split('|').skip(1);
            let id = cells
                .next()
                .and_then(|cell| cell.split('`').nth(1))
                .unwrap_or_else(|| panic!("field-table row {row:?} backticks a `from` id"));
            let fields_cell = cells
                .next()
                .unwrap_or_else(|| panic!("field-table row {row:?} has a fields column"));

            let source = format!("[[jumps]]\nfrom = \"{id}\"\nto = \"compute:network\"\n");
            let document: toml::Value = toml::from_str(&source).unwrap();
            let mut warnings = Vec::new();
            let parsed = JumpRules::from_file(
                document.get("jumps").expect("a [[jumps]] array"),
                &mut warnings,
            );
            assert!(
                warnings.is_empty(),
                "the README publishes {id:?} as a `from` id the parser accepts: {warnings:?}"
            );
            let kind = parsed
                .rules()
                .first()
                .unwrap_or_else(|| panic!("{id:?} produced no rule"))
                .from;
            documented_kinds.push(kind);

            // Sorted `Vec`s rather than sets: a field listed twice in a row is
            // a defect in the table, not something to collapse.
            let mut documented: Vec<&str> = fields_cell
                .split('`')
                .skip(1)
                .step_by(2)
                .map(|token| {
                    token
                        .strip_prefix('{')
                        .and_then(|token| token.strip_suffix('}'))
                        .unwrap_or_else(|| panic!("field-table row {row:?} lists {token:?}"))
                })
                .collect();
            documented.sort_unstable();
            let mut real: Vec<&str> = fields_for(kind).iter().map(|(name, _)| *name).collect();
            real.sort_unstable();
            assert_eq!(
                documented, real,
                "the field table claims {id:?} offers {documented:?}"
            );
        }

        // Both directions on the rows themselves: every service is listed, and
        // no service is listed twice under two different ids.
        assert_eq!(
            documented_kinds.len(),
            ResourceServiceKind::ALL.len(),
            "the field table has one row per service, no more and no fewer"
        );
        for kind in ResourceServiceKind::ALL {
            assert!(
                documented_kinds.contains(&kind),
                "the field table has no row for {kind:?}"
            );
        }
    }

    // The services with no location field, pinned to `location_field` rather
    // than presence-checked.
    //
    // This sentence was wrong for two whole services before it was pinned: it
    // named Secret Manager and Pub/Sub topics only, while `location_field`
    // already answered `None` for service accounts and then for the IAM policy
    // too. An operator reading it concluded `iam-policy` had a location, wrote
    // a rule with `location = "{name}"`, and got the load-time rejection the
    // sentence promised could not happen. A presence check cannot catch that —
    // the needle stays present while the list under it goes stale — so the
    // sentence's own ids are resolved through the parser and compared with the
    // predicate in both directions.
    {
        // Pinned parsing: the sentence's backticked ids are compared with
        // `location_field`.
        let section = readme.section_for_pinned_parsing(JUMP_RULES);
        let sentence = unwrapped(&section)
            .split(". ")
            .find(|sentence| sentence.contains("no location field at all"))
            .unwrap_or_else(|| panic!("README names the services with no location field at all"))
            .to_string();

        let mut documented: Vec<ResourceServiceKind> = Vec::new();
        for id in sentence.split('`').skip(1).step_by(2) {
            let source = format!("[[jumps]]\nfrom = \"{id}\"\nto = \"compute:network\"\n");
            let document: toml::Value = toml::from_str(&source).unwrap();
            let mut warnings = Vec::new();
            let parsed = JumpRules::from_file(
                document.get("jumps").expect("a [[jumps]] array"),
                &mut warnings,
            );
            assert!(
                warnings.is_empty(),
                "the locationless sentence names {id:?} as a `from` id the \
                 parser accepts: {warnings:?}"
            );
            documented.push(
                parsed
                    .rules()
                    .first()
                    .unwrap_or_else(|| panic!("{id:?} produced no rule"))
                    .from,
            );
        }
        documented.sort_by_key(|kind| format!("{kind:?}"));

        let mut locationless: Vec<ResourceServiceKind> = ResourceServiceKind::ALL
            .into_iter()
            .filter(|kind| location_field(*kind).is_none())
            .collect();
        locationless.sort_by_key(|kind| format!("{kind:?}"));

        assert_eq!(
            documented, locationless,
            "the README's locationless sentence must name exactly the services \
             `location_field` answers `None` for"
        );
    }

    // Replacement, not merge — the trap a user finds only after wanting a
    // built-in jump gone.
    //
    // The claim is checked by loading the document's own worked example
    // through the real parser rather than by looking for the sentence that
    // makes it. A sentence survives the behaviour it describes turning false;
    // a rule set does not. If `[[jumps]]` ever merged instead, the built-in
    // Cloud SQL and Cloud Run rules would stand beside the three the example
    // writes, and both the count and the empty services below would say so.
    // PRESENCE-ONLY (this line only): the sentence's wording. The block below
    // loads the document's own example through the real parser, which is what
    // actually holds the replacement behaviour.
    readme.presence(JUMP_RULES, &["replaces the built-in set"]);
    {
        let example = toml_block_before(
            readme.text_for_pinned_parsing(),
            "A `[[jumps]]` array replaces the built-in set",
        );
        assert_eq!(
            example.matches("[[jumps]]").count(),
            3,
            "the worked example is the three rules the prose walks through"
        );

        let document: toml::Value = toml::from_str(&example).expect("the example is valid TOML");
        let mut warnings = Vec::new();
        let configured = JumpRules::from_file(
            document
                .get("jumps")
                .expect("the example is a [[jumps]] array"),
            &mut warnings,
        );
        assert!(
            warnings.is_empty(),
            "the documented example must load cleanly: {warnings:?}"
        );
        assert_eq!(
            configured.rules().len(),
            3,
            "a present [[jumps]] is the whole rule set, not an addition to the defaults"
        );
        for replaced in [
            ResourceServiceKind::CloudSql,
            ResourceServiceKind::CloudRunServices,
        ] {
            assert!(
                JumpRules::defaults().for_service(replaced).next().is_some(),
                "{replaced:?} must have a built-in rule for its absence to mean anything"
            );
            assert!(
                configured.for_service(replaced).next().is_none(),
                "{replaced:?}'s built-in rule survived a [[jumps]] array that never mentions it"
            );
        }
    }

    // Per-rule rejection, with a real warning as it is really worded.
    // PRESENCE-ONLY: the warning text is retyped rather than produced by
    // feeding a bad rule to `JumpRules::from_file` and reading the warning
    // back, and "the rules beside it keep working" is a claim about isolation
    // that the warning string does not carry.
    readme.presence(
        JUMP_RULES,
        &["ignoring this rule", "the rules beside it keep working"],
    );

    // And that a rule can be valid at load time and still not be offered for
    // a given row, which no amount of config checking can prevent.
    // PRESENCE-ONLY: load-time validity versus per-row applicability is a
    // distinction drawn in prose; neither half is a value.
    readme.presence(JUMP_RULES, &["not offered for that row"]);

    // That suppression is about the *key*. A `label` is wording, so it falls
    // back instead — a user told only the first half would read the fallback
    // as a bug and go looking for the rule that "did not apply".
    //
    // What that fallback wording *is* comes from the tab's own label, so the
    // captions the document quotes are rebuilt from `DetailTab::label` rather
    // than copied: renaming a tab changes the caption a user sees, and the
    // document has to be renamed with it instead of going quietly stale.
    // PRESENCE-ONLY (these two lines): the fallback rule and its template
    // shape. The captions checked in the loop below are rebuilt from
    // `DetailTab::label` and are pinned.
    readme.presence(
        JUMP_RULES,
        &[
            "falls back to the built-in wording",
            "`\"<tab> of <name>\"`",
        ],
    );
    for (tab, resource) in [
        (DetailTab::Compute(ComputeDetailTab::Network), "web-01"),
        (
            DetailTab::CloudSql(CloudSqlDetailTab::Replicas),
            "orders-db",
        ),
    ] {
        let wording = format!("{} of {resource}", tab.label());
        // The captions themselves are published in the jump-picker section,
        // which is where a reader meets them; this section states the template
        // they are built from.
        readme.presence(JUMPS, &[wording]);
    }

    // Both halves of a rule name a service out of one vocabulary. Saying so
    // only for `from` is what made `to = "gce:disks"` read as a typo in the
    // document while being a working rule in the code.
    // PRESENCE-ONLY: that both halves draw on one vocabulary is a statement
    // about the parser's shape; the example id itself is covered by the
    // destination-tab table test below.
    readme.presence(JUMP_RULES, &["`to = \"gce:disks\"`"]);

    // A rule that is offered still only *lands* on a row the destination
    // service has already loaded, keyed on name and location both. Without
    // this, a cross-service rule reads as though writing it were enough.
    // PRESENCE-ONLY: the landing rule and its (name, location) key are matched
    // inside a private reducer with no accessor for the key it uses.
    // The landing rule is stated in both jump passages; this is the rule
    // section's copy, and the other is asserted below.
    readme.presence(
        JUMP_RULES,
        &[
            "already in the destination service's loaded list",
            "name and its zone/region",
        ],
    );

    // What a jump into a never-loaded destination does, in the words the
    // status line actually uses. Asserting the *messages* rather than a
    // paraphrase is what keeps this moving with the behaviour: all three are
    // produced by `reducer::navigation::unavailable`, and a reworded one has
    // to be reworded here too.
    readme.presence(
        JUMP_RULES,
        &[
            "Loading GKE clusters, so web-01-cluster cannot be opened yet — try again once they arrive",
            "No GKE cluster loaded yet, so web-01-cluster cannot be opened",
            "web-01-cluster is no longer available",
        ],
    );
    // PRESENCE-ONLY from here to the end of this test: the three messages above
    // are retyped, not produced — `reducer::navigation::unavailable` is private
    // — and everything below is prose about fetch and backoff behaviour behind
    // the same seam. The two negatives and the `count() == 2` hold specific
    // false claims out, which is the strongest a substring gets.
    readme.presence(JUMPS, &["the jump starts its fetch"]);
    // And that only the never-loaded list gets it: the document said the
    // opposite of the behaviour for one release, so the claim it replaced is
    // pinned as absent rather than merely superseded.
    readme.absent(&["Nothing new is fetched to *follow*"]);

    // Claims that run across a line break are asserted against the prose with
    // its wrapping collapsed: a reflow is not a change of meaning, and an
    // assertion that a reflow can break is one that gets weakened rather than
    // fixed.
    readme.presence(JUMPS, &["waiting on the retry backoff, is left alone"]);
    // Suppression is per configuration, which is the difference between "that
    // backend is broken" and "that backend has not been called here". Stated
    // in both passages, because a reader who finds only one of them would take
    // the unqualified claim as the whole rule.
    //
    // This replaces a document-wide `matches(...).count() == 2`, which said
    // that the phrase appears twice somewhere. Requiring it in each of the two
    // passages by name is what that count was standing in for: two copies in
    // one passage and none in the other satisfied the count.
    for passage in [JUMP_RULES, JUMPS] {
        readme.presence(passage, &["*in this configuration*"]);
    }
    readme.presence(
        JUMP_RULES,
        &["A failure inherited from a configuration you have since left does not count"],
    );
    // Stated in the picker passage, not the rule passage. The old assertions
    // searched the whole document, so their being adjacent in this test said
    // nothing about their being adjacent in the README — and they are not.
    readme.presence(
        JUMPS,
        &["switching configuration and jumping again fetches"],
    );

    // `{status}` is advice about what a name is, not about spaces: most status
    // labels have none, so "they never pass the check" was false for exactly
    // the values a user would try first.
    readme.absent(&["status labels contain spaces"]);
}

/// The `[favorites]` block in the config file page's full example is fed to the real
/// `FavoriteScopes::from_file` parser and checked against the exact values it
/// claims to produce. A wrong key name, a stray bracket, or a value that
/// `from_file` would reject fails this test instead of shipping a config
/// example a reader's first copy-paste cannot use.
#[test]
fn docs_favorites_schema_example_parses_to_the_values_it_shows() {
    let readme = Doc::user_docs();
    // Pinned parsing: the documented example is fed to the real parser.
    let example = toml_block_before(
        &readme.section_for_pinned_parsing(CONFIG_FILE_PAGE),
        "## Keys",
    );
    let document: toml::Value = toml::from_str(&example).expect("the schema example is valid TOML");
    let favorites_value = document
        .get("favorites")
        .expect("the schema example has a [favorites] table");

    let mut problems = ConfigProblems::default();
    let favorites = FavoriteScopes::from_file(favorites_value, &mut problems);
    assert!(
        problems.into_messages().is_empty(),
        "the documented [favorites] example must be valid, warning-free config"
    );
    assert_eq!(
        favorites.values(FavoriteKind::Project),
        &["prod-project".to_string()]
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

/// `[favorites]`'s own prose subsection states three facts a reader cannot
/// verify by staring at the schema example: which key feeds which resource
/// kind, and that the whole table is optional. The key-name assertions below
/// iterate `favorites::KEYS` itself, the single vocabulary both the parser
/// and this test read from, and require the README to name every key it
/// contains — so a fourth key added there, or a key renamed there, fails
/// this test unless the README is updated to match. (The check does not
/// care about `KEYS`' order, only its membership, matching the README's own
/// prose, which does not promise a particular key order either.)
#[test]
fn docs_document_the_favorites_keys() {
    let readme = Doc::user_docs();
    const FAVORITES: &str = "# Favorites";

    // The key names are derived from `favorites::KEYS`, the single vocabulary
    // both the parser and this test read from, and still go through the scoped
    // entry point: a derived needle is no better at telling one section from
    // another than a typed one. The hand-rolled `### `/`## ` scan this replaces
    // is now `section_body`'s job.
    let keys: Vec<String> = KEYS.iter().map(|(key, _)| format!("`{key}`")).collect();
    readme.presence(FAVORITES, &keys);

    // PRESENCE-ONLY (this block): "optional", "ordered first" / "marked with
    // `*`" and the "ordering, not a filter escape" rule are UI behavior
    // described in prose, not values `FavoriteScopes` exposes — there is no
    // code-side fact to derive them from beyond the `is_favorite`/
    // `visible_items` logic these sentences summarize in words.
    readme.presence(FAVORITES, &["optional", "*", "filter"]);
}

/// The destination-tab table is the only published copy of the `to`
/// vocabulary, and that vocabulary is *derived* — `DetailTab::config_key` over
/// `DetailTab::ALL`. A tab added to any service's `ALL` therefore becomes a
/// legal `to` the moment it exists, silently, while the README goes on listing
/// the old set.
///
/// That is the same drift `[columns]`' catalog and `DetailTab::from_config_key`
/// were both built to make impossible in code, so the document is held to it
/// too: every key must be listed, and nothing may be listed that is not a key
/// (which catches a typo in the table as well as a tab that has been removed).
#[test]
fn docs_list_every_detail_tab_a_jump_rule_may_name() {
    use nuvem::app::state::navigation::DetailTab;

    // Pinned parsing: every row's keys are compared with `DetailTab`, in both
    // directions.
    let readme = Doc::user_docs();
    let jump_rules_section = readme.section_for_pinned_parsing(JUMP_RULES);

    // The table's own rows: the `|` lines following its header. The header is
    // matched by its columns rather than by a padded literal, so reflowing the
    // table's whitespace does not report itself as the table having been
    // deleted — same shape as the three other table walks in this file.
    let mut lines = jump_rules_section
        .lines()
        .skip_while(|line| !(line.starts_with("| service") && line.contains("| destination tabs")));
    lines
        .next()
        .expect("README documents the destination tabs as a table");
    let rows: Vec<&str> = lines
        .skip(1)
        .take_while(|line| line.starts_with('|'))
        .collect();

    // Every backticked token in the tabs column is a claimed key, and it has to
    // belong to the row it sits in. Flattening the whole table into one list
    // held only the *set*: moving `compute:network` into the `gke` row left
    // every assertion below green while the table credited the wrong service
    // with the tab. The key's own prefix is the service, so the row's service
    // cell and the keys beside it must agree.
    let mut listed: Vec<String> = Vec::new();
    for row in &rows {
        let mut cells = row.split('|').skip(1);
        let service = cells
            .next()
            .and_then(|cell| cell.split('`').nth(1))
            .unwrap_or_else(|| panic!("destination-tab row {row:?} backticks a service"));
        let keys = cells
            .next()
            .unwrap_or_else(|| panic!("destination-tab row {row:?} has a tabs column"));
        for key in keys.split('`').skip(1).step_by(2) {
            assert!(
                key.starts_with(&format!("{service}:")),
                "the destination-tab table lists {key:?} in the {service:?} row"
            );
            listed.push(key.to_string());
        }
    }

    for tab in DetailTab::ALL {
        let key = tab.config_key();
        assert!(
            listed.contains(&key),
            "the destination tab table must list {key}, which is a legal `to` today"
        );
    }
    for key in &listed {
        assert!(
            DetailTab::from_config_key(key).is_some(),
            "the destination tab table lists {key}, which names no tab"
        );
    }
}

/// The forward-navigation gate is the single place history is written, so the
/// document's list of what is in it is a contract like the overlay precedence
/// list. It had already rotted once — it named `ResourcesAction::CycleService`,
/// an action that no longer exists, while omitting three that do — which is
/// exactly the drift a name-by-name assertion catches.
// PRESENCE-ONLY: the seven action names are retyped here, not walked out of
// `is_forward_nav_action` — it is a private `match` over `Action`, with no
// variant list to enumerate. So this catches a name being dropped from the
// document, and does not catch the gate gaining an eighth member. The negative
// on `CycleService` is what keeps the already-rotted claim from returning.
// Same shape as the overlay precedence list above.
#[test]
fn architecture_lists_every_forward_navigation_action() {
    let architecture = Doc::architecture();

    architecture.presence(
        "## Navigation History",
        &[
            "is_forward_nav_action",
            "ResourcesAction::OpenSelected",
            "ResourcesAction::SelectService",
            "ResourcesAction::OpenLogsForSelection",
            "NavigatorAction::OpenSelected",
            "CommandAction::Submit",
            "JumpsAction::Submit",
            // Why the gate may hold actions that do not always navigate.
            "captured location actually changed",
            "might* navigate",
        ],
    );

    // Removed action: naming it again would document a gate member that cannot
    // be dispatched. Document-wide rather than scoped, because that is what an
    // absence check is worth — see `Doc::absent`. It is also stronger than the
    // section-scoped negative it replaces: re-adding `CycleService` anywhere in
    // ARCHITECTURE.md now fails.
    architecture.absent(&["CycleService"]);
}

/// Both documents enumerate the Home inventory's services, and the enumeration
/// is the registry's.
///
/// The sentence is built from `ResourceServiceKind::ALL` rather than retyped, so
/// it fails three ways: a service added without touching the prose (the missing
/// label), a service renamed (the label itself), and a noun invented or
/// reordered in the prose (the closing parenthesis is part of the needle, so a
/// list with an extra entry, a dropped entry or a different order does not
/// match). Both documents named the same hand-written four — "GCE instances,
/// GKE clusters, Cloud SQL instances, Cloud Run services" — while the registry
/// had seven, three of which had no Home entry at all and two more of which
/// never painted. A hand-written list is what this pin exists to keep
/// out, which is also why the expected list is not written down here.
#[test]
fn home_inventory_prose_enumerates_every_service() {
    let expected = ResourceServiceKind::ALL
        .into_iter()
        .map(|kind| kind.label())
        .collect::<Vec<_>>()
        .join(", ");
    let sentence = format!("one line per service ({expected})");

    for (doc, heading) in [
        (Doc::user_docs(), HOME_README),
        (Doc::architecture(), HOME_ARCHITECTURE),
    ] {
        // The needle is built from the registry rather than written down, so a
        // service added to `ALL` and not to the prose fails here — but the
        // check itself is a presence check and goes through `presence`, not
        // through the raw-text escape hatch that entry point exists to keep
        // out of presence checks.
        doc.presence(heading, &[sentence.as_str()]);
    }
}

/// The BigQuery page carries the three command lines BigQuery really issues,
/// and none of a neighbouring service's.
///
/// What marks the boundary is the neighbours' **own command fragments**, not
/// the word `gcloud`: BigQuery prose has ordinary reasons to name the tool it
/// is being contrasted with ("`bq` has no `--configuration` flag", the
/// `CLOUDSDK_ACTIVE_CONFIG_NAME` variable gcloud reads), and a pin that a true
/// sentence can falsify gets "fixed" by deleting the true sentence.
///
/// `secrets versions list` and `projects get-iam-policy` cannot be written
/// truthfully about BigQuery at all — they are the Secret Manager and IAM
/// policy pages naming their own subcommands. Each is asserted present on its
/// own page first, so a needle that has stopped matching anything fails as the
/// inert pin it has become instead of passing on an absence that means
/// nothing.
#[test]
fn the_bigquery_section_is_scoped_to_bigquery() {
    let readme = Doc::user_docs();
    let section = unwrapped(&readme.section_body(BIGQUERY_SECTION));

    for needle in [
        "ls -d -n 500",
        "ls -n 500 <dataset>",
        "show <project>:<dataset>.<table>",
    ] {
        assert!(
            section.contains(needle),
            "the BigQuery section documents {needle:?}"
        );
    }

    // The neighbours' own subcommands. Present on their own pages --
    // otherwise the absence check below pins nothing -- and absent from
    // BigQuery's, which shells to `bq` and issues neither.
    const NEIGHBOURS: [(&str, &str); 2] = [
        (SECRETS_PAGE, "secrets versions list"),
        (IAM_SECTION, "projects get-iam-policy"),
    ];
    for (page, needle) in NEIGHBOURS {
        readme.presence(page, &[needle]);
    }
    for (_, needle) in NEIGHBOURS {
        assert!(
            !section.contains(needle),
            "the BigQuery section has swallowed a neighbour's prose: {needle:?} is a \
             command a neighbouring service issues and BigQuery does not, so the heading \
             is in the wrong place or the section runs past its prose"
        );
    }
}

/// Every `-n` the BigQuery prose spells out, in **both** documents, compared
/// against the cap the listings really request.
///
/// The number appears three times in the README's section — twice inside a
/// `bq ls` block an operator can paste, and once in the fan-out paragraph as
/// the only thing bounding how many `bq show` calls a dataset can cost — and
/// once more in ARCHITECTURE.md's BigQuery section, in the same idiom
/// (`the listing's own \`-n 500\` cap`) and about the same fan-out. Retuning
/// `RESOURCE_LIST_LIMIT` with only the README pinned reds one document and
/// leaves the other silently false, which is the failure this pin exists to
/// prevent rather than to relocate. So both sections go through the same
/// parse.
///
/// A pin asserting that `"500"` appears somewhere would still pass if the
/// constant moved to 400 and every sentence stayed at 500, which is precisely
/// the drift being caught. So the numbers are taken out of the prose and
/// compared with `RESOURCE_LIST_LIMIT`, and *every* occurrence is compared
/// rather than the first: an edit that updates one block and forgets the other
/// leaves the pasteable command lying about the cap.
///
/// What "every occurrence is compared" does **not** buy is a count. No
/// expected number of `-n` sites is asserted, so deleting one of the README's
/// three leaves the surviving two agreeing with the constant and this test
/// green. ARCHITECTURE.md is the accidental exception rather than a stronger
/// pin: it states the cap exactly once, so deleting that one empties
/// `documented` for it and the "documents the `-n` cap its listings request"
/// assertion below reds. Do not read that as coverage the pin has — one more
/// `-n` sentence in that section and it is gone. That is a deliberate trade:
/// pinning the count
/// would fail on every honest rewrite that merges two command blocks or moves
/// a sentence between documents, and deletion of a whole document's prose is
/// what the `presence` checks are for. Read this pin as "no documented cap
/// disagrees with the code", not as "the caps are all still written down".
///
/// The README's fan-out paragraph is then checked to still carry a cap at all.
/// It is the sentence that turns the cap into a cost bound, and unlike the
/// command blocks it can lose the number to a rewrite without losing its
/// meaning to a careless reader — which is the shape the earlier BigQuery
/// reviews kept finding.
#[test]
fn the_docs_document_the_real_bigquery_listing_cap() {
    use nuvem::services::resources::list_filter::RESOURCE_LIST_LIMIT;

    for (doc, heading) in [
        (Doc::user_docs(), BIGQUERY_SECTION),
        (Doc::architecture(), BIGQUERY_SECTION_ARCHITECTURE),
    ] {
        let name = doc.name;
        // Pinned parsing: the documented caps are extracted and compared with
        // the constant, so this needs the section's raw text.
        let section = unwrapped(&doc.section_for_pinned_parsing(heading));

        // Every `-n <number>`. `No `-n` on that one` carries no number and so
        // is not a documented cap; it is skipped rather than failing to parse.
        let mut documented: Vec<usize> = Vec::new();
        let mut rest = section.as_str();
        while let Some(at) = rest.find("-n ") {
            let after = &rest[at + "-n ".len()..];
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() {
                documented.push(
                    digits
                        .parse()
                        .unwrap_or_else(|_| panic!("the documented cap {digits:?} is a number")),
                );
            }
            rest = after;
        }

        assert!(
            !documented.is_empty(),
            "{name}: the BigQuery section documents the `-n` cap its listings request"
        );
        for (index, cap) in documented.iter().enumerate() {
            assert_eq!(
                *cap,
                RESOURCE_LIST_LIMIT,
                "{name}: the BigQuery section's `-n` #{} of {} documents a listing cap \
                 the code does not request",
                index + 1,
                documented.len()
            );
        }
    }

    let readme = Doc::user_docs();
    let section = unwrapped(&readme.section_for_pinned_parsing(BIGQUERY_SECTION));

    // The fan-out cost paragraph: from the sentence that counts the calls to
    // the one that starts talking about cancellation instead.
    let opening = "Opening a dataset therefore issues";
    let start = section
        .find(opening)
        .unwrap_or_else(|| panic!("the BigQuery section says {opening:?}"));
    let cost = &section[start..];
    let cost = match cost.find("Only starting another listing") {
        Some(end) => &cost[..end],
        None => cost,
    };
    assert!(
        cost.contains(&format!("`-n {RESOURCE_LIST_LIMIT}` cap")),
        "the fan-out paragraph no longer names the cap that bounds the calls"
    );
}

/// The one-call-per-row identity, as the README's own arithmetic.
///
/// "a dataset holding 200 tables is 200 calls" is not a constant to compare
/// against; it is the relationship the fan-out has — one `bq show` per listed
/// row. The two numbers are therefore checked against *each other*, because
/// the edit that makes this sentence a lie is the one that changes a single
/// side of it: "300 tables is 200 calls" reads plausibly and would satisfy any
/// pin looking for `"200"` somewhere in the section.
///
/// The wording is pinned along the way. Reaching the second number means
/// walking through `tables is`, so a rewrite that keeps both numbers but drops
/// the unit fails here rather than passing on arithmetic about nothing.
#[test]
fn docs_count_one_bigquery_show_per_listed_table() {
    let readme = Doc::user_docs();
    // Pinned parsing: both numbers are extracted from the sentence and
    // compared with each other.
    let section = unwrapped(&readme.section_for_pinned_parsing(BIGQUERY_SECTION));

    let leading_number = |text: &str, what: &str| -> (usize, usize) {
        let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
        let value = digits
            .parse()
            .unwrap_or_else(|_| panic!("the {what} count {digits:?} is a number"));
        (value, digits.len())
    };

    let opening = "a dataset holding ";
    let at = section
        .find(opening)
        .unwrap_or_else(|| panic!("the BigQuery section costs the fan-out as {opening:?}…"));
    let body = &section[at + opening.len()..];
    let (tables, width) = leading_number(body, "table");

    let middle = " tables is ";
    let body = &body[width..];
    assert!(
        body.starts_with(middle),
        "the fan-out cost sentence must read `a dataset holding N{middle}M calls`, \
         not {:?}",
        &body[..body.len().min(40)]
    );
    let body = &body[middle.len()..];
    let (calls, width) = leading_number(body, "call");
    assert!(
        body[width..].starts_with(" calls"),
        "the fan-out cost sentence must count calls, not {:?}",
        &body[width..body.len().min(width + 40)]
    );

    assert_eq!(
        tables, calls,
        "the fan-out issues one `bq show` per listed table, so the README's own \
         example must count the same number of calls as it lists tables"
    );
}

/// The three column claims the README makes about BigQuery's three levels,
/// each compared with the catalog that builds the header row.
///
/// A wrong column count was one of the five defects that motivated the pinning
/// slice, and it was the one nothing caught: the earlier work retired the
/// *string* `tables view has three columns` with an `absent` needle, which
/// blocks that one sentence coming back and nothing else. An edit to "seven
/// columns" passed every pin in the file. `absent` can only ever hold out a
/// wording someone already wrote down; a claim is bound to the code by being
/// read out of the prose and compared, which is what this does.
///
/// Two shapes of claim are made and both are checked:
///
/// * A **count**, spelled as a word — "six columns", "four columns". The word
///   is compared against the catalog's length, so adding a column to
///   `BIGQUERY_TABLE_COLUMNS` without touching the sentence reds here.
/// * A **list of headers**, in display order — DATASET/LOCATION/LABELS for the
///   datasets list, FIELD/TYPE/MODE/DESCRIPTION for the schema view. The
///   headers are compared against `spec.header`, in order, so renaming BYTES
///   or reordering two columns reds too.
///
/// The tables view names no header list — its six headers are discussed one
/// group at a time across the paragraph rather than enumerated — so only its
/// count is pinned. That is the weaker of the two bindings and it is the
/// honest one to have: inventing a list to pin would put a sentence in the
/// README to serve the test.
///
/// Every claim is required to be present, unlike the `-n` and fan-out-width
/// pins above. There is exactly one site per level here, so "compare whatever
/// you find" would go green on a section that had lost the sentence entirely —
/// and a lost count is how the original defect got in.
#[test]
fn docs_document_the_real_bigquery_column_counts() {
    use nuvem::app::projection::columns::{
        BIGQUERY_DATASET_COLUMNS, BIGQUERY_SCHEMA_COLUMNS, BIGQUERY_TABLE_COLUMNS,
    };

    /// The number words the prose could plausibly be spelling, in the order a
    /// `usize` indexes them: `NUMBER_WORDS[4]` is `"four"`. Same table as the
    /// fan-out-width pin in `src/services/runtime/session.rs`, for the same
    /// reason: the prose spells small numbers out.
    const NUMBER_WORDS: [&str; 11] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ];

    let readme = Doc::user_docs();
    // Pinned parsing: the counts and header names are taken out of the prose
    // and compared with the catalogs, so this needs the section's raw text.
    let section = unwrapped(&readme.section_for_pinned_parsing(BIGQUERY_SECTION));

    let spelled = |count: usize| -> &'static str {
        NUMBER_WORDS.get(count).copied().unwrap_or_else(|| {
            panic!(
                "the README spells its column counts as words; teach this test how to \
                 spell {count} before giving a BigQuery level that many columns"
            )
        })
    };

    for (level, claim, catalog_len) in [
        (
            "tables",
            "The tables view has ",
            BIGQUERY_TABLE_COLUMNS.len(),
        ),
        (
            "schema",
            "The schema view has ",
            BIGQUERY_SCHEMA_COLUMNS.len(),
        ),
    ] {
        let at = section.find(claim).unwrap_or_else(|| {
            panic!(
                "the README no longer says how many columns the BigQuery {level} view has: \
                 {claim:?} is gone, and a level whose column count is written down nowhere \
                 is the state the wrong count shipped from"
            )
        });
        let rest = &section[at + claim.len()..];
        let word: String = rest
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .collect::<String>()
            .to_ascii_lowercase();
        assert!(
            rest[word.len()..].starts_with(" columns"),
            "the README's {level}-view claim must read `{claim}<word> columns`, not {:?}. \
             A digit is not a count this test can read: spell it out, or teach the test to \
             read digits",
            &rest[..rest.len().min(40)]
        );
        assert_eq!(
            word,
            spelled(catalog_len),
            "the README says the BigQuery {level} view has {word} columns; its catalog \
             builds {catalog_len}"
        );
    }

    for (level, claim, headers) in [
        (
            "datasets",
            "The table columns are ",
            BIGQUERY_DATASET_COLUMNS
                .iter()
                .map(|column| column.spec.header)
                .collect::<Vec<_>>(),
        ),
        (
            "schema",
            "The schema view has four columns: ",
            BIGQUERY_SCHEMA_COLUMNS
                .iter()
                .map(|column| column.spec.header)
                .collect::<Vec<_>>(),
        ),
    ] {
        let at = section.find(claim).unwrap_or_else(|| {
            panic!(
                "the README no longer lists the BigQuery {level} view's columns: {claim:?} \
                 is gone"
            )
        });
        let rest = &section[at + claim.len()..];
        let sentence = &rest[..rest.find('.').unwrap_or(rest.len())];
        // `A, B and C` — the list as the prose writes it, which is also the
        // display order the header row paints.
        let documented: Vec<&str> = sentence
            .split(&[',', '.'][..])
            .flat_map(|part| part.split(" and "))
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        assert_eq!(
            documented, headers,
            "the README lists the BigQuery {level} view's columns as {documented:?}; its \
             catalog paints {headers:?}, in that order"
        );
    }
}

/// The VPC networks argv, read out of the README's own fenced block and
/// compared with the argv the service builds.
///
/// A prose pin would let the two drift the first time a flag is added: the
/// block is a command a reader may paste, so it is parsed rather than
/// searched for. `--limit` is checked against `RESOURCE_LIST_LIMIT` rather
/// than against the literal `500`, so retuning the cap fails here instead of
/// leaving the document quietly wrong.
#[test]
fn docs_publish_the_vpc_networks_argv_the_service_builds() {
    use nuvem::services::resources::list_filter::RESOURCE_LIST_LIMIT;

    let readme = Doc::user_docs();
    // Pinned parsing: the fenced block's own words are compared with the argv.
    let section = readme.section_for_pinned_parsing(VPC_SECTION);
    let block = section
        .split("```bash")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .unwrap_or_else(|| panic!("the VPC section publishes a fenced command"));
    let published: Vec<&str> = block
        .split_whitespace()
        .filter(|word| *word != "\\" && *word != "gcloud")
        .collect();

    assert!(
        published
            .windows(3)
            .any(|words| words == ["compute", "networks", "list"]),
        "the published command must be `compute networks list`: {published:?}"
    );
    assert!(
        published.contains(&"--format=json"),
        "the published command must ask for JSON: {published:?}"
    );
    assert!(
        published.contains(&format!("--limit={RESOURCE_LIST_LIMIT}").as_str()),
        "the published limit must be the shared row cap: {published:?}"
    );
    assert!(
        !published.iter().any(|word| word.starts_with("--filter")),
        "nothing is pushed down for this table, so the command carries no \
         --filter: {published:?}"
    );
}

/// The two fields the parser must never read, named in the document.
///
/// This is the one claim in the VPC section that a later author is most likely
/// to undo by accident: `x_gcloud_subnet_mode` carries the exact string the
/// MODE column paints, so reading it looks like a simplification. The parser
/// side is pinned by
/// `vpc_service_test::derives_mode_and_routing_from_the_api_fields_not_gclouds_synthesised_ones`;
/// this keeps the *reason* from being deleted from the document that explains
/// it.
///
/// PRESENCE-ONLY: nothing in the code reports "the README explains the
/// x_gcloud hazard". The behaviour is held by the parser test; this holds the
/// explanation.
#[test]
fn docs_name_the_two_gcloud_fields_the_vpc_parser_must_not_read() {
    let readme = Doc::user_docs();
    readme.presence(
        VPC_SECTION,
        &[
            "`x_gcloud_subnet_mode`",
            "`x_gcloud_bgp_routing_mode`",
            "gcloud's presentation layer",
        ],
    );
}

/// The VPC fixture-provenance disclosure, held in place by `presence`.
///
/// The same check `readme_keeps_the_bigquery_fixture_provenance_disclosure`
/// makes, for the same reason: a transcription taken for a capture
/// is a claim about evidence, and the failure mode that actually happened here
/// before was a *deleted* sentence rather than a wrong one. The networks
/// fixture is a capture and three of the table's cases are not in it, so the
/// document has to say which is which.
#[test]
fn docs_keep_the_vpc_fixture_provenance_disclosure() {
    let readme = Doc::user_docs();
    readme.presence(
        VPC_SECTION,
        &[
            "is a live capture taken on 2026-08-25",
            "constructed from Google's published `compute/v1/networks` reference",
        ],
    );
}

/// The firewall rules argv, read out of the README's own fenced block.
///
/// The networks pin's twin, and it exists separately because a subsequence
/// route is what serves this command in demo mode: `compute firewall-rules
/// list` and `compute networks list` are two shapes one match arm can confuse,
/// and the README is where a reader learns which one nuvem actually runs.
/// `--limit` is checked against `RESOURCE_LIST_LIMIT` rather than the literal,
/// so retuning the cap fails here instead of leaving the document wrong.
#[test]
fn docs_publish_the_firewall_rules_argv_the_service_builds() {
    use nuvem::services::resources::list_filter::RESOURCE_LIST_LIMIT;

    let readme = Doc::user_docs();
    // Pinned parsing: the fenced block's own words are compared with the argv.
    let section = readme.section_for_pinned_parsing(FIREWALL_SECTION);
    let block = section
        .split("```bash")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .unwrap_or_else(|| panic!("the firewall section publishes a fenced command"));
    let published: Vec<&str> = block
        .split_whitespace()
        .filter(|word| *word != "\\" && *word != "gcloud")
        .collect();

    assert!(
        published
            .windows(3)
            .any(|words| words == ["compute", "firewall-rules", "list"]),
        "the published command must be `compute firewall-rules list`: {published:?}"
    );
    assert!(
        published.contains(&"--format=json"),
        "the published command must ask for JSON: {published:?}"
    );
    assert!(
        published.contains(&format!("--limit={RESOURCE_LIST_LIMIT}").as_str()),
        "the published limit must be the shared row cap: {published:?}"
    );
    assert!(
        !published.iter().any(|word| word.starts_with("--filter")),
        "nothing is pushed down for this table, so the command carries no \
         --filter: {published:?}"
    );
    assert!(
        !published.iter().any(|word| word.starts_with("--network")),
        "a firewall rule is global and names its network in a column; a \
         per-network flag would be a fan-out this service does not do: \
         {published:?}"
    );
}

/// The firewall fixture's *mixed* provenance, held in place by `presence`.
///
/// This is the strongest case in the repository for the transcription rule, because
/// this fixture is the only one that is part capture and part transcription.
/// Every arm of this parser a reader would call interesting — EGRESS, `denied`,
/// target tags, service-account scoping, `disabled` — rests on the published
/// reference rather than on bytes gcloud produced, and two of those are the
/// arms whose misrendering is dangerous. A document that lost the sentence
/// saying so would leave the whole table looking captured.
///
/// PRESENCE-ONLY: nothing in the code reports "the README discloses this". The
/// parsing is held by `vpc_firewall_service_test`; this holds the disclosure,
/// and deletion is the failure mode that actually happened before.
#[test]
fn docs_keep_the_firewall_fixture_mixed_provenance_disclosure() {
    let readme = Doc::user_docs();
    readme.presence(
        FIREWALL_SECTION,
        &[
            "are a live capture taken on 2026-08-25",
            "constructed from Google's published `compute/v1/firewalls` reference",
            "rests on transcription rather than on bytes gcloud produced",
        ],
    );
}

/// The two firewall cells that have no field behind them, explained in prose.
///
/// Both are derivations a later author could delete while the table still
/// looks right: that `ACTION` comes from *which array the API sent* rather
/// than from an action field, and that an empty target list is the widest
/// scope rather than an absent value.
///
/// PRESENCE-ONLY: the behaviours are pinned in `vpc_firewall_service_test` and
/// `vpc_firewall_rendering_test`; this keeps the reasons in the document that
/// explains them.
#[test]
fn docs_explain_the_two_firewall_cells_that_have_no_field_behind_them() {
    let readme = Doc::user_docs();
    readme.presence(
        FIREWALL_SECTION,
        &[
            "It reports an `allowed` array or a `denied` array",
            "the widest scope there is",
            "`all instances`",
        ],
    );
}

/// The fixture-provenance disclosure, held in place by `presence`.
///
/// One of the five defects the pinning slice answered was a *removed*
/// sentence: `src/services/runtime/demo/mod.rs` briefly claimed a transcribed
/// fixture was a fresh capture, after the honest wording had been deleted. The
/// slice added `absent` needles and value comparisons and no `presence` pin at
/// all, so deletion — the failure mode that actually happened — stayed the one
/// shape nothing watched.
///
/// `presence` is the check for it, and this is the claim most worth holding:
/// it is the sentence that stops a reader taking the demo numbers for measured
/// ones. The needles are the load-bearing halves of the paragraph rather than
/// whole sentences, so an honest rewrite that keeps the disclosure survives
/// and a deletion does not.
#[test]
fn docs_keep_the_bigquery_fixture_provenance_disclosure() {
    let readme = Doc::user_docs();
    readme.presence(
        BIGQUERY_SECTION,
        &[
            "that response was transcribed rather than freshly captured",
            "its counters carry the shape the API sends and not verified values",
        ],
    );
}

// The third number the BigQuery prose names -- the enrichment fan-out's "four
// at a time", in both README.md and ARCHITECTURE.md -- is pinned in
// `src/services/runtime/session.rs`, as
// `tests::the_docs_document_the_real_bigquery_enrichment_concurrency`. It is
// not here because `BIGQUERY_ENRICHMENT_CONCURRENCY` is private to that
// module, and this file is a separate crate: `pub(crate)` would not reach it
// and `pub` would make an internal tuning number public API to satisfy a
// documentation test. The pin reads the README from inside the module
// instead, so the width is bound to the semaphore rather than to itself.

/// The four BigQuery claims this ticket had to retire, held out of the
/// document.
///
/// All four were true when they were written and false by the time they were
/// read: the slices that landed the tables drill-down, the per-row enrichment
/// and the schema view each falsified a sentence an earlier slice had left
/// behind. The `out of scope here` deferral is the same shape and not an
/// exception to it — `git log -S"out of scope here" -- README.md` puts the
/// clause in `6153219` (PR #176, the tables drill-down) and takes it out again
/// in `e8ab32e` (PR #177, the per-table enrichment). #176 shipped the
/// drill-down without the enrichment, so the deferral was true the day it was
/// written; #177 falsified it and corrected it in the same breath. Every one
/// was caught by a human reading the branch rather than by anything here. An
/// absence check is what turns that into a gate — a
/// superseded claim cannot come back through a revert, a copy-paste out of an
/// old section, or a rebase that resurrects a paragraph.
///
/// PRESENCE-ONLY (absence form): a substring check is all that is available.
/// Nothing in the code reports "the README no longer claims the schema is
/// unbrowsable", and the pins that the schema *is* browsable live in the
/// positive checks above; this is the other half, and it buys exactly one
/// thing — the retired wording cannot reappear.
///
/// **Every needle here was checked against the whole README before it was
/// added**, because `absent` is document-wide by design and a needle another
/// service uses truthfully would be a pin that fails on a true sentence — and
/// the way such a pin gets "fixed" is by deleting the true sentence. Two of
/// the four are therefore longer than the false claim's memorable phrase:
///
/// - `"does not issue"` on its own would fire on any future sentence saying
///   some path issues no call, which the BigQuery prose already says twice in
///   the other idiom (`"issues no call"`). The needle carries `nuvem` so it
///   names the retired claim and not the idiom.
/// - `"out of scope here"` is the phrase the retired claim used of the
///   per-table `bq show`, one PR before that fan-out shipped. The README defers work
///   truthfully in two other places — Cloud Storage ACL/IAM and Secret Manager
///   IAM — in the neighbouring wording (`"out of scope for this slice"`,
///   `"out of scope for this service"`), so the bare phrase is not in the
///   document today but is one rewrite away from being true somewhere else.
///   The needle takes the whole clause instead.
/// - `"not browsable"` is deliberately the short form: the word `browsable`
///   appears nowhere in the README now, and this claim was made in three
///   different wordings across the slices — `is not browsable`,
///   `are not browsable yet` and `is still not browsable` — which only a
///   needle this short catches all of. If some other service ever needs the phrase
///   truthfully, narrow this needle to the BigQuery wording; do not delete the
///   true sentence.
/// - `"three columns"` is scoped by its subject rather than pinned bare: the
///   README describes other tables by their column count in exactly this
///   idiom (`"six columns"`, `"four columns"`), so a bare count would be a
///   phrase another section could truthfully write. Holding out the retired
///   string is all an `absent` needle can do; the counts themselves are bound
///   to the catalogs by
///   [`readme_documents_the_real_bigquery_column_counts`], which is what makes
///   an edit to "seven columns" fail rather than pass.
#[test]
fn docs_hold_out_the_retired_bigquery_claims() {
    let readme = Doc::user_docs();

    readme.absent(&[
        // Reading a table's schema "needs a per-table `bq show`, which nuvem
        // does not issue". It issues one per listed table for the row stats,
        // and one more for the schema view.
        "`bq show`, which nuvem does not issue",
        // The row stats "come from a per-table `bq show`, one call per row,
        // which is out of scope here" — true in the drill-down slice, and
        // falsified by the enrichment slice that followed it.
        "one call per row, which is out of scope here",
        // The schema, in every wording the three slices gave it. It is a
        // drill-down of its own now.
        "not browsable",
        // The tables view had three columns before the enrichment slice added
        // ROWS, BYTES and LAST MODIFIED.
        "tables view has three columns",
    ]);
}

/// The IAM policy table's contract, as the README states it.
///
/// Three claims an operator acts on and cannot check from the screen. The
/// (role, member) flattening is why a member appears on several rows and a
/// role on several more, so a reader who assumes one row per binding
/// miscounts. The two filter directions are what make one table answer both
/// audit questions. And the role being verbatim is what keeps
/// `projects/p/roles/editor` distinguishable from the `roles/editor` it
/// shadows — an abbreviation would silently merge a custom role with the
/// predefined one, which is exactly the confusion an audit is run to resolve.
///
/// The badge paragraph is pinned rather than presence-checked: the words are
/// built from `PUBLIC_BADGE` and `DELETED_BADGE`, so renaming a badge in the
/// code without renaming it here fails, and a reader who types `/PUBLIC` after
/// reading this is typing a string the cell really carries.
///
/// PRESENCE-ONLY beyond that: these are prose claims, and a presence check
/// buys deletion detection and nothing more. It is worth having anyway — the
/// paragraph is the only place the flattening is written down.
#[test]
fn docs_document_the_iam_policy_table_contract() {
    use nuvem::app::state::resources::{DELETED_BADGE, PUBLIC_BADGE};

    let readme = Doc::user_docs();
    readme.presence(
        IAM_SECTION,
        &[
            "one row per (role, member) pair".to_string(),
            "filter by a member to see what that principal can do".to_string(),
            "filter by a role to see who holds it".to_string(),
            "The role is shown verbatim, exactly as the policy spells it".to_string(),
            format!("`{PUBLIC_BADGE}`: the grant is held by `allUsers`"),
            format!("`{DELETED_BADGE}`: the principal was deleted"),
            format!("`/{PUBLIC_BADGE}` isolates the badged rows"),
            // List-only, and the sentence that says so is the one an operator
            // reads before looking for an edit key that does not exist.
            "nuvem never issues `set-iam-policy`".to_string(),
        ],
    );
}

/// The declared type of `field` in the shape following `marker`, read to the
/// comma that ends the field rather than the first comma inside it — a
/// `Vec<(BatchRowRef, u64)>` contains one, and stopping there would compare
/// half a type against half a type and pass.
fn field_type(source: &str, marker: &str, field: &str) -> String {
    let at = source
        .find(marker)
        .unwrap_or_else(|| panic!("{marker:?} is present"));
    let body = &source[at + marker.len()..];
    let opening = format!("{field}:");
    let start = body
        .find(&opening)
        .unwrap_or_else(|| panic!("{marker:?} declares {field}"));
    let rest = &body[start + opening.len()..];
    let mut depth = 0usize;
    let mut end = rest.len();
    for (index, character) in rest.char_indices() {
        match character {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' | '}' if depth == 0 => {
                end = index;
                break;
            }
            _ => {}
        }
    }
    rest[..end].trim().to_string()
}

/// The batch pipeline diagram spells out the shape of
/// `Effect::RunResourceBatch`, and a reader adding a batchable service builds
/// that effect from the diagram. A field type that has moved on in the code
/// turns the diagram into instructions that do not compile — which is how
/// `targets` came to read `Vec<BatchRowRef>` after each target was paired with
/// the operations-log id its result settles. Both halves are read out of their
/// source, so the diagram going stale fails, and so does deleting it.
#[test]
fn architecture_documents_the_real_run_resource_batch_shape() {
    let effect = std::fs::read_to_string("src/app/effect.rs").unwrap();
    // Pinned parsing: the documented field types are compared with the real
    // ones.
    let architecture = Doc::architecture();

    for field in ["action", "targets"] {
        assert_eq!(
            field_type(
                architecture.text_for_pinned_parsing(),
                "Effect::RunResourceBatch {",
                field
            ),
            field_type(&effect, "RunResourceBatch {", field),
            "the batch pipeline diagram documents a {field} type Effect::RunResourceBatch does not have"
        );
    }
}

// PRESENCE-ONLY throughout: "a plugin runs, and these are the things standing
// between the palette entry and the child process" is a property of every path
// a plugin can take through the app, and no single literal in the source states
// it — the evidence is the reducer arm, the keymap guard and the runtime
// dispatch, each pinned by its own test, not a string this file could read.
// What these assertions buy is deletion detection on the paragraphs an operator
// relies on before pressing `Enter` on someone else's argv, plus the `absent`
// below, which is the one half that cannot go quietly stale: a claim that is
// false has to be *rewritten*, and merely deleting it fails here.
#[test]
fn docs_document_that_a_plugin_runs_from_a_confirmation() {
    let readme = Doc::user_docs();

    readme.presence(
        "# How plugins work",
        &[
            // The palette hands over to a modal, and that modal's `Enter` is
            // the moment a process starts.
            "opens a confirmation showing the exact command",
            "`Enter` runs it and `Esc` cancels",
            // The typed acknowledgement, stated as the gate on `Enter` it is
            // rather than as decoration on the modal.
            "will not offer `Enter` until you have typed `1`",
            // The two guards an operator cannot see from the screen: an argv
            // too big for the terminal is not confirmable, and read-only runs
            // only a plugin whose declaration proves it reads.
            "cannot be confirmed, only cancelled",
            "runs a plugin only when nuvem can prove from its declaration that\n\
             it reads",
            // Where the outcome of a run goes, since the run outlives the
            // frame that confirmed it.
            "pending from the moment you confirm",
            // The promise the schema has always made, and still does.
            "Nothing in the schema can run a shell command",
            // The three refusals, each named by the thing that would go wrong.
            "A field the row has no value for",
            "A substituted value that begins with `-`",
            "A rendered value carrying a control character",
            // The absence of a length bound is a decision, not an oversight.
            "no limit on how long a rendered value may be",
            // Read once at startup: the reason `PluginId` can be an index.
            "restart required for: plugins",
            // The second way in, and the four claims an operator has
            // to be able to find before trusting a two-keystroke gesture that
            // starts a process: what arms it, what the armed prefix tells them,
            // that a press that resolves to nothing explains itself rather than
            // going quiet, and that the gesture is a shortcut into the palette's
            // path rather than a path of its own with its own guards.
            "**plugin prefix key**",
            "the status line names the keys this service declares",
            "Every way a keyed press can come to nothing says so",
            "not a second way to run a command",
        ],
    );

    // Superseded claims, each from a slice that has been overtaken. The first
    // is from the slice that only validated the file; the next two are from the
    // slice that previewed a plugin and copied it; the next two are from the
    // slices before the prefix key was bound, and are the ones that cannot go
    // quietly stale — a `key` field the README calls unbound is a field an
    // operator will not write. The last is from the slices before the read-only refusal,
    // when the mode refused plugins wholesale; re-added anywhere it would tell
    // an operator that a plugin their declaration proves reads cannot run.
    // An unscoped absence check is what stops any of them being re-added a
    // section over.
    readme.absent(&[
        "Nothing is offered in the UI yet",
        "it does not run them",
        "copies exactly that line to the clipboard",
        "it does not yet bind plugins to keys",
        "which a later slice adds",
        "refuses every plugin, with no opt-out",
    ]);
}

// PRESENCE-ONLY: "the table reloads after a plugin run" is a claim about a
// reducer arm's effects, and no symbol reports it as a fact this crate can
// import and compare. `tests/plugin_refresh_evidence.rs` is where the
// behaviour itself is pinned; this only detects the paragraph being deleted.
#[test]
fn docs_document_the_plugin_run_reload() {
    let readme = Doc::user_docs();

    readme.presence(
        "# How plugins work",
        &[
            // Both halves of the guard, since documenting one gate and not the
            // other is a claim broader than the reducer arm: the workspace has
            // to be Resources *and* the result's context has to still be the
            // active one.
            "the resource table reloads, provided you are still looking at it",
            "under the configuration you launched the run from",
            // The in-place reload, which is the whole reason this arm does not
            // reuse the lifecycle path's clear-and-mark-stale.
            "survive the reload",
            // The negative case, carrying the switched-configuration half so
            // the needle cannot be satisfied by the positive sentence above.
            "after you have switched configuration reloads nothing either",
        ],
    );
}

/// The `[columns]` catalog table is the only published copy of the per-service
/// column vocabulary, and that vocabulary is *derived* — `catalog_meta` over
/// the typed catalogs. A column added, renamed or made unsortable therefore
/// changes what a user's file may say the moment it exists, silently, while
/// the README goes on listing the old set.
///
/// This file's own docstring cites "`[columns]`' catalog" as a thing built to
/// make that drift impossible in code, and until the BigQuery slice nothing held the
/// document to it: the `[[jumps]]` field table was walked, this one was not.
/// Writing the walk found both shapes of drift it exists to catch — five
/// column keys spelled with a hyphen where the catalog uses an underscore
/// (`ack-deadline`, `dead-letter`, `kms-key`, `display-name`, `unique-id`,
/// none of which `lookup` would have accepted from a real config file) and a
/// missing row for Persistent disks.
///
/// Each row is pinned three ways: the `service` id is resolved by the real
/// parser, so the id the README publishes is one a user's file may really
/// use; the row's own column list is the block that parser validates, so a
/// documented key the catalog does not have is rejected as `unknown column`;
/// and the keys are then compared with `catalog_meta` in order and in both
/// directions, with the `*` marker compared against `sortable`.
#[test]
fn docs_list_every_configurable_column_of_every_service() {
    use nuvem::config::columns::ColumnSettings;
    use std::collections::BTreeMap;

    const COLUMNS_AND_SORT: &str = "# Columns and sort";

    // Pinned parsing: every row's keys are compared with `catalog_meta`.
    let readme = Doc::user_docs();
    let section = readme.section_for_pinned_parsing(COLUMNS_AND_SORT);

    // Matched by the header's columns rather than by a padded literal, so
    // reflowing the table's whitespace does not report itself as a deletion —
    // the same shape as the other table walks in this file.
    let mut lines = section
        .lines()
        .skip_while(|line| !(line.starts_with("| service") && line.contains("| columns")));
    lines
        .next()
        .expect("README documents the configurable columns as a table");
    let rows: Vec<&str> = lines
        .skip(1)
        .take_while(|line| line.starts_with('|'))
        .collect();

    let mut documented_kinds: Vec<ResourceServiceKind> = Vec::new();
    for row in &rows {
        let mut cells = row.split('|').skip(1);
        let id = cells
            .next()
            .and_then(|cell| cell.split('`').nth(1))
            .unwrap_or_else(|| panic!("column-table row {row:?} backticks a service id"));
        let columns_cell = cells
            .next()
            .unwrap_or_else(|| panic!("column-table row {row:?} has a columns column"));

        // Each documented key, with whether the table marks it unsortable.
        // A `*` is only a marker where it directly follows the closing
        // backtick, so a `*` anywhere else in the cell is not read as one.
        let documented: Vec<(&str, bool)> = columns_cell
            .match_indices('`')
            .skip(1)
            .step_by(2)
            .map(|(close, _)| {
                let open = columns_cell[..close]
                    .rfind('`')
                    .expect("a closing backtick has an opening one");
                (
                    &columns_cell[open + 1..close],
                    columns_cell[close + 1..].starts_with('*'),
                )
            })
            .collect();
        assert!(
            !documented.is_empty(),
            "column-table row {row:?} lists no columns"
        );

        // The row's own list, run through the real parser: this resolves the
        // service id *and* validates every key against that service's
        // catalog, so a key the catalog does not have fails here before the
        // comparison below ever runs.
        let keys = documented
            .iter()
            .map(|(key, _)| format!("{key:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let block: toml::Value = toml::from_str(&format!("columns = [{keys}]\n")).unwrap();
        let mut raw = BTreeMap::new();
        raw.insert(id.to_string(), block);
        let mut problems = ConfigProblems::default();
        let settings = ColumnSettings::from_file(&raw, &mut problems, &ColumnSettings::empty());
        assert!(
            problems.is_empty(),
            "the README publishes `[columns.{id}] columns = [{keys}]` as a block a user \
             may write: {:?}",
            problems.messages()
        );
        let kind = *ResourceServiceKind::ALL
            .iter()
            .find(|kind| settings.get(**kind).is_some())
            .unwrap_or_else(|| panic!("`[columns.{id}]` configured no service"));
        documented_kinds.push(kind);

        // Both directions, in catalog order: `Vec` rather than a set, because
        // a column listed twice is a defect in the table rather than
        // something to collapse, and ordered because the table is read as the
        // left-to-right order the columns appear in.
        let real: Vec<(&str, bool)> = catalog_meta(kind)
            .iter()
            .map(|column| (column.key, !column.sortable))
            .collect();
        assert_eq!(
            documented, real,
            "the column table claims {id:?} offers {documented:?} (key, marked unsortable)"
        );
    }

    // Both directions on the rows themselves: every configurable service is
    // listed, and no service is listed twice under two of its ids.
    assert_eq!(
        documented_kinds.len(),
        ResourceServiceKind::ALL.len(),
        "the column table has one row per service, no more and no fewer"
    );
    for kind in ResourceServiceKind::ALL {
        assert!(
            documented_kinds.contains(&kind),
            "the column table has no row for {kind:?}"
        );
    }
}

/// The section the REST decision lives in. It is unheaded prose beside
/// the `CommandProgram` discussion rather than a section of its own, so the
/// nearest enclosing heading is the scope available; nothing else in this file
/// is pinned to it, so widening it costs no other assertion.
const CONFIG_SECTION_ARCHITECTURE: &str = "### `src/config/`";

/// The REST decision states two facts that live in this repository rather
/// than in the Cloud SDK, and both are the kind that gets retuned without
/// anyone rereading the paragraph that quotes them.
///
/// The poll interval is the argument's whole cost basis: the prose says a token
/// acquired per poll would spend a third of the default before the HTTP call
/// began, and that ratio is a claim about *this* number. Halve the default and
/// the sentence becomes false while every word of it survives.
///
/// The `deny.toml` ban is what turns "`syn` 3 alongside `syn` 2" from an
/// observation into a refusal. Relax it to `warn` and the paragraph reports a
/// gate that no longer exists — which is the failure mode the pins exist for,
/// since a reader checking the decision would find a claim about tooling and no
/// reason to doubt it.
///
/// Note what neither pin buys. Every other number in that passage — the SDK
/// surface listing, the crate counts, the 1.5–1.8s startup, the five licenses —
/// is a fact about software outside this repository, and no assertion here can
/// keep those true. They are dated to SDK 578.0.0 in the prose for that reason.
#[test]
fn architecture_rest_decision_pins_the_repo_numbers_it_argues_from() {
    use nuvem::Config;

    let architecture = Doc::architecture();
    let section = architecture.section_for_pinned_parsing(CONFIG_SECTION_ARCHITECTURE);
    let body = unwrapped(&section);

    // The interval is extracted from the prose and compared, rather than the
    // prose being searched for a literal `5`: a `contains("5s")` would be
    // satisfied by any other number containing a five, and by the sentence
    // being deleted down to some unrelated mention.
    let quoted = body
        .split_once("s `poll_interval_secs` default")
        .map(|(before, _)| before)
        .and_then(|before| {
            let digits: String = before
                .chars()
                .rev()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.chars().rev().collect::<String>().parse::<u64>().ok()
        })
        .expect(
            "the REST decision states the poll interval as `<n>s `poll_interval_secs` default`",
        );
    assert_eq!(
        quoted,
        Config::default().poll_interval_secs,
        "the REST decision's cost argument quotes the poll interval default"
    );

    // The ban is quoted verbatim in the prose, so the pin is that the quoted
    // setting is the one `deny.toml` actually carries.
    let quoted_ban = "`bans.multiple-versions = \"deny\"`";
    assert!(
        body.contains(quoted_ban),
        "the REST decision cites the multiple-versions ban as its `syn` 3 refusal"
    );
    let deny = std::fs::read_to_string("deny.toml").unwrap();
    assert!(
        unwrapped(&deny).contains("[bans] multiple-versions = \"deny\""),
        "deny.toml still bans multiple versions, as the REST decision claims"
    );
}

/// The backend the user-facing documents describe must be the backend the
/// code has. Every native client module under `src/services/sdk/` maps to a
/// service the README names as native, and no contributor-facing document
/// still claims the CLI is the only path to Google APIs.
#[test]
fn the_documents_describe_the_backend_the_code_has() {
    let readme = std::fs::read_to_string("README.md").expect("README.md is readable");
    assert!(
        readme.contains("official Google Cloud client libraries"),
        "README names the native read backend"
    );
    assert!(
        readme.contains("config-helper"),
        "README says where the native path's token comes from"
    );

    let modules: Vec<String> = std::fs::read_dir("src/services/sdk")
        .expect("the native client modules are listed")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".rs") && name != "mod.rs")
        .collect();
    assert!(
        modules.len() >= 10,
        "the native module scan found {} modules; it is looking in the wrong place",
        modules.len()
    );

    for doc in [
        "README.md",
        "SECURITY.md",
        "CONTRIBUTING.md",
        ".github/ISSUE_TEMPLATE/feature_request.md",
    ] {
        let text = std::fs::read_to_string(doc).unwrap_or_else(|error| panic!("{doc}: {error}"));
        for stale in [
            "never talks to Google APIs directly",
            "never talks to Google Cloud APIs directly",
            "only ever shells out",
            "Every operation shells out",
        ] {
            assert!(
                !text.contains(stale),
                "{doc} still says {stale:?}, which stopped being true when reads moved to the client libraries"
            );
        }
    }
}

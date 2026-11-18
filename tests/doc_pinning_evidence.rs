//! Evidence — the converted documentation pins move with the prose.
//!
//! The defect this ticket paid off is that `tests/documentation_test.rs`
//! asserted README/ARCHITECTURE claims with bare `contains("substring")`. Those
//! substrings survive while the sentence around them turns false: once
//! a false README sentence sat *between* two asserted substrings and
//! the suite stayed green, certifying it through a full review round. Tasks 1-3
//! converted the machine-derivable claims into assertions that either derive
//! their expected value from the code or extract the claim out of the document
//! and compare it against the code.
//!
//! What that buys is one property, and it is the only thing this binary tries
//! to record: **the assertion moves with the prose**. For a representative pin
//! from each of tasks 1, 2 and 3 this test takes the real document text,
//! applies in memory the exact edit a reviewer reproduced as the historical
//! defect, and records that the derivation now rejects it — together with an
//! unmutated baseline, because a check that rejects a mutation but also
//! rejects the real document would prove nothing.
//!
//! HONESTY ABOUT WHAT THIS IS. The shipped derivations live inside `#[test]`
//! functions in `tests/documentation_test.rs`; they read the document off disk
//! themselves and report through `assert!`/`assert_eq!`, so they cannot be
//! handed a mutated string and observed. Restructuring them to be callable
//! would invalidate a finished review, so the three derivations below are
//! **re-implementations** of them, transcribed into a form that returns a
//! verdict instead of panicking. They therefore demonstrate that the *property*
//! holds for a derivation of this shape; they are not a proof of the shipped
//! assertion itself, and a future edit that weakens the shipped derivation
//! without touching this file would not be caught here. Read this as evidence
//! about the technique, not as a second copy of the guarantee.
//!
//! Nothing here writes to README.md or ARCHITECTURE.md. The documents are read
//! once and every mutation is applied to an owned `String`. Every assertion
//! runs unconditionally; when `NUVEM_EVIDENCE_DIR` is set a transcript of the six
//! verdicts is also written, and nothing at all is written when it is unset.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use nuvem::app::state::resources::{
    BatchAck, BatchAction, BatchActionConfirmation, CloudSql, CloudSqlInstanceAction, Compute,
    ComputeInstanceAction, ResourcesState,
};
use nuvem::config::jumps::JumpRules;
use nuvem::ui::resources::resource_batch_confirmation_lines;
use nuvem_domain::gcp::configuration::{GcloudConfigurationName, GcloudContext};
use nuvem_domain::gcp::filter::label_filter_field;
use nuvem_domain::gcp::resource::{
    CloudSqlAvailability, CloudSqlInstance, CloudSqlStatus, ComputeInstance, EndpointVisibility,
    ResourceServiceKind, ResourceStatus,
};

// ---------------------------------------------------------------------------
// evidence writing
// ---------------------------------------------------------------------------

/// The evidence directory, or `None` when `NUVEM_EVIDENCE_DIR` is unset.
///
/// There is deliberately no fallback location: with the variable unset this
/// test writes nothing, so it cannot leave files behind on a developer's
/// machine or collide with a concurrent run.
fn evidence_dir() -> Option<PathBuf> {
    let dir = std::env::var("NUVEM_EVIDENCE_DIR").ok()?;
    let path = PathBuf::from(dir);
    fs::create_dir_all(&path).ok()?;
    Some(path)
}

// ---------------------------------------------------------------------------
// document helpers — transcribed from `tests/documentation_test.rs`
// ---------------------------------------------------------------------------

const BATCH_SECTION: &str = "## Batch actions on marked rows";

/// The section a claim lives in, from its `##` heading to the next one.
fn document_section(document: &str, heading: &str) -> Result<String, String> {
    let start = document
        .find(heading)
        .ok_or_else(|| format!("the documentation has no {heading:?} section"))?;
    let body = &document[start + heading.len()..];
    let end = body.find("\n## ").unwrap_or(body.len());
    Ok(body[..end].to_string())
}

/// Reflowed to one line, so a claim that wraps across a line break is still one
/// string to match.
fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Replaces `from` with `to` exactly once, and refuses to return a document
/// that is unchanged.
///
/// A mutation that silently failed to apply would produce a "rejected" line
/// this test could not earn — the derivation would be running against the real
/// document. So a no-op edit is a hard failure here, not a skip.
fn mutate(document: &str, from: &str, to: &str) -> String {
    assert_eq!(
        document.matches(from).count(),
        1,
        "the text to mutate must occur exactly once: {from:?}"
    );
    let mutated = document.replacen(from, to, 1);
    assert_ne!(mutated, document, "the mutation must change the document");
    mutated
}

// ---------------------------------------------------------------------------
// task 1 — the label-pushdown attributions in ARCHITECTURE.md
// ---------------------------------------------------------------------------

/// Re-implementation of `architecture_documents_the_label_filter_pushdown`'s
/// pairing walk, returning the first disagreement instead of panicking.
///
/// Each `X.k = "v"` path is read together with the service named after it, and
/// the pair has to be `label_filter_field`'s, in both directions. The old test
/// asserted the three paths as bare substrings, which is exactly what the
/// mutation below survives.
fn label_pushdown_verdict(architecture: &str) -> Result<(), String> {
    let prose = unwrapped(architecture);

    // The path token, closing backtick included, so the field name can be read
    // back to its opening backtick.
    const TOKEN: &str = r#".k = "v"`"#;
    let claim =
        |field: &str, kind: ResourceServiceKind| format!("`{field}{TOKEN} for {}", kind.label());

    // Forward: every service that has a pushdown, with its path attributed to
    // that service by name.
    let mut attributed: BTreeSet<String> = BTreeSet::new();
    for kind in ResourceServiceKind::ALL {
        if let Some(field) = label_filter_field(kind) {
            let claim = claim(field, kind);
            if !prose.contains(&claim) {
                return Err(format!(
                    "{kind:?} pushes down {field}, so the section must say {claim:?}"
                ));
            }
            attributed.insert(claim);
        }
    }

    // Reverse: every `X.k = "v"` path anywhere in the document, read with the
    // service beside it.
    let mut scanned: BTreeSet<String> = BTreeSet::new();
    let mut cursor = 0;
    while let Some(offset) = prose[cursor..].find(TOKEN) {
        let path_end = cursor + offset;
        let open = prose[..path_end]
            .rfind('`')
            .ok_or_else(|| "a documented label path opens with a backtick".to_string())?;
        let field = &prose[open + 1..path_end];
        let tail = &prose[path_end + TOKEN.len()..];

        let kind = ResourceServiceKind::ALL
            .into_iter()
            .find(|kind| tail.starts_with(&format!(" for {}", kind.label())))
            .ok_or_else(|| format!("the label path `{field}{TOKEN} names no service after it"))?;

        if label_filter_field(kind) != Some(field) {
            return Err(format!("the document gives `{field}{TOKEN} to {kind:?}"));
        }
        scanned.insert(claim(field, kind));
        cursor = path_end + TOKEN.len();
    }

    if scanned != attributed {
        return Err(format!(
            "the documented label paths must be exactly `label_filter_field`'s, \
             service by service: {scanned:?} vs {attributed:?}"
        ));
    }
    Ok(())
}

/// The historical defect: `labels` credited to GKE and `resourceLabels` to
/// Compute — the document telling a reader to expect a field gcloud would
/// reject, with all three path substrings still present.
fn swap_label_attributions(architecture: &str) -> String {
    mutate(
        architecture,
        "`labels.k = \"v\"` for GCE instances, `resourceLabels.k = \"v\"` for GKE",
        "`resourceLabels.k = \"v\"` for GCE instances, `labels.k = \"v\"` for GKE",
    )
}

// ---------------------------------------------------------------------------
// task 2 — the batch-action lists and their acknowledgements in README.md
// ---------------------------------------------------------------------------

/// What the code says: the two halves of the batch partition, and the
/// acknowledgement prompt each half really renders.
struct BatchFacts {
    destructive: Vec<String>,
    simple: Vec<String>,
    destructive_prompt: String,
    simple_prompt: String,
}

/// Drives the real confirmation flow for every batch action, exactly as
/// `readme_documents_the_batch_actions_that_require_a_typed_count` does, so the
/// expected values below are the app's and not retyped prose.
fn batch_facts() -> BatchFacts {
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

    // How the README names an action, and the acknowledgement line the modal
    // really renders with the eligible count swapped for the README's
    // placeholder.
    let documented_name = |action: BatchAction| {
        format!(
            "{} {}",
            action.service_label(),
            action.label().trim_end_matches(" instance")
        )
    };
    let documented_prompt = |confirmation: &BatchActionConfirmation| -> String {
        let count = confirmation.eligible().len().to_string();
        resource_batch_confirmation_lines(confirmation)
            .into_iter()
            .find(|line| line.ends_with("to confirm"))
            .expect("the confirmation modal ends with an acknowledgement line")
            .replace(&count, "<n>")
    };

    let mut destructive = Vec::new();
    let mut simple = Vec::new();
    let mut destructive_prompts: BTreeSet<String> = BTreeSet::new();
    let mut simple_prompts: BTreeSet<String> = BTreeSet::new();
    {
        let mut record = |wrapped: BatchAction, confirmation: &BatchActionConfirmation| {
            assert_eq!(
                matches!(confirmation.ack(), BatchAck::CountRequired { .. }),
                wrapped.is_destructive(),
                "{wrapped:?} opens an acknowledgement its destructiveness does not explain"
            );
            if wrapped.is_destructive() {
                destructive.push(documented_name(wrapped));
                destructive_prompts.insert(documented_prompt(confirmation));
            } else {
                simple.push(documented_name(wrapped));
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
    destructive.sort();
    simple.sort();
    BatchFacts {
        destructive_prompt: one(destructive_prompts, "destructive"),
        simple_prompt: one(simple_prompts, "non-destructive"),
        destructive,
        simple,
    }
}

/// One half of the README's partition: the action names it lists, and the
/// clause that follows the list to the end of its sentence — what that half is
/// claimed to *do*, which is the second claim the list alone does not pin.
struct BatchClaim {
    names: Vec<String>,
    clause: String,
}

fn documented_batch_claim(section: &str, opening: &str) -> Result<BatchClaim, String> {
    let services = [
        BatchAction::Compute(ComputeInstanceAction::Start).service_label(),
        BatchAction::CloudSql(CloudSqlInstanceAction::Start).service_label(),
    ];
    let at = section
        .find(opening)
        .ok_or_else(|| format!("README says {opening:?}"))?;
    let body = &section[at + opening.len()..];
    let close = body
        .find(')')
        .ok_or_else(|| format!("README closes the list after {opening:?}"))?;
    let mut names = Vec::new();
    for segment in body[..close].split(';') {
        let segment = segment.trim();
        let service = services
            .iter()
            .find(|service| segment.starts_with(*service))
            .ok_or_else(|| format!("README attributes {segment:?} to a real service"))?;
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
    names.sort();
    Ok(BatchClaim {
        names,
        clause: rest[..end].to_string(),
    })
}

/// Re-implementation of the batch pin: both lists compared as sorted `Vec`s
/// against `BatchAction::is_destructive`, and each list's trailing clause
/// pinned to the acknowledgement *its own* half renders.
fn batch_verdict(readme: &str, facts: &BatchFacts) -> Result<(), String> {
    let section = unwrapped(&document_section(readme, BATCH_SECTION)?);
    let destructive = documented_batch_claim(&section, "Destructive batch actions (")?;
    let simple = documented_batch_claim(&section, "Non-destructive batch actions (")?;

    if destructive.names != facts.destructive {
        return Err(format!(
            "the typed-count sentence must name exactly the destructive batch actions: \
             {:?} vs {:?}",
            destructive.names, facts.destructive
        ));
    }
    if simple.names != facts.simple {
        return Err(format!(
            "the `Enter` sentence must name exactly the non-destructive batch actions: \
             {:?} vs {:?}",
            simple.names, facts.simple
        ));
    }

    for (half, claim, own, other) in [
        (
            "destructive",
            &destructive,
            &facts.destructive_prompt,
            &facts.simple_prompt,
        ),
        (
            "non-destructive",
            &simple,
            &facts.simple_prompt,
            &facts.destructive_prompt,
        ),
    ] {
        let clause = &claim.clause;
        if !clause.contains(&format!("`{own}`")) {
            return Err(format!(
                "the {half} sentence must document the `{own}` prompt it renders, not {clause:?}"
            ));
        }
        if clause.contains(&format!("`{other}`")) {
            return Err(format!(
                "the {half} sentence claims the `{other}` prompt, which is the other half's"
            ));
        }
    }
    Ok(())
}

/// The historical defect: the two trailing consequence clauses swapped, both
/// action lists left exactly as they were. The README then claims destructive
/// batch actions confirm with `Enter` and non-destructive ones require typing
/// the count — the precise inverse of the code, with every substring the old
/// assertions looked for still present.
fn swap_batch_consequences(readme: &str) -> String {
    let readme = mutate(
        readme,
        "Stop, Restart, and Fail over) prompt `Type <n> to confirm` and require typing\n\
         that exact number of eligible targets, a stronger acknowledgement than the\n\
         single-resource confirmation.",
        "Stop, Restart, and Fail over) prompt `Enter to confirm` like the single-resource\n\
         flow.",
    );
    mutate(
        &readme,
        "Resume; Cloud SQL Start) prompt `Enter to confirm` like the single-resource\n\
         flow.",
        "Resume; Cloud SQL Start) prompt `Type <n> to confirm` and require typing\n\
         that exact number of eligible targets, a stronger acknowledgement than the\n\
         single-resource confirmation.",
    )
}

// ---------------------------------------------------------------------------
// task 3 — the default-jumps table in README.md
// ---------------------------------------------------------------------------

/// Re-implementation of `readme_documents_the_related_resource_jump_picker`'s
/// table walk: each row is matched to a service by its `selection_noun`, and
/// its Jumps cell has to be the `DetailTab::label`s of that service's default
/// rules, in order.
fn jumps_table_verdict(readme: &str) -> Result<(), String> {
    let mut lines = readme
        .lines()
        .skip_while(|line| !(line.starts_with("| Selection") && line.contains("| Jumps offered")));
    lines
        .next()
        .ok_or_else(|| "README documents the default jumps as a table".to_string())?;
    let rows: Vec<&str> = lines
        .skip(1)
        .take_while(|line| line.starts_with('|'))
        .collect();
    let cell = |row: &str, column: usize| -> Result<String, String> {
        Ok(row
            .split('|')
            .nth(column)
            .ok_or_else(|| format!("jumps row {row:?} has no column {column}"))?
            .trim()
            .to_string())
    };

    let defaults = JumpRules::defaults();
    let mut documented_selections: Vec<String> = Vec::new();
    let mut saw_revision_row = false;
    for row in &rows {
        let selection = cell(row, 1)?;
        let offered = cell(row, 2)?;
        let Some(kind) = ResourceServiceKind::ALL
            .iter()
            .copied()
            .find(|kind| kind.selection_noun() == selection)
        else {
            // The one row that names no service: a Cloud Run revision is a
            // sub-view of a Cloud Run service, not a `ResourceServiceKind`.
            if selection != "Cloud Run revision" {
                return Err(format!(
                    "the jumps table row {selection:?} names no known selection"
                ));
            }
            if offered != "none" {
                return Err("a Cloud Run revision can offer no jumps at all".to_string());
            }
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
        if offered != expected {
            return Err(format!(
                "the jumps table credits {selection:?} with {offered:?}, not {expected:?}"
            ));
        }
    }

    documented_selections.sort();
    let mut real_selections: Vec<String> = ResourceServiceKind::ALL
        .iter()
        .map(|kind| kind.selection_noun().to_string())
        .collect();
    real_selections.sort();
    if documented_selections != real_selections {
        return Err(format!(
            "the jumps table must have a row for exactly the services that exist: \
             {documented_selections:?} vs {real_selections:?}"
        ));
    }
    if !saw_revision_row {
        return Err("the jumps table must keep its `Cloud Run revision` row".to_string());
    }
    Ok(())
}

/// Swaps the `Jumps offered` cells of two rows, leaving the `Selection` column
/// untouched: every jump name the old `contains` looked for is still on the
/// page, attached to the wrong service. Column widths are preserved so the
/// mutated table is still a table.
fn swap_jumps_cells(readme: &str, first: &str, second: &str) -> String {
    let offered = |selection: &str| -> String {
        readme
            .lines()
            .find(|line| line.starts_with(&format!("| {selection} ")))
            .unwrap_or_else(|| panic!("the jumps table has a {selection:?} row"))
            .split('|')
            .nth(2)
            .expect("the row has a Jumps offered cell")
            .to_string()
    };
    let (a, b) = (offered(first), offered(second));
    assert_ne!(
        a.trim(),
        b.trim(),
        "swapping two identical cells would not be a mutation"
    );
    let swapped = |selection: &str, cell: &str| -> (String, String) {
        let row = readme
            .lines()
            .find(|line| line.starts_with(&format!("| {selection} ")))
            .expect("checked above")
            .to_string();
        // Same width, so the replacement cell keeps the table aligned.
        let width = row.split('|').nth(2).expect("the row has a cell").len() - 2;
        let padded = format!(" {:width$} ", cell.trim());
        let mut parts: Vec<&str> = row.split('|').collect();
        parts[2] = &padded;
        let rewritten = parts.join("|");
        (row.clone(), rewritten)
    };
    let (old_a, new_a) = swapped(first, &b);
    let (old_b, new_b) = swapped(second, &a);
    let readme = mutate(readme, &old_a, &new_a);
    mutate(&readme, &old_b, &new_b)
}

// ---------------------------------------------------------------------------
// the six verdicts
// ---------------------------------------------------------------------------

/// The pages the pins are read from, matching `USER_DOCS` in
/// tests/documentation_test.rs.
///
/// The sections this file mutates — the batch-action lists, the default-jumps
/// table — were README sections when these pins were written and are now
/// pages under `docs/`. What the pins assert is unchanged; only the file the
/// prose sits in moved, so the corpus moves with it.
const USER_DOCS: &[&str] = &[
    "README.md",
    "docs/keys.md",
    "docs/configuration.md",
    "docs/plugins.md",
    "docs/features.md",
    "docs/demo-and-read-only.md",
];

/// The user documentation, concatenated in `USER_DOCS` order.
fn user_docs() -> String {
    USER_DOCS
        .iter()
        .map(|path| {
            let page = fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("{path} is readable: {error}"));
            format!("{page}\n")
        })
        .collect()
}

#[test]
fn the_document_pins_reject_a_mutated_document() {
    // Read once, mutate only in memory. Nothing below writes to either file.
    let readme = user_docs();
    let architecture = fs::read_to_string("ARCHITECTURE.md").expect("ARCHITECTURE.md is readable");

    let mut transcript = String::from(
        "Do the converted documentation pins move with the prose?\n\
         \n\
         Each block is one pin, run twice: once against the real document and once\n\
         against a copy carrying the edit that was the historical defect. The pin has\n\
         to accept the first and reject the second. The derivations below are\n\
         re-implementations of the ones in tests/documentation_test.rs (those assert\n\
         inline and cannot be handed a string), so this records that the property\n\
         holds for a derivation of this shape, not that the shipped assertion ran.\n",
    );
    let mut section = |title: &str, body: String| {
        transcript.push_str(&format!("\n{title}\n{}\n{body}", "-".repeat(title.len())));
    };

    // ---- 1. The label-pushdown attributions -------------------------------
    label_pushdown_verdict(&architecture).expect(
        "the real ARCHITECTURE.md must be accepted: a pin that rejects everything is not a pin",
    );
    let mutated = swap_label_attributions(&architecture);
    let rejection = label_pushdown_verdict(&mutated)
        .expect_err("crediting `labels` to GKE and `resourceLabels` to Compute must be rejected");
    section(
        "1. Task 1 — ARCHITECTURE.md label-pushdown attributions",
        format!(
            "  mutation: swap which service each gcloud label path is credited to\n\
             \x20           (`labels` -> GKE clusters, `resourceLabels` -> GCE instances);\n\
             \x20           all three path substrings stay on the page.\n\
             \x20 real document: accepted\n\
             \x20 mutated copy:  rejected — {rejection}\n"
        ),
    );

    // ---- 2. The batch lists and their acknowledgements ---------------------
    let facts = batch_facts();
    batch_verdict(&readme, &facts)
        .expect("the real README.md must be accepted: a pin that rejects everything is not a pin");
    let mutated = swap_batch_consequences(&readme);
    // The lists themselves are untouched by this edit, which is the whole
    // point: a pin on the lists alone stays green while the document states
    // the exact inverse of the code. Recorded here so the rejection below is
    // visibly the clause pin's doing and not a side effect of a broken list.
    let mutated_section = unwrapped(
        &document_section(&mutated, BATCH_SECTION).expect("the mutated README keeps its section"),
    );
    let destructive = documented_batch_claim(&mutated_section, "Destructive batch actions (")
        .expect("the mutated README still lists the destructive actions");
    let simple = documented_batch_claim(&mutated_section, "Non-destructive batch actions (")
        .expect("the mutated README still lists the non-destructive actions");
    assert_eq!(
        destructive.names, facts.destructive,
        "the mutation must leave the destructive list alone"
    );
    assert_eq!(
        simple.names, facts.simple,
        "the mutation must leave the non-destructive list alone"
    );
    let rejection = batch_verdict(&mutated, &facts)
        .expect_err("a README claiming the inverse acknowledgements must be rejected");
    section(
        "2. Task 2 — README.md batch actions and their acknowledgements",
        format!(
            "  mutation: swap the two trailing consequence clauses, leaving both action\n\
             \x20           lists untouched, so the README claims destructive actions confirm\n\
             \x20           with `Enter` and non-destructive ones require typing the count.\n\
             \x20 both action lists after the mutation: still exactly the code's partition\n\
             \x20   destructive:     {:?}\n\
             \x20   non-destructive: {:?}\n\
             \x20 real document: accepted\n\
             \x20 mutated copy:  rejected — {rejection}\n",
            destructive.names, simple.names
        ),
    );

    // ---- 3. The default-jumps table ---------------------------------------
    jumps_table_verdict(&readme)
        .expect("the real README.md must be accepted: a pin that rejects everything is not a pin");
    let mutated = swap_jumps_cells(&readme, "GCE instance", "Cloud SQL instance");
    let rejection = jumps_table_verdict(&mutated)
        .expect_err("swapping two rows' `Jumps offered` cells must be rejected");
    section(
        "3. Task 3 — README.md default-jumps table",
        format!(
            "  mutation: swap the `Jumps offered` cells of the GCE instance and Cloud SQL\n\
             \x20           instance rows, leaving the Selection column untouched.\n\
             \x20 real document: accepted\n\
             \x20 mutated copy:  rejected — {rejection}\n"
        ),
    );

    // The documents on disk are what they were: every edit above was applied
    // to an owned copy.
    assert_eq!(
        user_docs(),
        readme,
        "the user documentation must be untouched on disk"
    );
    assert_eq!(
        fs::read_to_string("ARCHITECTURE.md").expect("ARCHITECTURE.md is readable"),
        architecture,
        "ARCHITECTURE.md must be untouched on disk"
    );

    let Some(dir) = evidence_dir() else {
        return;
    };
    fs::write(dir.join("doc-pinning-transcript.txt"), transcript)
        .expect("write doc pinning transcript");
}

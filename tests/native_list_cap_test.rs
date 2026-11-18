//! Every native listing is bounded by the same cap the CLI path asks for.
//!
//! `RESOURCE_LIST_LIMIT` exists to bound what one poll tick costs on a large
//! project: the gcloud path spells it `--limit=500` on every resource listing,
//! and a client that drains a paginator instead pays for the whole collection
//! every five seconds. Nothing fails when the bound is missing -- the rows are
//! a superset, the table looks right -- so the omission is invisible on a small
//! project and expensive on a large one. Eight of the fifteen native listers
//! shipped without it.
//!
//! This walks the source rather than the API because the bound cannot be
//! observed without a large live project: a `list` call reaches the network,
//! and the harnesses run with no clients at all. What it holds is narrow and
//! exact -- each listing function names the constant -- and the point is the
//! ALL-walk: a service added later is caught by the enumeration rather than by
//! whoever remembers this rule.
//!
//! The log reads are outside it. They return a `LogReadOutcome` rather than a
//! row vector, and they are bounded by the viewer's own `limit` argument --
//! the size of the log buffer, which is a different bound from the
//! resource-list cap and is chosen by the caller rather than by the service.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Listing functions that are deliberately unbounded, with the reason.
///
/// Kept as an explicit list so an exemption is a decision someone wrote down
/// rather than a silence. Each entry is `(file, function)`.
const UNBOUNDED: &[(&str, &str, &str)] = &[
    (
        "discovery.rs",
        "load_projects",
        "the project picker lists what the account can see, and the CLI path \
         caps it no more than this one does",
    ),
    (
        "discovery.rs",
        "load_regions",
        "a fixed, small set per project; the CLI path passes no --limit either",
    ),
    (
        "discovery.rs",
        "load_zones",
        "a fixed, small set per project; the CLI path passes no --limit either",
    ),
    (
        "cloud_run.rs",
        "load_revisions",
        "scoped to one service and opened from its pane, not polled; the CLI \
         path passes no --limit either",
    ),
    (
        "secrets.rs",
        "load_secret_versions",
        "scoped to one secret and opened from its pane, not polled",
    ),
    (
        "cloudsql.rs",
        "load_users",
        "scoped to one instance and opened from its Users tab, not polled",
    ),
    (
        "storage.rs",
        "load_buckets",
        "the CLI path passes no --limit on `storage buckets list` either, and \
         a project's bucket count is bounded by quota rather than by usage",
    ),
    (
        "finops.rs",
        "load_recommendations",
        "one call per (recommender, location) pair, and the CLI path passes no \
         --limit on `recommender recommendations list` either",
    ),
    (
        "iam.rs",
        "load_policy",
        "one policy, expanded into a row per member; `get-iam-policy` fetches a \
         single resource and has nothing to page",
    ),
];

/// The body of each `pub async fn load_*`/`read_*` in a native service module,
/// as `(function name, body text)`.
///
/// Split on the function header and cut at the next one, which is enough for a
/// question about whether a named constant appears inside: these modules are
/// flat, and a nested `fn` would only widen the text searched, never narrow it.
fn listing_bodies(source: &str) -> Vec<(String, String)> {
    let mut bodies = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("    pub async fn ") {
        let after = &rest[at + "    pub async fn ".len()..];
        let name: String = after
            .chars()
            .take_while(|character| character.is_alphanumeric() || *character == '_')
            .collect();
        let body_end = after.find("\n    pub async fn ").unwrap_or(after.len());
        bodies.push((name, after[..body_end].to_string()));
        rest = &after[body_end..];
        if rest.starts_with('\n') {
            rest = &rest[1..];
        }
    }
    bodies
}

/// A function that returns a collection of rows, as opposed to one resource.
///
/// `load_instance_detail` and friends fetch a single named thing and have
/// nothing to cap; the listings are the ones this rule is about.
fn is_a_listing(name: &str, body: &str) -> bool {
    !name.ends_with("_detail") && body.contains("Result<Vec<")
}

#[test]
fn every_native_listing_names_the_row_cap_or_declares_why_it_does_not() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services/sdk");
    let exempt: BTreeSet<(String, String)> = UNBOUNDED
        .iter()
        .map(|(file, function, _)| ((*file).to_string(), (*function).to_string()))
        .collect();

    let mut checked = 0usize;
    let mut uncapped: Vec<String> = Vec::new();
    let mut stale_exemptions: BTreeSet<(String, String)> = exempt.clone();

    for entry in fs::read_dir(&directory).expect("the native service directory is readable") {
        let path = entry.expect("a readable directory entry").path();
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a named file")
            .to_string();
        if file == "mod.rs" || !file.ends_with(".rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("a readable service module");
        for (function, body) in listing_bodies(&source) {
            if !is_a_listing(&function, &body) {
                continue;
            }
            checked += 1;
            let key = (file.clone(), function.clone());
            if exempt.contains(&key) {
                stale_exemptions.remove(&key);
                continue;
            }
            if !body.contains("RESOURCE_LIST_LIMIT") {
                uncapped.push(format!("{file}::{function}"));
            }
        }
    }

    assert!(
        checked >= 15,
        "the walk found only {checked} listings, which means it stopped seeing \
         them rather than that they went away"
    );
    assert!(
        uncapped.is_empty(),
        "these native listings drain the whole collection where the CLI path \
         asks for RESOURCE_LIST_LIMIT rows: {uncapped:?}. Cap the listing, or \
         add it to UNBOUNDED with the reason."
    );
    assert!(
        stale_exemptions.is_empty(),
        "these UNBOUNDED entries name nothing the walk found, so they are \
         stale and would hide a real omission under the same name: \
         {stale_exemptions:?}"
    );
}

/// A page size is a request the API can refuse, and one of them does.
///
/// `serviceAccounts.list` documents its maximum as 100 and rejects anything
/// larger with `INVALID_ARGUMENT` -- it does not clamp, it fails, so the whole
/// listing comes back as an error. Asking for the row cap there took the IAM
/// table from working to empty, and the read-path benchmark is what noticed.
/// Every other listing either documents a maximum above the row cap or clamps
/// silently, which is why only this one carries its own constant.
///
/// The pin is the pair of inequalities rather than the literal: the page size
/// must stay within what the API accepts, and below the row cap, or paging
/// stops meaning what the cap says it means.
#[test]
fn the_service_account_page_size_stays_inside_what_that_api_accepts() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/services/sdk/iam.rs"))
            .expect("the IAM service module is readable");

    let declared: i32 = source
        .split("const SERVICE_ACCOUNT_PAGE_SIZE: i32 = ")
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .and_then(|value| value.trim().parse().ok())
        .expect("the module declares a service-account page size");

    const DOCUMENTED_MAXIMUM: i32 = 100;
    assert!(
        declared <= DOCUMENTED_MAXIMUM,
        "serviceAccounts.list rejects a page larger than {DOCUMENTED_MAXIMUM}, \
         and {declared} would fail the listing rather than shorten it"
    );
    assert!(
        (declared as usize) <= nuvem::services::resources::list_filter::RESOURCE_LIST_LIMIT,
        "a page larger than the row cap would fetch rows the cap then throws away"
    );
    assert!(
        !source.contains(".set_page_size(RESOURCE_LIST_LIMIT"),
        "the row cap must not be passed to this API as a page size"
    );
}

// ------------------------------------------------- the predicates themselves
//
// The walk above asks whether each listing names the cap. These two ask what
// the cap and the placeholder rule actually decide. Both were comparisons
// written out inside bodies that reach the network, where no test could reach
// them: a mutation sweep replaced `>=` with `<` at twelve call sites and
// `!=` with `==` at a thirteenth, and every one of them survived. They are one
// function each now, and these are the tests that kill those mutants.

use nuvem::services::resources::list_filter::{RESOURCE_LIST_LIMIT, rows_are_capped};
use nuvem::services::resources::storage::is_folder_placeholder;

#[test]
fn the_row_cap_stops_at_the_cap_and_not_before_it() {
    assert!(!rows_are_capped(0, 3), "an empty listing has room");
    assert!(!rows_are_capped(2, 3), "one row short is not full");
    assert!(rows_are_capped(3, 3), "exactly the cap is full");
    assert!(
        rows_are_capped(4, 3),
        "a page that overshot the cap is still full, so the loop ends rather \
         than running until the paginator does"
    );
    assert!(
        rows_are_capped(0, 0),
        "a cap of zero admits nothing, rather than admitting everything"
    );
    assert!(
        !rows_are_capped(RESOURCE_LIST_LIMIT - 1, RESOURCE_LIST_LIMIT),
        "the boundary holds at the real cap, not only at small numbers"
    );
    assert!(rows_are_capped(RESOURCE_LIST_LIMIT, RESOURCE_LIST_LIMIT));
}

#[test]
fn only_the_object_named_for_the_prefix_is_the_folder_placeholder() {
    assert!(
        is_folder_placeholder("logs/2026/", "logs/2026/"),
        "the marker's whole name is the prefix it stands for"
    );
    assert!(
        !is_folder_placeholder("logs/2026/app.json", "logs/2026/"),
        "an object inside the folder is a row, not the marker"
    );
    assert!(
        !is_folder_placeholder("logs/2026", "logs/2026/"),
        "a name one character short of the prefix is a different object"
    );
    assert!(
        !is_folder_placeholder("logs/2026/nested/", "logs/2026/"),
        "a deeper folder's own marker belongs to that folder, not this listing"
    );
    assert!(
        is_folder_placeholder("", ""),
        "at the bucket root the prefix is empty, and so is the marker's name"
    );
}

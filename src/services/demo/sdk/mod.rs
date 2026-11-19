//! The fixture backend for the native clients.
//!
//! `DemoCommandRunner` next door answers a *subprocess*; this answers a
//! *client*. Both serve the same files under `tests/fixtures/` (the captures,
//! or the showcase under `tests/fixtures/demo/` that `nuvem --demo` lists; see
//! [`crate::services::demo::Corpus`]), which is the point: demo mode reads through the generated clients now, and a second copy
//! of any capture would let the two backends drift while every test stayed
//! green. The runner still answers what no client covers -- BigQuery, FinOps
//! budgets (Cloud Billing publishes no Rust client; the recommender calls are
//! served here natively, by `finops.rs`), gcloud's configuration and auth
//! state, label-filtered listings -- so both backends are live in a demo
//! session and the shared corpus is what keeps them telling the same story.
//!
//! A stub here is reached through [`google_cloud_gax`]'s own seam —
//! `Client::from_stub(x)` takes no credentials and no transport, so a client
//! built from one is structurally incapable of reaching the network. That is
//! the property demo mode needs, and it is a property of the type rather than
//! of a flag anyone can forget to check.
//!
//! # Why no service here answers an empty response
//!
//! The migration wired one tranche of services per commit, and while some
//! stubs answered captures and the rest did not, an un-wired one could just as
//! easily have answered an empty response. That was exactly what they must not
//! do: an empty listing paints a blank table, and a blank table is
//! indistinguishable from a project that really has no topics, no buckets and
//! no instances. Silently painting a default is the failure this whole
//! migration is built against -- it is how `count > 0` coverage stayed green
//! over half-populated rows -- so every un-wired method answered an `Err` that
//! named its service instead.
//!
//! Every service is wired now, so that machinery is gone rather than kept
//! around unused. The rule it enforced outlives it: a stub added here answers
//! a capture or an error, never a default-constructed response. What holds it
//! is each service's own per-column test and the cross-backend comparison
//! beside it, both of which fail on blank cells where a row count passes.
//!
//! This is what `--demo` now runs on. `services/runtime/session.rs` builds a
//! [`crate::services::sdk::ClientSource::Fixtures`] cache for a demo session,
//! so every stub below is on the path an operator sees and a wrong capture
//! here is a wrong cell on their screen.

use google_cloud_gax::error::Error;

/// Reads a captured `gcloud` listing as the RPC response that carries it.
///
/// The CLI answers a bare JSON array; the RPC answers a message with that
/// array in one named field, and for most of the corpus the envelope is the
/// *only* difference — the elements themselves are the same proto3 JSON, which
/// is why `pubsub_topics.json` deserializes into `Topic` values unchanged.
///
/// Wrapping at runtime is what keeps one copy of each capture. The same file
/// that the CLI parser tests read is the one a stub serves, so a capture that
/// is edited for one backend cannot go stale for the other.
///
/// Malformed JSON is reported. A wrong `field` is NOT, and this helper cannot
/// make it so — which is worth stating plainly, because it is the one thing a
/// caller here can get wrong.
///
/// `Error::deser` covers bytes that are not JSON. It does not cover naming a
/// field the message does not have: a generated deserializer absorbs an
/// unrecognised key into a private `_unknown_fields` map rather than failing,
/// so `envelope("item", …)` against a message whose field is `items` succeeds
/// and yields a response with no rows.
///
/// Two guards were tried here and neither works. Comparing the result against
/// `T::default()` fails because the absorbed key lands in `_unknown_fields`,
/// which `PartialEq` reads — so the response is *not* equal to the default
/// even though every row is gone. Round-tripping and counting the field back
/// fails too, because those unknown fields are re-serialized.
///
/// What does catch it is each service's own per-column test, and that is
/// measured rather than assumed: renaming this call's field to `item` in
/// `vpc.rs` fails `the_fixture_networks_carry_every_column_the_table_paints`
/// with `left: []` against `right: ["default"]`, and fails the cross-backend
/// comparison beside it. Every wired service has such a test, and a service
/// wired without one would have no coverage of its field name at all.
pub fn envelope<T: serde::de::DeserializeOwned>(field: &str, bytes: &str) -> Result<T, Error> {
    let items: serde_json::Value = serde_json::from_str(bytes).map_err(Error::deser)?;
    let message = serde_json::Value::Object(
        [(field.to_string(), items)]
            .into_iter()
            .collect::<serde_json::Map<String, serde_json::Value>>(),
    );
    serde_json::from_value(message).map_err(Error::deser)
}

/// Answers a `get` with the resource it names, as the live API does.
///
/// `documents` are searched in order, each either one resource or a listing of
/// them, and the first resource whose `name` satisfies `wanted` is the answer.
/// A detail capture comes first, so the resource it describes keeps its full
/// document; the listing after it answers every other row. That is faithful
/// rather than a shortcut: each of these listings returns whole resources, the
/// same message `get` returns, so a row is a complete detail of itself.
///
/// A name no document carries is `NotFound`, which is what the live API
/// answers and what a detail pane shows as a failure. Answering the capture
/// for every name instead, as these stubs once did, handed the pane a
/// document about another resource, and the pane waited on its own forever.
pub fn named<T: serde::de::DeserializeOwned>(
    documents: &[&str],
    wanted: impl Fn(&str) -> bool,
    description: &str,
) -> Result<T, Error> {
    for document in documents {
        let value: serde_json::Value = serde_json::from_str(document).map_err(Error::deser)?;
        let candidates = match value {
            serde_json::Value::Array(rows) => rows,
            single => vec![single],
        };
        for candidate in candidates {
            let matches = candidate
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(&wanted);
            if matches {
                return serde_json::from_value(candidate).map_err(Error::deser);
            }
        }
    }
    Err(Error::service(
        google_cloud_gax::error::rpc::Status::default()
            .set_code(google_cloud_gax::error::rpc::Code::NotFound)
            .set_message(format!("demo fixtures hold no {description}")),
    ))
}

/// The rows of a listing whose `name` starts with `prefix`, as JSON: the
/// children of one parent, for a showcase listing that holds every parent's.
pub fn rows_under(listing: &str, prefix: &str) -> Result<String, Error> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(listing).map_err(Error::deser)?;
    let mine: Vec<serde_json::Value> = rows
        .into_iter()
        .filter(|row| {
            row.get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    serde_json::to_string(&mine).map_err(Error::ser)
}

/// The last `/`-separated segment of a resource name: the short name a row
/// paints, whichever of the API's spellings carried it.
pub fn short_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Regroups a captured flat listing into the scope map an *aggregated* RPC
/// answers.
///
/// `instances.aggregatedList`, `disks.aggregatedList` and
/// `subnetworks.aggregatedList` do not answer an array at all. They answer a
/// map keyed by scope -- `zones/<zone>` or `regions/<region>` -- whose values
/// are per-scope objects holding that scope's rows, and the capture is what
/// gcloud printed after flattening that map. No field name turns one into the
/// other, so [`envelope`] does not apply and the grouping has to be rebuilt.
///
/// It is rebuilt here, at call time, from the bytes that are already on disk.
/// The alternative -- a second file recording which rows belong to which scope
/// -- is what this whole module exists to avoid: a grouping written down beside
/// the data can say one zone while the row's own field, and therefore the
/// painted cell, says another, and nothing would notice. `scope_of` is handed
/// each deserialized row so that the key can be read off the row itself, with
/// the same function the service's mapper reads it with.
///
/// Rows keep the capture's order within a scope. Across scopes there is no
/// order to keep: the generated response carries a `HashMap`, and the item
/// paginator hands back `items.into_iter()`.
pub fn by_scope<T: serde::de::DeserializeOwned>(
    bytes: &str,
    scope_of: impl Fn(&T) -> String,
) -> Result<std::collections::HashMap<String, Vec<T>>, Error> {
    let rows: Vec<T> = serde_json::from_str(bytes).map_err(Error::deser)?;
    let mut scopes: std::collections::HashMap<String, Vec<T>> = std::collections::HashMap::new();
    for row in rows {
        scopes.entry(scope_of(&row)).or_default().push(row);
    }
    Ok(scopes)
}

/// The key an aggregated listing files a row under, from the row's own
/// already-shortened zone and region.
///
/// Both arguments are `Option` rather than the `unwrap_or_default()` string
/// the mappers paint with, because the distinction that is flattened there is
/// the one that decides the key here: a row carrying no `zone` is zonal
/// nowhere, and a `zones/` key built from an empty id would claim otherwise.
/// A disk carries exactly one of the two and `zone` wins, matching the
/// precedence `map_disk` reads them in.
///
/// `global` is the API's own name for the scope of a row that is neither
/// zonal nor regional; `persistent_disks.json` carries one such row. Filing it
/// under a scope rather than dropping it is what keeps the native backend's
/// row count equal to the CLI backend's on the same bytes.
///
/// # Why a test pins the spelling that the code cannot
///
/// Nothing in the generated stack reads these keys back. The item paginator
/// flattens the map with `items.into_iter()` without looking at a key, and
/// each of the three services binds the scope out of the entry and discards it
/// (`let (_scope, scoped) = entry?`). So a key spelled bare, or spelled as a
/// full self-link, or one key holding every row, all paint exactly the same
/// table today -- the failure is not an empty listing but a *silent* one, and
/// it stays silent until the day something paints the scope or filters on it.
/// What breaks immediately is the only claim demo mode rests on: that the
/// fixture backend answers the shape the live API answers.
/// `demo_sdk_fixture_test` therefore asserts these keys by name, walking the
/// paginator itself, because no assertion about rows can see them.
pub fn scope(zone: Option<String>, region: Option<String>) -> String {
    match (zone, region) {
        (Some(zone), _) => format!("zones/{zone}"),
        (None, Some(region)) => format!("regions/{region}"),
        (None, None) => "global".to_string(),
    }
}

/// The project ids this demo universe contains, read off the capture the
/// project picker itself lists.
///
/// Sourced rather than written down for the reason the whole module is: a
/// second list of project names beside the capture could say one thing while
/// `search_projects` answered another, and nothing would notice. The demo
/// configurations name `dev-project` and `prod-project`, the picker lists the
/// same two, and an operator switching configurations in `--demo` moves
/// between them -- so neither is special and neither can be hard-coded here.
fn demo_projects() -> &'static std::collections::BTreeSet<String> {
    static PROJECTS: std::sync::OnceLock<std::collections::BTreeSet<String>> =
        std::sync::OnceLock::new();
    PROJECTS.get_or_init(|| {
        #[derive(serde::Deserialize)]
        struct Row {
            #[serde(rename = "projectId")]
            project_id: String,
        }
        let rows: Vec<Row> = serde_json::from_str(discovery::PROJECTS)
            .expect("the projects capture is valid JSON and every row names a projectId");
        rows.into_iter().map(|row| row.project_id).collect()
    })
}

/// The error the live API answers for a project the caller cannot see.
fn no_such_project(project: &str) -> Error {
    Error::service(
        google_cloud_gax::error::rpc::Status::default()
            .set_code(google_cloud_gax::error::rpc::Code::NotFound)
            .set_message(format!(
                "demo fixtures hold no project `{project}`; this demo universe \
                 contains {:?}",
                demo_projects()
            )),
    )
}

/// The error the live API answers for a routing field it cannot bind.
fn malformed_parent(parent: &str, template: &str) -> Error {
    Error::service(
        google_cloud_gax::error::rpc::Status::default()
            .set_code(google_cloud_gax::error::rpc::Code::InvalidArgument)
            .set_message(format!(
                "request parent `{parent}` does not match `{template}`"
            )),
    )
}

/// Checks the bare project id a Compute listing is scoped to, aggregated or
/// flat.
///
/// # Why a stub judges its request at all
///
/// Because otherwise nothing does. A stub-backed client re-runs the real
/// request builder, which is half the reason the seam sits at the wire rather
/// than at the service: the `parent` a service formats, the page size it asks
/// for and the flags it sends are all built on the demo path exactly as they
/// are on the live one. But *building* a request nobody reads proves only that
/// the builder did not panic. Aiming `load_instances` at
/// `projects/{project}` instead of `{project}`, or at a project that does not
/// exist, is a listing that fails or comes back empty against Google and a
/// green suite here -- which is the same class of silence this module's
/// no-default-responses rule exists to close, one field earlier.
///
/// Compute's aggregated RPCs take the project as a bare id, not a resource
/// path: the routing template is `/compute/v1/projects/{project}/aggregated/…`
/// and the segment is substituted, so a value carrying its own `projects/`
/// prefix produces a URL with two.
pub fn aggregated_project(project: &str) -> Result<(), Error> {
    if project.is_empty() || project.contains('/') {
        return Err(malformed_parent(project, "{project}"));
    }
    if !demo_projects().contains(project) {
        return Err(no_such_project(project));
    }
    Ok(())
}

/// Checks the two flags that decide which scopes an aggregated listing covers.
///
/// Both are what `gcloud compute instances list` and `gcloud compute disks
/// list` send, and both change what comes back rather than how fast: without
/// `include_all_scopes` the response is confined to the scopes the API picks
/// by default, and without `return_partial_success` a single unreachable zone
/// fails the whole listing where the CLI backend returned every zone that
/// answered. Flipping either to `false` is therefore a behaviour change on the
/// live path, and until this was read it was one the whole suite passed over.
///
/// `Subnetworks::aggregated_list` sends neither, so `vpc.rs` checks only the
/// project. That is a real difference between the three callers rather than an
/// oversight here, and it is left alone because closing it would change what a
/// live subnets listing returns.
pub fn fans_out_over_every_scope(
    include_all_scopes: Option<bool>,
    return_partial_success: Option<bool>,
) -> Result<(), Error> {
    for (field, value) in [
        ("include_all_scopes", include_all_scopes),
        ("return_partial_success", return_partial_success),
    ] {
        if value != Some(true) {
            return Err(Error::service(
                google_cloud_gax::error::rpc::Status::default()
                    .set_code(google_cloud_gax::error::rpc::Code::InvalidArgument)
                    .set_message(format!(
                        "aggregated listing sent {field}={value:?}; gcloud sends \
                         true, and anything else changes which scopes answer"
                    )),
            ));
        }
    }
    Ok(())
}

/// Checks a resource-path `parent` against the routing template it must match.
///
/// `template` is the template as the API documents it, `*` standing for a
/// substituted segment. A `*` in the position after `projects` is also checked
/// against `demo_projects`; the literal `_` that Storage uses for
/// "project inferred from the bucket" is matched as a literal and not looked
/// up, because it names no project.
///
/// Segment count and literals both, because the two mistakes look different:
/// a parent built from the wrong pieces (`locations/{region}` with the
/// `projects/` half dropped) changes the count, and one built in the wrong
/// order keeps it.
pub fn parent_matches(parent: &str, template: &str) -> Result<(), Error> {
    let actual: Vec<&str> = parent.split('/').collect();
    let expected: Vec<&str> = template.split('/').collect();
    if actual.len() != expected.len() {
        return Err(malformed_parent(parent, template));
    }
    let mut previous = "";
    for (segment, pattern) in actual.iter().zip(&expected) {
        match *pattern {
            "*" => {
                if segment.is_empty() {
                    return Err(malformed_parent(parent, template));
                }
                if previous == "projects" && !demo_projects().contains(*segment) {
                    return Err(no_such_project(segment));
                }
            }
            literal if literal == *segment => {}
            _ => return Err(malformed_parent(parent, template)),
        }
        previous = pattern;
    }
    Ok(())
}

pub mod cloud_run;
pub mod cloudsql;
pub mod compute;
pub mod discovery;
pub mod disks;
pub mod finops;
pub mod functions;
pub mod gke;
pub mod iam;
pub mod logging;
pub mod pubsub;
pub mod secrets;
pub mod storage;
pub mod vpc;

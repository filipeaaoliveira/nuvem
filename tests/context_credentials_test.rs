//! The credentials bridge: a token from `gcloud config config-helper` for one
//! named configuration, cached in memory to its own expiry, never on disk and
//! never in a `Debug` rendering.
//!
//! Every test stands a scripted runner behind the provider. The runner records
//! each argv it is handed, so a test can say how many times gcloud was asked
//! and for which configuration — which is the whole contract: one call per
//! token lifetime, aimed by name.

use std::sync::{Arc, Mutex};

use google_cloud_auth::credentials::{CacheableResource, CredentialsProvider, EntityTag};
use http::Extensions;
use nuvem::services::gcloud::cli::{CommandError, CommandOutput, CommandRunner, GcloudCommand};
use nuvem::services::gcloud::context_credentials::ConfigurationCredentials;

const TOKEN: &str = "ya29.PLANTED-TOKEN-VALUE";

/// Answers every `config-helper` call with the same token and a scripted
/// expiry, and remembers the argv of every call.
struct HelperRunner {
    expiry: Option<String>,
    universe_domain: Option<&'static str>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl HelperRunner {
    fn new(expiry: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            expiry,
            universe_domain: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.calls.lock().expect("calls").len()
    }
}

impl CommandRunner for HelperRunner {
    fn run(&self, command: GcloudCommand) -> Result<CommandOutput, CommandError> {
        self.calls.lock().expect("calls").push(command.args.clone());
        let expiry = self
            .expiry
            .as_deref()
            .map(|value| format!(r#", "token_expiry": "{value}""#))
            .unwrap_or_default();
        let properties = match self.universe_domain {
            Some(domain) => format!(
                r#", "configuration": {{"properties": {{"core": {{"universe_domain": "{domain}"}}}}}}"#
            ),
            None => String::new(),
        };
        Ok(CommandOutput {
            stdout: format!(
                r#"{{"credential": {{"access_token": "{TOKEN}"{expiry}}}{properties}}}"#
            ),
            stderr: String::new(),
        })
    }
}

fn far_expiry() -> String {
    (chrono::Utc::now() + chrono::Duration::hours(1))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn near_expiry() -> String {
    (chrono::Utc::now() + chrono::Duration::seconds(30))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

fn bearer(headers: &CacheableResource<http::HeaderMap>) -> Option<&http::HeaderValue> {
    match headers {
        CacheableResource::New { data, .. } => data.get(http::header::AUTHORIZATION),
        CacheableResource::NotModified => None,
    }
}

#[test]
fn the_token_is_asked_of_the_named_configuration_and_sent_as_a_sensitive_bearer() {
    let runner = HelperRunner::new(Some(far_expiry()));
    let credentials = ConfigurationCredentials::new(runner.clone(), "staging");

    let headers = runtime()
        .block_on(credentials.headers(Extensions::new()))
        .expect("headers");
    let value = bearer(&headers).expect("an Authorization header");

    assert_eq!(value.to_str().unwrap(), format!("Bearer {TOKEN}"));
    assert!(
        value.is_sensitive(),
        "the bearer token must be marked sensitive so a Debug of the header map redacts it"
    );
    let argv = runner.calls.lock().unwrap()[0].clone();
    assert_eq!(
        argv,
        [
            "--configuration",
            "staging",
            "config",
            "config-helper",
            "--format=json"
        ],
        "the token is resolved for the configuration named here, not the machine's active one"
    );
}

#[test]
fn a_token_far_from_expiry_is_served_from_the_cache() {
    let runner = HelperRunner::new(Some(far_expiry()));
    let credentials = ConfigurationCredentials::new(runner.clone(), "dev");
    let rt = runtime();
    rt.block_on(credentials.headers(Extensions::new()))
        .expect("first");
    rt.block_on(credentials.headers(Extensions::new()))
        .expect("second");
    rt.block_on(credentials.headers(Extensions::new()))
        .expect("third");
    assert_eq!(
        runner.calls(),
        1,
        "three requests within the token's life cost one gcloud call"
    );
}

#[test]
fn a_token_inside_the_expiry_margin_is_refreshed() {
    let runner = HelperRunner::new(Some(near_expiry()));
    let credentials = ConfigurationCredentials::new(runner.clone(), "dev");
    let rt = runtime();
    rt.block_on(credentials.headers(Extensions::new()))
        .expect("first");
    rt.block_on(credentials.headers(Extensions::new()))
        .expect("second");
    assert_eq!(
        runner.calls(),
        2,
        "a token thirty seconds from expiry is inside the one-minute margin and is not reused"
    );
}

#[test]
fn a_token_with_no_stated_expiry_is_refreshed_every_time() {
    let runner = HelperRunner::new(None);
    let credentials = ConfigurationCredentials::new(runner.clone(), "dev");
    let rt = runtime();
    rt.block_on(credentials.headers(Extensions::new()))
        .expect("first");
    rt.block_on(credentials.headers(Extensions::new()))
        .expect("second");
    assert_eq!(
        runner.calls(),
        2,
        "no expiry means refresh every time, never never-expires"
    );
}

#[test]
fn an_empty_expiry_string_is_no_expiry() {
    let runner = HelperRunner::new(Some(String::new()));
    let credentials = ConfigurationCredentials::new(runner.clone(), "dev");
    let rt = runtime();
    rt.block_on(credentials.headers(Extensions::new()))
        .expect("first");
    rt.block_on(credentials.headers(Extensions::new()))
        .expect("second");
    assert_eq!(
        runner.calls(),
        2,
        "gcloud documents the field as possibly empty; empty is absent"
    );
}

#[test]
fn a_matching_entity_tag_answers_not_modified() {
    let runner = HelperRunner::new(Some(far_expiry()));
    let credentials = ConfigurationCredentials::new(runner.clone(), "dev");
    let rt = runtime();
    let first = rt
        .block_on(credentials.headers(Extensions::new()))
        .expect("first");
    let CacheableResource::New { entity_tag, .. } = first else {
        panic!("the first answer carries headers and a tag");
    };
    let mut extensions = Extensions::new();
    extensions.insert::<EntityTag>(entity_tag);
    let second = rt
        .block_on(credentials.headers(extensions))
        .expect("second");
    assert!(
        matches!(second, CacheableResource::NotModified),
        "a caller presenting the tag of the token it already holds gets NotModified"
    );
}

#[test]
fn debug_never_renders_the_token() {
    let runner = HelperRunner::new(Some(far_expiry()));
    let credentials = ConfigurationCredentials::new(runner.clone(), "dev");
    let before = format!("{credentials:?}");
    assert!(
        before.contains("<none>"),
        "before any call the cache reads as empty: {before}"
    );
    runtime()
        .block_on(credentials.headers(Extensions::new()))
        .expect("headers");
    let after = format!("{credentials:?}");
    assert!(
        after.contains("<cached>"),
        "after a call the cache reads as held: {after}"
    );
    for rendering in [&before, &after] {
        assert!(
            !rendering.contains(TOKEN),
            "the token reached a Debug rendering: {rendering}"
        );
        assert!(
            rendering.contains("\"dev\""),
            "the configuration name is what identifies the provider: {rendering}"
        );
    }
}

#[test]
fn the_universe_domain_is_the_configurations_own() {
    let mut inner = HelperRunner::new(Some(far_expiry()));
    Arc::get_mut(&mut inner).unwrap().universe_domain = Some("example.test");
    let credentials = ConfigurationCredentials::new(inner, "dev");
    let domain = runtime().block_on(credentials.universe_domain());
    assert_eq!(domain.as_deref(), Some("example.test"));

    let plain = ConfigurationCredentials::new(HelperRunner::new(Some(far_expiry())), "dev");
    assert_eq!(
        runtime().block_on(plain.universe_domain()),
        None,
        "a configuration that sets no universe domain reports none rather than a guess"
    );
}

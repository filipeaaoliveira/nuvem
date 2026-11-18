//! Credentials that come from the gcloud configuration the operator selected.
//!
//! The native clients authenticate through Application Default Credentials by
//! default, and ADC is machine-wide: `application_default_credentials.json`
//! sits beside `configurations/` rather than inside any one of them, so it has
//! no idea which configuration is active. This application's whole model is
//! that the operator picks a configuration and every reading follows from it,
//! so a client authenticating itself would be aimed somewhere the operator did
//! not choose — the defect `docs/decisions/0001-native-sdk-for-read-paths.md`
//! refuses.
//!
//! `gcloud config config-helper` is the bridge, and it is the same one `bq`
//! already inherits the guarantee through. The SDK documents it as "a helper
//! for providing auth and config data to external tools … appropriate when
//! external tools want to operate within the context of the user's current
//! gcloud session", and it answers with a token *and* the full property set of
//! the named configuration.
//!
//! The token is cached to its own expiry, because acquiring one costs a Python
//! interpreter start and the poll would otherwise pay that on every tick —
//! which is the cost this migration exists to remove. The cache is keyed by
//! configuration name, so a context switch cannot be served a token minted for
//! the configuration the operator just left.

use std::sync::{Arc, Mutex};

use google_cloud_auth::credentials::{
    CacheableResource, Credentials, CredentialsProvider, EntityTag,
};
use http::{Extensions, HeaderMap, HeaderValue};

use crate::services::gcloud::cli::{CommandRunner, GcloudCommand};

/// How long before a token's stated expiry it stops being handed out.
///
/// A token that expires mid-flight fails the call it was attached to, and the
/// listing that fails is the one the operator is looking at. One minute is
/// longer than any call this application makes.
const EXPIRY_MARGIN: chrono::Duration = chrono::Duration::minutes(1);

#[derive(Clone)]
struct CachedToken {
    access_token: String,
    /// `None` when gcloud reports no expiry, which it does for some credential
    /// types. Treated as "refresh every time" rather than "never expires":
    /// guessing long on a token's life is how a session starts failing every
    /// call at once.
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    universe_domain: Option<String>,
    entity_tag: EntityTag,
}

// Deliberately no `Debug`, derived or written: a type that cannot be
// formatted cannot reach a log line through any rendering of the structs that
// own it, which is a stronger guarantee than a redacting implementation.
impl CachedToken {
    fn usable_at(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        match self.expires_at {
            Some(expiry) => now + EXPIRY_MARGIN < expiry,
            None => false,
        }
    }
}

/// The `config-helper` payload, named as gcloud spells it. Deliberately not
/// `Debug`: it carries the token.
#[derive(serde::Deserialize)]
struct ConfigHelper {
    credential: ConfigHelperCredential,
    #[serde(default)]
    configuration: ConfigHelperConfiguration,
}

#[derive(serde::Deserialize)]
struct ConfigHelperCredential {
    access_token: String,
    /// UTC, formatted `%Y-%m-%dT%H:%M:%SZ`, and documented as possibly empty.
    #[serde(default)]
    token_expiry: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigHelperConfiguration {
    /// The full active property set, section by section.
    #[serde(default)]
    properties: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
}

/// A [`CredentialsProvider`] backed by one named gcloud configuration.
pub struct ConfigurationCredentials {
    runner: Arc<dyn CommandRunner + Send + Sync>,
    configuration: String,
    cached: Mutex<Option<CachedToken>>,
}

/// Written by hand rather than derived: the runner is not `Debug`, and the
/// cache holds a bearer token that must not reach a log line through a derived
/// rendering of the struct that owns it.
impl std::fmt::Debug for ConfigurationCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfigurationCredentials")
            .field("configuration", &self.configuration)
            .field(
                "token",
                &match self.cached.lock() {
                    Ok(cache) if cache.is_some() => "<cached>",
                    Ok(_) => "<none>",
                    Err(_) => "<poisoned>",
                },
            )
            .finish()
    }
}

impl ConfigurationCredentials {
    pub fn new(runner: Arc<dyn CommandRunner + Send + Sync>, configuration: &str) -> Self {
        Self {
            runner,
            configuration: configuration.to_string(),
            cached: Mutex::new(None),
        }
    }

    /// The gcloud invocation this provider is built on.
    ///
    /// `--configuration` is a gcloud-wide flag, so the token comes back
    /// resolved for the configuration named here rather than for whichever one
    /// happens to be active on the machine.
    fn command(&self) -> GcloudCommand {
        GcloudCommand::new([
            "--configuration".to_string(),
            self.configuration.clone(),
            "config".to_string(),
            "config-helper".to_string(),
            "--format=json".to_string(),
        ])
    }

    /// Returns a usable token, refreshing through gcloud only when the cached
    /// one is missing or close enough to expiry to be risky.
    async fn token(&self) -> Result<CachedToken, google_cloud_auth::errors::CredentialsError> {
        let now = chrono::Utc::now();
        if let Some(cached) = self
            .cached
            .lock()
            .expect("token cache is not poisoned")
            .as_ref()
            .filter(|cached| cached.usable_at(now))
        {
            return Ok(cached.clone());
        }

        let runner = Arc::clone(&self.runner);
        let command = self.command();
        // The runner blocks: it spawns gcloud and waits. Doing that on the
        // reactor would stall every other task for the ~1.3s an interpreter
        // start costs.
        let output = tokio::task::spawn_blocking(move || runner.run(command))
            .await
            .map_err(|error| {
                google_cloud_auth::errors::CredentialsError::from_msg(true, error.to_string())
            })?
            .map_err(|error| {
                google_cloud_auth::errors::CredentialsError::from_msg(true, error.to_string())
            })?;

        let helper: ConfigHelper = serde_json::from_str(&output.stdout).map_err(|error| {
            google_cloud_auth::errors::CredentialsError::from_msg(false, error.to_string())
        })?;

        let expires_at = helper
            .credential
            .token_expiry
            .as_deref()
            .filter(|value| !value.is_empty())
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&chrono::Utc));

        let token = CachedToken {
            access_token: helper.credential.access_token,
            expires_at,
            universe_domain: helper
                .configuration
                .properties
                .get("core")
                .and_then(|core| core.get("universe_domain"))
                .cloned(),
            entity_tag: EntityTag::new(),
        };

        *self.cached.lock().expect("token cache is not poisoned") = Some(token.clone());
        Ok(token)
    }
}

impl CredentialsProvider for ConfigurationCredentials {
    async fn headers(
        &self,
        extensions: Extensions,
    ) -> std::result::Result<
        CacheableResource<HeaderMap>,
        google_cloud_auth::errors::CredentialsError,
    > {
        let token = self.token().await?;
        if let Some(tag) = extensions.get::<EntityTag>()
            && token.entity_tag.eq(tag)
        {
            return Ok(CacheableResource::NotModified);
        }

        let mut headers = HeaderMap::new();
        let mut value =
            HeaderValue::from_str(&format!("Bearer {}", token.access_token)).map_err(|error| {
                google_cloud_auth::errors::CredentialsError::from_msg(false, error.to_string())
            })?;
        // The token is a credential: keep it out of any `Debug` rendering of
        // the header map, which is otherwise reachable from tracing.
        value.set_sensitive(true);
        headers.insert(http::header::AUTHORIZATION, value);

        Ok(CacheableResource::New {
            data: headers,
            entity_tag: token.entity_tag,
        })
    }

    async fn universe_domain(&self) -> Option<String> {
        self.token()
            .await
            .ok()
            .and_then(|token| token.universe_domain)
    }
}

/// Builds SDK [`Credentials`] aimed by the named gcloud configuration.
pub fn credentials_for(
    runner: Arc<dyn CommandRunner + Send + Sync>,
    configuration: &str,
) -> Credentials {
    Credentials::from(ConfigurationCredentials::new(runner, configuration))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(expires_at: Option<chrono::DateTime<chrono::Utc>>) -> CachedToken {
        CachedToken {
            access_token: "t".to_string(),
            expires_at,
            universe_domain: None,
            entity_tag: EntityTag::new(),
        }
    }

    /// The margin is exclusive: a token whose expiry is exactly one margin
    /// away is already too close, and one a second further is not. Pinned
    /// with a fixed clock because the boundary cannot be hit from outside.
    #[test]
    fn the_expiry_margin_is_exclusive_at_the_boundary() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(!token(Some(now + EXPIRY_MARGIN)).usable_at(now));
        assert!(token(Some(now + EXPIRY_MARGIN + chrono::Duration::seconds(1))).usable_at(now));
        assert!(!token(Some(now + EXPIRY_MARGIN - chrono::Duration::seconds(1))).usable_at(now));
        assert!(!token(None).usable_at(now));
    }
}

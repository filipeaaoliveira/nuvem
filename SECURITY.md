# Security Policy

## Supported versions

nuvem is pre-1.0. Only the latest release (and `main`) receives security
fixes.

## Reporting a vulnerability

Please do not open a public issue for security problems. Instead, use GitHub's
private vulnerability reporting on this repository
(*Security → Report a vulnerability*). You should get an initial response
within a week.

## Scope notes

nuvem reads Google Cloud through the official Google Cloud Rust client
libraries and changes it through the locally installed `gcloud` CLI, inheriting
gcloud's authentication in both cases. It stores no credentials on disk: the
bearer token the clients use comes from `gcloud config config-helper`, is kept
in memory only, is cached per configuration name until one minute before its
stated expiry, and is discarded on a configuration switch. Of the gcloud
properties that shape a request, the native path applies `core/universe_domain`
and does not yet apply `api_endpoint_overrides`, `billing/quota_project` or the
`proxy/*` section, which `gcloud`'s own invocations still honour. Areas of
particular interest for review:

- **The in-memory access token** (`src/services/gcloud/context_credentials.rs`):
  the token is marked sensitive on the `Authorization` header, kept out of the
  provider's `Debug`, and the log file pins every client crate at `info` so a
  request body cannot be traced out. A path that logs, renders, copies or
  persists the token is a vulnerability.

- **Diagnostics sanitisation** (`crates/nuvem-domain`): gcloud output shown in
  diagnostics or copied to the clipboard is scrubbed of bearer tokens, API
  keys, and refresh tokens. A path where raw output escapes unsanitised is a
  vulnerability.
- **Secret Manager views**: nuvem lists secret metadata but deliberately never
  accesses secret payloads (`tests/secrets_no_payload_access_test.rs` pins
  this). A change that reads payload data is a regression.
- **Plugins**: a plugin declaration names arguments to `gcloud` — there is no
  program field and no shell on the path. `gcloud` itself can hand an argv to
  `ssh` (`gcloud compute ssh … --ssh-flag=…`), so the boundary is the
  confirmation modal showing the exact argv, not the fixed program. A way to
  start a process without that confirmation is a vulnerability.
- **Terminal output**: untrusted resource names and log payloads are rendered
  to the terminal; control-character injection that survives sanitisation is
  in scope.

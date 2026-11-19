# Security policy

## Supported versions

nuvem is pre-1.0. Only the latest release (and `main`) receives security
fixes.

## Reporting a vulnerability

Do not open a public issue for a security problem. Use GitHub's private
vulnerability reporting on this repository instead
(*Security → Report a vulnerability*). Expect a first response within a week.

## Scope notes

nuvem reads Google Cloud through the official Google Cloud Rust client
libraries. Lifecycle actions on GCE and Cloud SQL instances go through the same
libraries. Other changes, such as configuration changes, SSH and plugins, run
the locally installed `gcloud` CLI. Both paths use gcloud's authentication.

nuvem stores no credentials on disk. The bearer token the clients use comes
from `gcloud config config-helper`. nuvem keeps it in memory only, caches it per
configuration name until one minute before its stated expiry, and discards it
on a configuration switch.

Of the gcloud properties that shape a request, the native path applies
`core/universe_domain`. It does not yet apply `api_endpoint_overrides`,
`billing/quota_project` or the `proxy/*` section, which `gcloud`'s own
invocations still honor.

These areas are of particular interest for review:

- The in-memory access token (`src/services/gcloud/context_credentials.rs`).
  The token is marked sensitive on the `Authorization` header and kept out of
  the provider's `Debug`. The log file pins every client crate at `info`, so a
  request body cannot be traced out. A path that logs, renders, copies or
  persists the token is a vulnerability.
- Diagnostics sanitization (`crates/nuvem-domain`). gcloud output shown in
  diagnostics or copied to the clipboard is scrubbed of bearer tokens, API
  keys, and refresh tokens. A path where raw output escapes unsanitized is a
  vulnerability.
- Secret Manager views. nuvem lists secret metadata and deliberately never
  accesses secret payloads (`tests/secrets_no_payload_access_test.rs` pins this). A change that
  reads payload data is a regression.
- Plugins. A plugin declaration names arguments to `gcloud`. There is no
  program field and no shell on the path. `gcloud` itself can hand an argv to
  `ssh` (`gcloud compute ssh … --ssh-flag=…`), so the boundary is the
  confirmation that shows the exact argv, not the fixed program. A way to
  start a process without that confirmation is a vulnerability.
- Terminal output. nuvem renders untrusted resource names and log payloads to
  the terminal. Control-character injection that survives sanitization is in
  scope.

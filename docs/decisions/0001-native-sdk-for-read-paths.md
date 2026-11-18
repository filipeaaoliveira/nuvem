# 1. Native Google Cloud SDK for the read paths

Status: accepted
Date: 2026-09-02

## Context

Every listing and every detail load in this application is a `gcloud`
subprocess. `gcloud` is a shell stub that execs a Python interpreter, so each
call pays a full interpreter start before any data moves.

The background poll re-lists the active workspace on a `poll_interval_secs`
cadence, whose default is 5 seconds. Measured over one ordinary working day on
one machine, from `~/.config/gcloud/logs`:

| command | invocations | window | gap |
| --- | --- | --- | --- |
| `storage buckets list` | 847 | 71 minutes | 5s |
| `container clusters list` | 217 | 5 hours | 5s |
| everything else | 119 | — | on demand |

801 of the 846 intervals between bucket listings were exactly five seconds.
That is one session sitting on a workspace, not unusual usage.

The per-call cost, measured on the same machine against the same project:

| | wall |
| --- | --- |
| `gcloud storage buckets list --format=json` | 1.32s – 1.56s |
| the same data over HTTPS with a cached token | 0.31s – 0.47s |

Both return the same eleven buckets. Roughly **1.0s of every call is
interpreter startup**, about 70% of the wall time. At a 5 second poll that is
~12 minutes of overhead and ~8 minutes of CPU per hour, for as long as the
application is open.

`ARCHITECTURE.md` already records a decision that this application gains no
REST path. That decision stands on its own reasoning and is not overturned
here; what follows is what changed since it was written, and what it means for
a *client library* rather than for hand-assembled REST.

## What changed since that decision

**An official SDK exists.** Google published Google Cloud client libraries for
Rust in September 2025. Every service this application reads has a crate, and
all but BigQuery are past 1.0: `google-cloud-compute-v1` 3.5,
`google-cloud-container-v1` 1.13, `google-cloud-storage` 1.18,
`google-cloud-sql-v1` 2.10, `google-cloud-logging-v2` 1.14,
`google-cloud-secretmanager-v1` 1.13, `google-cloud-iam-v1` 1.12,
`google-cloud-pubsub` 1.4, with `google-cloud-monitoring-v3` 1.12 covering the
time series that prompted the original question.

**The endpoint objection is answered by a client library.** The recorded
decision argues that a hand-built call would hold "a URL and a header set it
assembled itself", silently dropping `api_endpoint_overrides`,
`core/universe_domain`, `billing/quota_project` and the proxy section. That is
true of hand-rolled REST. A generated client resolves endpoints and universe
domain itself, and `ClientBuilder::with_endpoint` exists for the rest.

**Three of the four configuration properties are retrieved but not applied.**
`config-helper` returns the full property set and the credentials provider
consumes `core/universe_domain` only. `api_endpoint_overrides`,
`billing/quota_project` and the `proxy` section are dropped on the native read
paths while `gcloud` continues to honour them on every mutation, so a
configuration that sets any of the three reads and writes through differently
aimed transports. `ClientBuilder::with_endpoint` and a proxy-aware transport
are where that would be closed.

**The configuration properties are retrievable.** `gcloud config config-helper`
is documented in the SDK's own source as "a helper for providing auth and
config data to external tools … appropriate when external tools want to operate
within the context of the user's current gcloud session". It returns
`credential.access_token`, `credential.token_expiry`, and
`configuration.properties` built from `properties.VALUES.AllValues()` — the
full property set, including all four properties the decision feared losing.
This is the same mechanism `bq` already inherits the guarantee through, which
`ARCHITECTURE.md` accepts as precedent.

**The claims can now be measured.** The recorded decision closes by saying
"None of the above was measured against a live API … no successful token was
ever obtained." Credentials are available now, and the figures above are
captures rather than reasoning.

## What did not change

**The configuration guarantee does not transfer to the SDK's own auth.**
`google-cloud-auth` supports service account, user account, metadata server,
external account, impersonated and API key credentials — the ADC family.
Nothing in it reads gcloud's named configurations, and
`application_default_credentials.json` is a sibling of `configurations/`
rather than a member of one. An SDK client authenticating itself is
machine-wide, which is exactly the defect the original decision refused.

**The dependency cost is worse, not better.** A resolution of
`google-cloud-storage` + `google-cloud-auth` alone against this workspace's
lockfile:

- 268 crates resolved, of which **171 are new** to this workspace. The decision
  measured 59 for a bare `reqwest`.
- Both `syn` 2 and `syn` 3 are present, which `bans.multiple-versions = "deny"`
  refuses, along with nine other duplicated crates.
- `ring`, `untrusted` and `subtle` carry licenses outside `deny.toml`'s
  allowlist, as the decision predicted, and `aws-lc-sys` adds a C toolchain
  (`cmake`) to the build.

This price is paid at the *first* crate, not per service: `gax`, `auth` and the
TLS stack arrive with whichever service migrates first.

## Decision

Migrate the **read paths** — every `list` the poll issues and every `describe`
behind a detail pane — to the native client libraries. Keep everything else on
`gcloud`:

| stays on gcloud | why |
| --- | --- |
| configuration discovery and activation | no API exists; it is gcloud's own local state under `~/.config/gcloud/configurations/` |
| per-configuration auth | only gcloud resolves a *named configuration* to a token; `config-helper` is the bridge |
| lifecycle mutations (start/stop/reset/failover) | operator-initiated and rare, so no measurable saving, and the confirmation modal shows the exact argv that will run. Replacing that with an RPC means displaying a command string nothing executes — a transcription presented as a capture |
| tunnels, SSH, cloud-sql-proxy | not API calls: long-lived child processes, an IAP relay gcloud implements, and a TTY handover |
| plugins | the feature is "run this operator-declared gcloud command line" |
| BigQuery via `bq` | `google-cloud-bigquery` is the one pre-1.0 crate, and `bq` already inherits the guarantee via `CLOUDSDK_ACTIVE_CONFIG_NAME` |

Authentication for the migrated paths goes through `config-helper`, cached to
the token's expiry and invalidated on every configuration switch, so a client
is always aimed by the configuration the operator selected rather than by ADC.

### The JSON tab, and a claim this record got wrong

Every detail pane carries a JSON tab rendering `raw_json` — the payload the
service returned, pretty-printed, and for Cloud SQL and Cloud Storage passed
through a redaction pass first. It is the tab an operator opens precisely when
the summary fields are not enough.

An earlier revision of this record kept every `describe` on `gcloud` and said
the generated models "implement `Deserialize` but not `Serialize`, so there is
no honest way to produce that string from a native response."

**That was wrong.** The models do implement `Serialize`. The impls are not
derived on the structs — which carry only `Clone, Default, PartialEq` — but
hand-written in a separate generated module (`src/model/serialize.rs`, and for
the storage crate `src/generated/gapic/model/serialize.rs`). The mistake was
reading the struct derives and concluding from their absence; the check that
settles it is serializing a model and looking at the output. Done that way it
emits the API's own wire format, camelCase, absent fields omitted:

```json
{ "canIpForward": false, "cpuPlatform": "Intel Cascade Lake",
  "creationTimestamp": "2026-01-02T03:04:05.000-08:00", "name": "web-01" }
```

So the detail panes migrate too, and the boundary moves: every read nuvem
issues is native except the four listed above. `raw_json` becomes
`serde_json::to_string_pretty` over the response model, with the same
redaction pass applied to the same `serde_json::Value` as before.

Two consequences worth stating plainly rather than discovering later:

- **The JSON is the model's view, not the wire bytes.** A field the generated
  model does not carry will not appear, where `gcloud` passes the REST response
  through verbatim. For clients generated from the service definition that gap
  is far smaller than the hand-written schemas being replaced, but it is not
  nothing, and it is the one thing this migration takes away.
- **Redaction is keyed by field name, so a new spelling defeats it silently.**
  The storage pass already carried two spellings because redacting only the
  JSON API's meant real `gcloud` output was never redacted. The clients
  introduce a third: `kmsKey` and `defaultKmsKey`, where the API documents
  `kmsKeyName` and `defaultKmsKeyName`. Serializing a model through the
  unwidened pass would have put key material in the JSON tab. Any future
  backend must be checked against the redaction list the same way.

Latency is the reason this is worth doing at all, and it favours the panes more
than the poll: a listing repeats on a timer nobody watches, while a `describe`
runs with an operator waiting for the pane to open. The measured gap — 1.32s –
1.56s against 0.31s – 0.47s — is felt directly here.

That figure is one path on one machine, which is all this record claimed.
Every read path has since been measured on both backends:
`docs/performance-read-paths.md` reports the whole surface at a little over 2x,
the detail panes at 2.8x – 8.5x, and the paths where the two backends are level.
It also lists the seven divergences the comparison uncovered.

`deny.toml` is widened deliberately and in this commit's own diff: the license
allowlist gains the entries the TLS stack needs, and the duplicate-version ban
gains explicit skips. Policy widened knowingly for a named reason is different
from policy that drifted.

## Consequences

- The poll stops spawning interpreters. On the measured day that is 1,064 of
  1,183 invocations removed.
- Responses become types generated from the service definition. The defect this
  repository just fixed — a hand-written schema drifting from what `gcloud`
  actually emits — cannot recur on a migrated path.
- Real pagination becomes available. `gcloud storage objects list` emits no
  page token at all, which is why the object browser has none.
- Cancellation becomes a dropped future rather than a killed process group.
- The build gains 171 crates and a C toolchain, and two policy gates are
  widened.
- Two code paths now reach GCP. A read and its neighbouring mutation no longer
  travel the same way, and the argv preview covers only the mutation.
- Every argument the old call carried has to be carried again, and seven were
  not. `docs/performance-read-paths.md` lists them: a second call that filled a
  column, a page token, an order, a policy version, a page size, two
  vocabularies and a filter. The row cap `--limit=500` was among them on eight
  listings, and `returnPartialSuccess` -- without which one unreachable zone
  fails an aggregated listing that the CLI backend would have answered.
- Server-side filter pushdown does not travel either. Regex mode never needed
  it for correctness — the client re-narrows — but label mode narrows nothing
  on the client, so a label-mode read is routed through `gcloud` even where a
  native client exists (`list_filter::label_mode_needs_gcloud`). The native
  path pushes no filter of its own until the per-API filter grammars are
  measured the way gcloud's was.
- The hand-written `describe` schemas survive, but as the demo-mode and
  fallback backend rather than as the production path: `services/resources/`
  keeps its CLI implementations because `--demo` is built on them. The drift
  risk that motivated this decision is removed from the live path and retained
  in the fixtures; a listing and its `describe` no longer share a parser,
  so a field the API renames is now caught in one place instead of two.

## What would reverse this

A measurement showing the poll cadence is the real cost. Raising
`poll_interval_secs` from 5 to 30 would turn the measured 847 bucket listings
into roughly 140, with a one-line change and no new dependencies, and the poll
currently re-lists *buckets* while the operator is drilled into a bucket, which
is work nobody asked for.

That experiment has since been run, and it does not reverse this.
`docs/performance-read-paths.md` measures a poll tick as processor time on both
backends: 376 - 392 ms on the CLI backend, almost all of it starting an
interpreter, against about 1 ms natively. At the default cadence that is
roughly four and a half CPU minutes an hour against a little over half a
second. Lengthening the interval would have reclaimed most of the former, which
is what made the question worth asking; against the latter it reclaims nothing
worth the staleness it costs. The cadence stays where it is, and the
outstanding item is closed rather than deferred.

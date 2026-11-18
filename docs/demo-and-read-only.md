# Demo mode and read-only mode

Two ways to run nuvem without touching a real project, or without being able to
change one.

## Manual Testing

Install the Google Cloud CLI and make sure `gcloud` is on your `PATH`.

```bash
gcloud config configurations list
cargo run
```

To start on a specific native gcloud configuration, pass its name:

```bash
cargo run -- --configuration prod
```

The TUI loads native gcloud configurations at startup. If you pass
`--configuration`, it asks `gcloud` to activate that configuration first, then
reloads the native configuration list.

Pass `--poll-interval <secs>` to control how often the active resource service
is refreshed in the background while the Resources workspace is focused — the
same tick also drives the Cloud Logging tail while the Logging workspace is
focused (default: `5`). Set `--poll-interval 0` to disable background refresh
entirely; manual `r` and auto-load on focus/scope change still work.

Pass `--k9s-binary <path>` and `--kubectl-binary <path>` to point at the `k9s`
and `kubectl` executables when they are not on `PATH` (defaults: `k9s` and
`kubectl`). These are used by the `Open selected GKE cluster in k9s` workflow.

Pass `--cloud-sql-proxy-binary <path>` to point at the Cloud SQL Auth Proxy
executable when it is not on `PATH` (default: `cloud-sql-proxy`). It is used by
the Cloud SQL tunnel described under [Tunnels](features.md#tunnels).

Pass `--bq-binary <path>` to point at the `bq` binary when it is not on `PATH`
(default: `bq`). The BigQuery datasets table shells to `bq` rather than gcloud,
because BigQuery has no GA gcloud surface: `gcloud alpha bq` is the only gcloud
route to it, and nuvem runs no alpha command. `bq` ships with the same Cloud
SDK, so the default is usually right. Browsing runs three levels deep, and two
listings cover the first two: one `bq ls -d` for the project's datasets, and
one `bq ls <dataset>` for the tables inside the dataset you press `Enter` on.
Neither has a detail pane. `bq show` covers the third, in two unrelated roles.
One per listed table, issued once a tables listing lands, fills that table's
ROWS, BYTES and LAST MODIFIED columns and reads nothing else from the answer.
One more, issued by the `Enter` that opens a table row, is read for its schema
alone and is the whole content of the column list that opens. Every one of
these calls is described with the other services below.

### Demo mode

```bash
cargo run -- --demo
```

Runs the TUI offline against embedded JSON fixtures — the same JSON the test
suite uses. Demo reads them through the same native client libraries
(`services/sdk/`) a live session reads a real project through: the clients are
built from stubs that serve the fixtures, so they hold no credentials and open
no transport, and the code that lists, pages, filters and paints is the code
that runs against Google. What differs is where the bytes come from, not which
path reads them, so a parsing bug is now one demo can reproduce rather than one
it can hide. Pagination is not: no demo stub sets a next-page token, so a demo
walk exercises page one of every paginator and never the loop around it.

Some calls have no native client behind them on either backend and still
travel as a command: BigQuery (which goes to `bq`, for the reason given above),
FinOps budgets — Cloud Billing publishes no Rust client for them, though the
recommender half beside them does read natively — gcloud's own configuration
and auth state, and label-filtered listings, whose `labels.k = "v"` expression
only the pushed gcloud filter honours. In demo those are answered by the
fixture command runner instead of by `gcloud`.

Every workspace
(configurations, Compute Engine, GKE, Cloud Run, Cloud Functions, Cloud SQL,
Cloud Storage, Secret Manager, persistent disks, Pub/Sub, BigQuery, VPC, IAM,
Cloud Logging, FinOps) renders with realistic data. Read commands
return fixtures; mutating commands (activate a config, start/stop an instance,
...) succeed as no-ops and never touch a real backend or your local gcloud
configurations.
Intended for UI iteration, demo recordings, docs screenshots, and reproducible
bug reports.

FinOps takes one step first, in demo exactly as it does against a live project:
each recommender call is scoped to one location — a segment of the `parent` it
lists under — and the locations come from rows nuvem has already listed, so open
Resources on GCE instances and on persistent disks before opening FinOps. Until
then its cards read `not probed` and say why, which is the real behaviour and
not a gap in the fixtures.

### Read-only mode

```bash
cargo run -- --read-only
```

Refuses any command that changes cloud state, or that hands you an
interactive session which can. That is the rule; the current list is:

- starting, stopping, resetting, suspending, or resuming a Compute Engine
  instance;
- starting, stopping, restarting, or failing over a Cloud SQL instance;
- SSHing to an instance;
- opening a GKE cluster in k9s — k9s is a full read-write Kubernetes UI, and
  the launcher runs `get-credentials` when no kube context is present;
- every plugin (see *Plugins* below) nuvem cannot prove only reads: a plugin's
  command line comes from your `plugins.toml` rather than from nuvem, so the
  mode runs one only when its declared `args` open with a gcloud command nuvem
  recognises as read-only, and refuses the rest — a reading command nuvem does
  not happen to name included. In read-only, choosing a refused plugin reports
  the refusal instead of opening its prompt or its confirmation.

The refusal applies at every point a mutation can still be ahead of the mode:
submitting from the palette, confirming a modal that was already open, and —
for k9s, whose confirmation only starts `get-credentials` — the interactive
launch itself, once those credentials come back. Turning the mode on stops an
in-flight k9s launch from reaching the cluster.

Read-only is a promise about **cloud resources**, not a claim that nuvem
changes nothing at all. Three things it deliberately still does, because none
of them touch a cloud resource: activating a gcloud configuration (local
gcloud state), opening a tunnel (a process on this machine), and copying a
value to the clipboard. Read commands — listing, detail views, logs — are
unaffected. Treat the exceptions as *these three*, not as "anything local":
if a new command changes cloud state or opens a session that can, it is
blocked.

Blocked commands stay in the command palette, shown and selectable, marked
with a trailing `— read-only` and rendered dimmed; submitting one reports why
instead of doing nothing. Turn it on with either `--read-only` or
`[general] read_only` in the config file below — either one is enough, and
neither can force the mode off, so the flag's absence never overrides a file
that turns it on. It is also sticky for the life of the process: reloading a
configuration can turn read-only on, never off.

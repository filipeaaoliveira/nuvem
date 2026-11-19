# nuvem Architecture And TUI Practices

This document captures the structure and engineering practices that `nuvem`
follows: a Rust TUI for Google Cloud, inspired by `k9s`, built as a layered
application in which a typed domain sits beneath services, app reducers, input
handling, UI rendering, and the runtime, with dependencies pointing one way.

The goal is to keep the project professional from the first commit: clear
module ownership, typed state transitions, service-isolated side effects,
render-only UI code, and a test strategy that protects the tricky parts of an
interactive terminal application.

## Product Direction

`nuvem` should be a single-process terminal application for inspecting and
operating Google Cloud resources.

The app should feel closer to `k9s` than to a static dashboard:

- fast keyboard-first navigation
- resource lists with filtering, sorting, and selection
- detail panes for the selected resource
- logs/events/activity views where relevant
- command workflows for safe operations
- explicit gcloud configuration, project, account, region, and zone awareness
- degraded states when auth, API, quota, or local tools are unavailable

The architecture is built so that adding a service does not require rewriting
the app shell. More than a dozen Google Cloud services are wired in today, and
each one arrives as a domain type and a service module, with typed projections
added only where a view needs derived display data; the reducer, input, and
render layers consume the new resource as data.

Native gcloud configurations are the source of truth for context. `nuvem`
should list, activate, and create configurations through `gcloud config
configurations ...` instead of inventing an app-only profile system. The active
configuration should behave like the current Kubernetes context in `k9s`: it is
always visible, switching it reloads the visible world, and actions must make
the target context obvious before they run.

## Architecture Principles

The important pattern is dependency direction:

1. domain types define shared concepts
2. services observe or mutate external systems
3. services emit typed events or effect results
4. app reducers own visible state and workflow behavior
5. input maps keys to semantic actions
6. UI renders state without mutating behavior
7. runtime owns terminal lifecycle, async tasks, cancellation, and effect dispatch

For `nuvem`, the equivalent rule is:

- `gcloud`, Google APIs, auth files, caches, and filesystem state belong in services
- resource identity, gcloud configuration identity, project identity, status,
  and typed rows belong in domain
- filtering, selection, modal state, and workspace state belong in app reducers
- rendering belongs in UI modules only
- keyboard interpretation belongs in input modules only

Avoid direct calls to `gcloud`, network clients, filesystem writes, or mutable app
logic from render code.

## Architecture Choice

Use a layered architecture as the default.

Other approaches are possible, but they are weaker fits for this project:

- A single-crate, module-only app is faster to start, but domain and runtime
  concerns will blur quickly once multiple Google Cloud services, background
  refresh, command workflows, and tests are added.
- A plugin-heavy architecture may become useful later, but it adds complexity
  before the core resource model, command model, and UI conventions are proven.
- A client/server split is unnecessary for the initial scope. `nuvem` should
  start as one terminal process unless a concrete remote or multi-user workflow
  requires a daemon later.

The recommended approach is therefore a single-process TUI with a workspace
layout, a separate domain crate, reducer/effect app flow, service-isolated
Google Cloud integration, and render-only UI modules.

## Repository Layout

The repository is a Rust workspace: the `nuvem` binary crate at the root and the
`nuvem-domain` library crate under `crates/`. The module tree follows the
layering above, with the domain named around GCP resources.

The tree below is checked in both directions by
`tests/architecture_layout_test.rs`: every `.rs` file under `src/` and
`crates/nuvem-domain/src/` is listed here, and every entry listed here exists on
disk. Adding a module under either root means adding a line here in the same
commit.

<!-- architecture-layout:verified:begin -->

**Verified module tree.** Machine-checked against the source tree by
`tests/architecture_layout_test.rs`.

```text
# VERIFIED - checked in both directions against the filesystem by tests/architecture_layout_test.rs
crates/nuvem-domain/src/
  gcp/
    argv.rs
    configuration.rs
    diagnostics.rs
    filter.rs
    finops.rs
    k9s.rs
    location.rs
    logging.rs
    matcher.rs
    mod.rs
    money.rs
    plugin_output.rs
    project.rs
    resource.rs
    resource_call.rs
    text.rs
    text_filter.rs
    tunnel.rs
  lib.rs
src/
  app/
    action.rs
    effect.rs
    mod.rs
    projection/
      binding.rs
      columns.rs
      configurations.rs
      detail_body.rs
      finops.rs
      frame.rs
      home.rs
      logging.rs
      mod.rs
      table.rs
      tunnels.rs
    reducer/
      command.rs
      configurations.rs
      finops.rs
      jumps.rs
      logging.rs
      mod.rs
      navigation.rs
      navigator.rs
      resources.rs
      root.rs
      tunnels.rs
    service_capabilities.rs
    service_registry.rs
    state/
      command.rs
      configurations.rs
      diagnostics.rs
      finops.rs
      health.rs
      jumps.rs
      logging.rs
      mod.rs
      navigation.rs
      navigator.rs
      operations.rs
      plugin_output.rs
      plugins.rs
      remembered_view.rs
      resources/
        batch.rs
        bigquery.rs
        cloudrun.rs
        cloudsql.rs
        compute.rs
        disks.rs
        drill.rs
        filter_cache.rs
        firewall.rs
        functions.rs
        gke.rs
        iam.rs
        mod.rs
        pubsub.rs
        secrets.rs
        storage.rs
        vpc.rs
        vpc_subnets.rs
      root.rs
      system.rs
      tunnels.rs
  config/
    aliases.rs
    columns.rs
    favorites.rs
    file.rs
    hotkeys.rs
    jumps.rs
    mod.rs
    plugins.rs
    problems.rs
    read.rs
    state.rs
    theme.rs
  exit.rs
  input/
    keymap.rs
    mod.rs
  lib.rs
  main.rs
  services/
    demo/
      mod.rs
      sdk/
        cloud_run.rs
        cloudsql.rs
        compute.rs
        discovery.rs
        disks.rs
        finops.rs
        functions.rs
        gke.rs
        iam.rs
        logging.rs
        mod.rs
        pubsub.rs
        secrets.rs
        storage.rs
        vpc.rs
    gcloud/
      cli.rs
      configurations.rs
      context_credentials.rs
      diagnostics.rs
      discovery.rs
      mod.rs
    mod.rs
    resources/
      bigquery.rs
      cloud_run.rs
      cloudsql.rs
      compute.rs
      disks.rs
      finops.rs
      functions.rs
      gke.rs
      iam.rs
      list_filter.rs
      logging.rs
      mod.rs
      pubsub.rs
      secrets.rs
      storage.rs
      vpc.rs
    runtime/
      clipboard.rs
      interactive.rs
      k9s.rs
      log_export.rs
      logging.rs
      mod.rs
      remembered_view_store.rs
      session.rs
      tail.rs
      terminal.rs
      tunnel/
        mod.rs
        port.rs
    sdk/
      cloud_run.rs
      cloudsql.rs
      compute.rs
      discovery.rs
      disks.rs
      finops.rs
      functions.rs
      gke.rs
      iam.rs
      logging.rs
      mod.rs
      pubsub.rs
      secrets.rs
      storage.rs
      vpc.rs
  ui/
    configurations/
      mod.rs
    finops/
      mod.rs
    home/
      mod.rs
    logging/
      mod.rs
    mod.rs
    resources/
      mod.rs
    service_navigator.rs
    shared/
      components.rs
      frame.rs
      help.rs
      layout.rs
      mod.rs
      picker.rs
      table.rs
    theme.rs
    tunnels/
      mod.rs
```

<!-- architecture-layout:verified:end -->

**Supporting files.** Illustrative, not machine-checked. This block names the
files a newcomer looks for first and does not claim to be complete.

```text
# ILLUSTRATIVE - not machine-checked, not exhaustive.
nuvem/
  Cargo.toml           workspace and binary crate manifest
  Cargo.lock
  rust-toolchain.toml
  deny.toml
  README.md
  ARCHITECTURE.md
  crates/
    nuvem-domain/
      Cargo.toml       domain crate manifest
  docs/                specs, plans, and reference notes
  scripts/             developer quality gates (see CONTRIBUTING.md)
  tests/               integration tests and fixtures, one file per feature area
```

## Workspace And Crate Boundaries

### `crates/nuvem-domain`

The domain crate owns stable concepts and must stay free of runtime concerns.

It may contain:

- gcloud configuration identifiers and summaries
- project identifiers
- account identifiers
- resource identifiers
- service names
- locations and regions
- resource status enums
- health and degraded-state types
- shared time/value helpers

It must not depend on:

- `ratatui`
- `crossterm`
- `tokio`
- `clap`
- filesystem APIs
- network clients
- `gcloud` process execution

If a type is needed by reducers, services, tests, and UI projections, it is a
candidate for the domain crate. If a type mentions terminal layout, subprocesses,
HTTP, credentials, paths, or CLI flags, keep it out of the domain crate.

### `src/main.rs`

Keep `main.rs` thin:

```rust
use clap::Parser;
use nuvem::{run, Cli, Command, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(Command::Info) = cli.command {
        print!("{}", nuvem::config::run_info(&cli));
        return Ok(());
    }

    let outcome = Config::resolve(&cli);
    for warning in outcome.problems.messages() {
        eprintln!("warning: {warning}");
    }

    run(outcome, cli).await
}
```

Do not put startup orchestration, terminal setup, logging setup, or app behavior
in `main.rs`.

### `src/config/`

The `config/` module owns CLI arguments and the layered resolution of runtime
settings: built-in defaults, folded over the optional TOML config file, folded
over CLI flags (`ResolvedConfig::resolve`). It also owns the `nuvem info`
subcommand and the config file schema (themes, hotkeys, aliases, favorite
picker scopes, per-service table columns and default sort, related-resource
jump rules, general settings), and the separate plugin file's schema
(user-declared row-scoped commands), whose user-facing contract is documented
in `docs/plugins/`.

It also owns the other direction of persistence: `config/state.rs` stores state
the app learns for itself in a `state.toml` beside the config file in use, so
features that need memory across sessions never rewrite the TOML the user
hand-edits. Its module docs own the durability contract — file separation,
atomic replace, versioned envelope, last-write-wins between instances, and why
`--read-only` does not suppress those writes — and are the place to read before
building on it.

`config/plugins.rs` owns the third direction: a file that declares commands. It
is deliberately separate from `config.toml` and deliberately not resolved
relative to `--config`, which inverts the rule `config/state.rs` uses. Locality
is right for a file the app writes for itself and wrong for one that declares
commands; read that module's docs before changing where it is found.

`config/plugins.rs` stops at "this declaration is well-formed".
`app/state/plugins.rs` answers the next question — "can *this row* fill *these*
templates" — and it is the only place that composes a plugin argv:

- each `args` element is rendered independently against the selected row's
  fields and against whatever values the operator supplied at the prompt, the
  same `Template` and the same per-service vocabulary the jump rules use, and
  becomes exactly one argv element; there is no splitting step anywhere, so
  there is no shell parser under another name. The two lookups stay separate
  rather than merging into one map, so a row field cannot silently satisfy a
  template that asked for a value nobody typed;
- `--configuration <active configuration>` is prepended rather than declared,
  because every built-in argv carries it and a plugin without it would name
  whatever configuration the ambient gcloud state has rather than the one nuvem
  is showing;
- two rendered results are refused: a *substituted* value that begins with `-`,
  which gcloud would read as a flag (an operator's own literal `--quiet` is a
  literal and unaffected; a value **typed at the prompt** is substituted and so
  is policed by this rule too), and any element carrying a control character, which
  the confirmation line cannot show faithfully. There is deliberately no length
  bound — the modal already refuses a command line that does not fit, so an
  over-long element degrades to the existing small-terminal path rather than to
  a truncated line the operator misreads;
- `applicable` lists the plugins that resolve, in file order, and `diagnostics`
  composes the sentence explaining an empty list against the row under the
  cursor — "needs *this* field, which this instance has no value for". Both go
  through one resolution pass, so the palette and its explanation can never
  disagree about which plugins resolved. `diagnostics` still has no UI caller.
  It was written for the keybinding, on the reasoning that pressing a key and
  getting nothing is where an explanation is owed — but the keyed press
  composes its refusal through `keyed`, which resolves the one plugin
  the operator named and can therefore say which field *that* plugin needed,
  where `diagnostics` explains an empty palette across all of them. The two
  answer different questions, so the binding landing did not give this one a
  caller.

`GcloudCommand` carries a program (`CommandProgram::Gcloud` or
`CommandProgram::Bq`, defaulted to gcloud by `GcloudCommand::new`) and a list of
environment entries alongside its argv, and the session's gcloud runner holds
one path per program, taken from config's CLI-only `gcloud_binary` and
`bq_binary`. (The type has both slots, not the guarantee: the session's kubectl
runner puts the kubectl path in the `gcloud` slot and never uses the other one.)
BigQuery is why: it has no GA gcloud surface, so the browsing this plumbing was
built for reaches it through `bq` rather than through another gcloud argv. The
environment list exists for the same reason. `bq` has no `--configuration` flag,
so where every other service prefixes `--configuration <active configuration>`
to its argv, a `bq` command carries the active configuration as
`CLOUDSDK_ACTIVE_CONFIG_NAME` in the child environment instead. Both halves are
load-bearing now: the BigQuery commands are the only production ones that use
them — `bq ls -d` for the project's datasets, `bq ls <dataset>` for the tables
inside the one `Enter` drilled into, a `bq show` per listed table for that
row's `ROWS`, `BYTES` and `LAST MODIFIED`, and one more `bq show` for the
column schema of the table a second `Enter` opened. The three lists live behind
a single `ResourceServiceKind::BigQueryDatasets`, the shape Cloud Storage's
object browser set, and none of them has a detail pane: the schema is a third
list rather than a pane, and the enrichment `show` still reads none of the
schema its answer carries — the drill re-issues its own.

`CommandProgram` stays closed over binaries the Cloud SDK ships. What it never
grew is a hand-assembled HTTP client, and the reasoning for that refusal is
worth keeping, because it is what shaped the read path that did arrive.

The objection was never that a request could not be aimed correctly. It was
that nuvem would be the thing aiming it. A call this application assembles
itself holds a URL and a header set it built, and every property the active
configuration carries beyond account-and-project is dropped without a sound:
`api_endpoint_overrides`, whose host defaults the SDK's own `properties.py`
reads from the configuration's `core/universe_domain`; that `universe_domain`
itself; `billing/quota_project`; and the proxy section. A hardcoded
`https://<service>.googleapis.com/v1/...` ignores all four. That is the same
defect a wrong-project reading is, relocated from the project to the endpoint
and no less silent for having moved. `bq` is the standing counter-example of a
second mechanism that keeps the guarantee: nuvem hands it a configuration
*name* and the SDK resolves everything that name implies, re-deriving nothing.

The token was the second objection. `gcloud auth print-access-token` carries
`--configuration`, but the SDK's own help says the token it prints is bound to
the gcloud CLI's client ID and directs code that wants to call an API to
Application Default Credentials instead — and ADC carries no configuration
guarantee at all, since `application_default_credentials.json` sits at the root
of the gcloud config directory as a sibling of `configurations/` rather than
inside any one of them. It is machine-wide and has no idea which configuration
is active. Cost compounds that: every gcloud invocation pays a full Python
interpreter start before any data moves, so a token acquired per poll would
spend a fifth of the 5s `poll_interval_secs` default before the request began.
Caching to the token's own lifetime is therefore mandatory, and that cache is
new state which has to be invalidated on every context switch or it becomes the
wrong-project reading with a longer memory.

Generated client libraries answer the endpoint objection, and
`docs/decisions/0001-native-sdk-for-read-paths.md` records the decision to move
the read paths onto them. A generated client resolves its own endpoint and
universe domain rather than leaving that to a caller holding a string. The
token objection is answered by not using ADC:
`src/services/gcloud/context_credentials.rs` implements the SDK's
`CredentialsProvider` over `gcloud config config-helper` for the active
configuration, which returns a token alongside the configuration's full
property set. The token is held in memory only, handed out until one minute
before its stated expiry, and keyed by configuration name, so a context switch
cannot be served a token minted for the configuration the operator just left;
the `Authorization` header is marked sensitive so it stays out of any `Debug`
rendering reachable from tracing.

What the native path honours today is narrower than what gcloud honours, and
the difference is worth stating rather than discovering. `core/universe_domain`
is taken from the `config-helper` payload and surfaced through the provider, so
a client resolves its hostname inside the universe the configuration names.
`api_endpoint_overrides`, `billing/quota_project` and the `proxy/*` section are
*not* applied on the native path, and that now includes the GCE and Cloud SQL
lifecycle actions, which go through the client libraries. Every call that still
goes through gcloud — every tunnel, every plugin, configuration activation and
edits, BigQuery through `bq`, and the reads that still fall back to it —
continues to honour all of them, because gcloud resolves them itself. Those
paths keep the argv boundary for their own reasons — their confirmation shows
the exact command line that will run, and a tunnel, an SSH session or a
cloud-sql-proxy is a long-lived child process rather than a request — and
the property coverage is a second benefit of that boundary rather than its
cause. A deployment that depends on those properties gets them on the argv path
and not on the native one.

The dependency price the earlier reasoning predicted was real, and larger than
its estimate: 171 crates new to this workspace, and a C toolchain by way of
`aws-lc-sys`. `deny.toml`'s license allowlist gains `ISC`, `BSD-3-Clause` and
`CDLA-Permissive-2.0`, each annotated with the crates that need it, and the
duplicate-version gate gains a list of named skips for the copies the generated
crates bring alongside ones the workspace already had. Both widenings are
deliberate and recorded in the decision record rather than drifted into. The
skips are listed crate by crate rather than by tree, so
`bans.multiple-versions = "deny"` still fails on a duplicate nobody signed off.
The whole price is paid at the first crate rather than per service: the
transport, the auth crate and the TLS stack arrive with whichever service moves
first.

The numbers that reasoning lacked were taken later and live in the same record:
1.32–1.56s for a `gcloud` listing against 0.31–0.47s for the same data over a
cached token, with roughly a second of every gcloud call being interpreter
startup. The refusal was reasoned from the installed SDK's source; the decision
that replaced it is measured.

What is still out of reach is anything that is a rate, a utilization or an
aggregate rather than a listing or a single resource. nuvem reads no metric
time series, so a count that saturates at a `--limit` must never be labelled a
rate.

None of that moves the argv boundary drawn for plugins, and plugins still
inherit it: a declaration names arguments to `gcloud`, never the binary. An
earlier proposal named `gcloud/bq/gsutil` as something a declaration could name
and no declaration ever grew it; the plugin work closed on gcloud alone,
deliberately.
The runner's `bq` program does not reopen that: only nuvem's own
services can select it, never a declaration. The reason is `declared_effect`: a
plugin is `Reads` only when its literal prefix matches `READING_COMMANDS`, a
fixed list of *gcloud* paths, and `--read-only` refuses everything else on the
grounds that nuvem never vouched for it. A `bq` or `gsutil` plugin could match
nothing there, so it would be permanently refused under the one mode that
matters, and making it work would mean one allowlist per program — the same
maintenance surface `DeclaredEffect` already declines to double for a second
gcloud table. The secondary reason is the trust boundary `plugins.toml` draws:
arguments to one binary nuvem already shells to is a different promise from any
binary on the operator's `PATH`. Reopening this needs its own design pass,
starting with what `--read-only` can mean over an unclassifiable program.

The plugin set lives in `RootState::plugins`, installed once at startup and
never swapped: `PluginId` is an index into it, so a live reload would silently
repoint every id a palette row or modal is already holding. `reload_config`
therefore reports `restart required for: plugins` when the file has changed,
alongside the other restart-required fields.

`CommandDescriptor::Plugin(PluginId)` puts the resolvable plugins in the
command palette for whatever service is in view, labelled by the plugin's own
`label` template rendered against the row. Every plugin answers `is_mutating()`
with `true`, but `--read-only` no longer asks it: it asks
`plugins::plugin_blocked_by_read_only`, the one place the mode's rule about a
plugin lives, consulted by the palette's `Submit`, by the keyed press and by
the palette's dimming alike — three inlined copies of the same condition is how
the palette comes to dim a row the reducer runs. The palette's two sites ask
through `command::command_blocked_by_read_only`, which owns the whole
condition — the plugin arm and the `read_only && is_mutating()` arm both — so
the split between the two questions is written once as well. The keyed press
has no gate of its own, so both doors route through one
`open_plugin`, which carries the refusal *ahead of* deciding whether a prompt
or a confirmation opens. Refusing any later would ask a read-only operator to
fill in every value before telling them no.

`--read-only`'s answer for a plugin comes from `declared_effect`
(`app::state::plugins`), which matches the plugin's declared `args` templates
against an anchored table of gcloud command paths. Anchored full paths rather
than a verb scan, because `["compute", "instances", "delete", "list"]` names an
instance `list`; matching positionally cannot mistake a positional for a verb.
Templates rather than the composed argv, because a plugin may take its verb from
an input, and because the refusal has to land ahead of the prompt. Everything
the table does not name is `Unproven` and refused, so an omission costs a plugin
the operator has to run without the flag rather than letting a mutation
through.

Selecting a plugin whose templates name declared `[[plugins.inputs]]` opens
`PluginPrompt` first: one field per input the plugin's own templates
name, in declaration order, pre-filled from each declared default. It holds a
snapshot of the row's fields taken when it opened — the modal owns the keyboard,
so the selection cannot move under it, and snapshotting is what makes "the argv
the operator approves is the argv that runs" exact rather than incidental. A
`dropdown` field holds an index into its declared choices rather than free text,
so a value the plugin never offered is unrepresentable. `Enter` is inert while
any field holds something it cannot send and while the modal does not fit the
terminal, and `Esc` cancels the whole gesture without reaching the confirmation.
Every declared type is collected: a text field carries its own
`TextRule`, so a `number` refuses what `default_for` refuses a non-finite float
default for and an `integer` refuses what it refuses a fractional one for —
`"inf".parse::<f64>()` succeeds, and the prompt is the door with no config
review behind it. The rule lives on the draft rather than beside it, so a
dropdown carrying a numeric rule is unrepresentable. What the operator types
reaches the argv verbatim, unlike a declared default, which is stored
canonically: the confirmation shows the characters that will run. A `bool` is a two-state
`Draft::Toggle` rather than a two-item `Choice`, so a bool with a third choice
is unrepresentable; it sends the text `true`/`false` — the representation
`default_for` already normalises a declared bool default into — rather than a
flag's presence, which would make argv length depend on an answer the
confirmation has to show exactly. With no declared default it starts `false`,
the answer that adds nothing. No declared kind withholds a plugin any more, so
`Unoffered` lost the `NeedsInput` variant that said one did. Submitting composes the argv and opens the
confirmation — it starts nothing, so the confirmation remains the only door to
a process.

That confirmation, `PluginConfirmation`, is a modal of its own with a `Mode`
of its own, carrying the plugin's label and its **argv** and rendering the
command line from the bytes that will run — not a rendered line stored beside
them, which is what `ResourceCommandPreview` holds precisely because it never
executes. This one does, so what the operator reads and what reaches the
process are the same object. Its `Enter` is a confirmation rather than a copy.
It shared that preview and `CopyPendingCommand` while a plugin was a command to
copy, and no longer does — the preview is the SSH command line's again. A
`dangerous` plugin's confirmation carries `BatchAck::CountRequired` with a
target of 1, so `Enter` is unreachable until the operator types it; the keymap
gates `Enter` on that *and* on the modal being renderable, so an argv the
terminal is too small to show cannot be confirmed unread.

`ConfirmPlugin` opens one operations-log entry and emits `Effect::RunPlugin`,
which the session hands to a blocking thread rather than running on the
reactor: a plugin's argv is user-written and can take as long as it likes.
The result settles that entry by id, unconditionally, because a run outlives
the context it was confirmed under; the status line is scoped, and by two
guards rather than one — the context the run was confirmed under *and*
`operation_generation` — because a plugin can be outlived both by a
configuration switch and by whatever the operator started next in the same
context, and a slow plugin must not stamp itself over the refresh they are
waiting on.
"As long as it likes" is why `Ctrl-x` exists: the session keeps one
handle per in-flight run and `Effect::CancelRunningPlugins` cancels every one
of them, because the operations-log entry a run leaves on Home's read-only
overview card cannot be selected. Each handle carries the run's
`operation_id` and its own `BatchTargetClaim` — the same claim type the batch
path uses, because the race is the same one, a worker on a `spawn_blocking`
thread against a cancellation on the reactor — so a cancelled entry settles
`Cancelled` or `Unknown` by exactly the rule described under batch
cancellation below, and the worker that loses the compare-exchange issues
nothing. `EffectResult::PluginRunCancelled` is distinct from
`PluginRunFailed` because the latter records a diagnostic unconditionally, and
a cancellation must not park a fabricated gcloud failure under `Y`; that arm
settles no entry, since the cancellation already settled it by what the claim
proved.

The other scoped consequence of a settled run is the reload it triggers,
which is what keeps the table from disagreeing with the log entry the
run just closed. All three arms carry it — succeeded, failed and cancelled,
since a run that died partway is the one most likely to have half-applied a
change — under a guard that requires the `Resources` workspace *and* a result
context that is still the active one, the same pair `poll_active_resource` uses;
a run settling from anywhere else reloads nothing. The guard does not consult
`is_mutating()`, which answers `true` for every plugin unconditionally and so
would make this either a no-op or unconditional: the reason to reload is that a
`gcloud` child ran at all, which holds for a `describe` too. Narrowing by verb
was decided against here rather than left open: reconciling the screen
after any gcloud run is worth one list call even when the run only read, and the
in-place reload below costs a `describe` nothing beyond that call. `--read-only`
does consult a command-path classifier, because being wrong there breaks a
promise rather than costing a request. The reload is in place, delegating to
`ResourcesAction::Refresh` — the one definition of "reload the active service",
covering every `ResourceServiceKind` and carrying that service's own filter —
rather than the lifecycle arms' `mark_active_stale_and_clear`, which is a
teardown: it clears rows, resets the cursor, drops every mark and closes the
detail pane, the right price for a known mutation and the wrong one for a
`describe`. One effect is subtracted from what `Refresh` returns:
`LoadStorageObjectPage`, because `Refresh` re-requests the Cloud Storage object
page with `page_token: None` and a non-append first page replaces the
accumulated pages wholesale. That reset is a fair price for a deliberate `r` and
not for a reload nobody pressed a key for, and the background poll never emitted
it either.

`CommandRunner` has three spawn methods and one body behind them
(`ProcessCommandRunner::run_bounded`). All three spawn the child into a process
group of its own, poll for its exit without reaping it, and drain the pipes on
reader threads rather than waiting on EOF — the EOF wait is what a leaked
non-CLOEXEC pipe in a concurrent spawn can withhold indefinitely. All three also
give the child a null stdin, because nuvem holds the terminal in raw mode and
an inherited stdin is the operator's keyboard — including for a plugin's argv,
which is arbitrary and operator-declared: a plugin cannot prompt while nuvem
owns the terminal, so an inherited stdin only let it steal the keystrokes nuvem
was reading. What differs is only who is
allowed to end the call. `run` carries a ceiling and no cancellation, because
it has no owner to cancel it; `run_with_cancellation` carries a cancellation
token and no ceiling, because a plugin command may legitimately run for minutes
and there is an operator watching it; `run_bounded_with_cancellation` carries
both, because a lifecycle mutation needs the ceiling and is also the one call
shutdown has to be able to walk away from (below). A call ended any of
those ways kills the child's whole group, reaps it, drains and discards whatever
the child had already written, and refuses — no ended-call variant has anywhere
to carry output.

The ceiling `run` enforces belongs to the command, not to the runner. A read
— every listing, `describe` and `bq show` — is held to the
two minute ceiling `RUN_TIMEOUT` sets, argued from read latency. The Cloud SQL
and GCE lifecycle mutations are not reads: their argv carries no `--async`, so
gcloud blocks until GCP finishes the operation, and a failover or a large
instance's stop runs past that ceiling routinely. They declare
`LIFECYCLE_TIMEOUT` instead, and a call that hits it says the operation may
still be running server-side — killing the CLI ends nuvem's view of an
operation, never the operation.

Thirty minutes is the right ceiling for the operation and the wrong one for a
shutdown, and the two are separated. A lifecycle action — batched or
single-row — runs on a `spawn_blocking` thread, and `Runtime::drop` blocks on
every started one, so pressing `q` with a batch of stops in flight restored the
terminal, returned from `run()`, and then hung with nothing on screen to explain
it; `TerminationSignals::install` had registered tokio handlers for SIGTERM and
SIGHUP by then, so only `kill -9` got out. Each action now holds a shutdown
token, flipped once on the way out by
`RuntimeSession::cancel_lifecycle_actions_for_shutdown` beside the plugin one.
It is a distinct token from the batch's own: `batch_cancellation` is scoped to
one batch and is cancelled whenever a context switch or a supersession detaches
it, where an already-issued command is deliberately left alone and reported
`Unknown`. Cancelling at shutdown ends the wait and not the operation,
so `CommandError::Cancelled` carries the command's ceiling and repeats the same
caveat a lifecycle timeout does.

`run` was `Command::output()` until it was replaced — no timeout, no kill, no
cancellation, and no process group. That was survivable while four concurrent
`run` calls meant an operator had deliberately started a batch action, and
stopped being so when BigQuery made a fan-out of up to `-n 500` of them the
routine consequence of pressing `Enter` on a dataset.

`docs/plugins/`' promise that nothing in the plugin *schema* can run a shell
command survives this: `args` is an argv, a plugin's program is always
`gcloud`, and `CommandRunner` never goes through a shell. The plugin prefix key
(`,`) reaches a plugin without the palette and changes none of that: it
resolves the keyed plugin into the same `Offer` the palette produces and hands
it to the same confirmation, so the schema's promise is enforced in one place
regardless of which key opened the modal.

Recommended initial responsibilities:

- optional starting gcloud configuration override
- poll interval
- log file path
- cache directory
- optional `gcloud` binary path
- optional credentials/config root override
- API/CLI mode selection if both are supported later

Prefer convention over heavy configuration early. Add config only when there is
a concrete workflow that needs it.

Do not persist a separate app-level profile model at first. If users need named
contexts, they should use native gcloud configurations. App config may remember
UI preferences and an optional preferred starting configuration, but account,
project, region, and zone should be read from gcloud configuration properties.

## Runtime Layer

Place runtime orchestration under `src/services/runtime/`.

Runtime owns:

- tracing initialization
- terminal interactivity checks
- terminal initialization and restoration
- panic restoration hook
- top-level render/input/event loop
- background task spawning
- cancellation tokens
- service polling channels
- effect dispatch coordination
- transient runtime-only state

Runtime should not own resource business rules. It coordinates app state and
services, then asks UI to render the current view.

Recommended startup order:

1. parse CLI in `main.rs`
2. build `Config`
3. initialize file logging
4. verify stdin/stdout are interactive terminals
5. install panic terminal-restoration hook
6. create `AppState`
7. create `RuntimeSession`
8. load or activate the initial native gcloud configuration through effects
9. initialize `ratatui`
10. enter the render/input/effect loop

Most effects are still dispatched synchronously from `RuntimeSession`. Context
picker discovery is one exception, and list refreshes for the active resource
service (GCE instances, GKE clusters, Cloud SQL instances, Cloud Run services,
Secret Manager secrets, Cloud Functions, Pub/Sub topics) are another: manual `r`, the poll
ticker, focus/scope auto-load, and the post-action refresh after a successful
GCE or Cloud SQL action all go through one async, non-blocking background
list-refresh path per active service. The runtime runs those gcloud requests in
bounded background tasks, sends typed results through the runtime event loop,
and uses cancellation or context/operation generation guards so results for
closed, replaced, or superseded work are ignored. Manual `r` supersedes an
in-flight scheduled refresh for the same service, so the older in-flight result
is dropped by the generation guard when it arrives. `operation_generation` is the
one counter that decides a background result's fate, and it is stamped once per
`dispatch_effects` **batch**, not once per effect: a batch that emits several
resource effects — `ResourcesAction::Refresh` with a detail view open emits a
list refresh and a detail load — must not invalidate the fetch it just started
itself. Cross-batch supersession is what the counter exists for and is
unaffected; the next batch stamps again. The bump is lazy, taken on the first
effect of the batch that opens a status-bar operation, so a batch that opens
none leaves in-flight fetches alone. That set of effects is wide enough to make
this the only supersession stamp a background result carries besides context
and scope: a second counter over a hand-maintained list of resource effects
could only ever drop results this one had already dropped. Whether that stays
true is a question about `effect_operation`, whose match lists every `Effect`
variant with no catch-all, so a new variant cannot be added without answering
it.
Dropping a list-refresh result is not a pure no-op: the refresh took the
service's load phase over when it started, so the drop site must also resolve
that phase back to idle, or a fetch detached by a context/scope switch leaves a
spinner nothing ever clears. Each background list refresh therefore carries a
`ResourceLoadTicket` that identifies the exact load it started, and the drop
site hands it to `ResourcesState::abandon_load`, which resets only while that
load still owns the phase. The reset belongs at the drop site and deliberately
not at context-switch time, where the fetch is still genuinely in flight and
"loading" is the honest state; `abandon_load`'s doc comment owns the details.
The drop site also re-arms the auto-load: when `abandon_load` reports that this
ticket still owned the phase and the recorded `auto_loaded_scope`'s service
matches the ticket's, that record is cleared, so returning to the Resources
workspace refetches instead of early-returning on a scope whose fetch the user
never received. Matching on service alone is enough because `auto_loaded_scope`
is only written past `ensure_active_resource_loaded`'s
`pending_background_refreshes != 0` early return, which cannot be reached while
this ticket is outstanding.
A `pending_background_refreshes` counter is a single global in-flight count,
not a per-service/context one; since only the active service/context is ever
scheduled, that global bound of one still yields at most one scheduled list
request per active service/context in flight at a time, guarding both the poll
ticker and auto-load. Background pollers and other cancellation-aware tasks
belong in this layer as the app grows.

A poll ticker driven by `Config::poll_interval_secs` refreshes only the active
resource service while the Resources workspace is focused; passing
`--poll-interval 0` disables background refresh entirely (manual `r` and
auto-load on focus/scope change still work). The active service auto-loads
once per distinct `(service, context)` scope on focus, service switch, and
configuration change, so the user never needs to press `r` just to see initial
data. A scope whose fetch was dropped never counts as loaded: the drop site
forgets the record, so the next return to Resources auto-loads it again. That
cannot loop — the reschedule still needs `needs_refresh`, and it only runs
past the `pending_background_refreshes != 0` early return, which defers
without recording the scope; a reschedule therefore only fires when nothing
is in flight, so it can never itself drop a result and restart the cycle. A
failed auto-load does not re-fire auto-load for that scope; the next poll
tick retries instead, so repeated failures wait for the next interval rather
than looping tightly.

Demo mode is a substitution at the service boundary and nowhere above it.
`--demo` swaps the process command runner for `DemoCommandRunner`
(`src/services/demo/`), which answers every `gcloud` and `bq` invocation with
an embedded JSON fixture — the same fixture files the test suite parses — and
swaps the tunnel launcher for one that behaves like a healthy child without
spawning a process. A demo session builds no native clients, so every read
falls to the CLI implementations under `src/services/resources/` that know how
to parse those fixtures. It also withholds the remembered-view store in both
directions: the fixture configurations are named `dev` and `prod` like real
ones, so a demo that read the state file would open on whatever service was
last open for real, and a demo that wrote it would save invented entries over
real remembered views. Those substitutions are the whole mechanism — app
state, reducers, projections, and every UI module above the service layer run
identically to a live session.

Never log to stdout or stderr while the TUI is active. Logs should go to a file.

## App State, Actions, And Reducers

The app layer is the behavioral core.

Use typed actions, never stringly-typed dispatch. Everything the app can do
is a variant of one `Action` enum, and each domain owns a nested action enum
rather than a free-form string or command name. The snippet below illustrates
that shape with a few example variants; it is not an inventory, and
`src/app/action.rs` is the source of truth for which variants exist.

```rust
// Illustrative shape only - see src/app/action.rs for the real enum.
pub enum Action {
    QuitRequested,
    Resources(ResourcesAction),
    EffectResult(EffectResult),
    // ... one variant per domain, plus effect plumbing
}
```

Use reducers for state transitions:

- workspace switching
- gcloud configuration selection, creation, activation, and reload state
- selection movement
- filter updates
- sort mode changes
- resource row reconciliation
- command palette modes
- confirmation workflows
- degraded state updates
- status messages

Reducers should be pure where practical. When a user action requires IO or a
network call, the reducer should return an `Effect`, and runtime should dispatch
that effect through services.

Do not let `AppState` become a broad object that mixes state, IO, workflow rules,
and persistence. Keep state ownership explicit under `app/state/`, and keep
transitions explicit under `app/reducer/`.

Configuration state should be first-class under `app/state/configurations.rs`.
It should own:

- loaded native gcloud configurations
- selected configuration row
- active configuration name
- account, project, region, and zone for each configuration when available
- filter/edit/confirmation modes for configuration workflows
- the active project/region/zone discovery picker, including loading, empty,
  filtered, selected, and degraded states
- loading, stale, and degraded states for configuration reads/writes

The root app state should also expose a current `GcloudContext` projection used
by resource reducers and UI rendering. Resource state must be scoped by the
active configuration so rows from one configuration cannot silently survive a
switch into another configuration.

## Navigation History

`NavigationState` (`app/state/navigation.rs`) is reducer-owned back/forward
history, held on `RootState` alongside workspace, configuration, resource, and
system state. It stores two stacks of `NavLocation` values; it does not store
widget state.

A `NavLocation` captures the workspace, and — inside the Resources workspace —
the active service plus, when a detail view is open, the detail target's
resource identity (name/zone for GCE, name/location for GKE, name/region for
Cloud SQL) and active detail tab. Filter text, sort field, and list selection are deliberately excluded:
they already live in per-service collection state and are sticky across
service and workspace switches, and refresh reconciliation restores selection
and open detail by resource identity rather than row index. Recording a
location's filter/sort/selection would fight that reconciliation and bloat
history entries with state that is not actually part of "where you were."

History is recorded only on forward-navigation actions — the set
`reducer::root::is_forward_nav_action` names: `Workspace` switches,
`ResourcesAction::OpenSelected`, `ResourcesAction::SelectService`,
`ResourcesAction::OpenLogsForSelection` (the `L` log jump),
`NavigatorAction::OpenSelected`, `CommandAction::Submit`, and
`JumpsAction::Submit` (the `J` related-resource jump) — and only when the
captured location actually changed. That last condition is what lets the set
include actions that only *might* navigate: a palette submit that copies a
value, or a jump to the tab already on screen, captures an unchanged location
and records nothing. New navigating actions belong in that gate rather than
calling `NavigationState::record` by hand, so there stays exactly one place
where history is written.
`Action::Navigation(NavAction::Back | NavAction::Forward)`
is handled before the general reducer: it captures the current location, pops
the appropriate stack, and re-applies the popped location by restoring
workspace/service/detail state directly. It never mutates
`ConfigurationsState` — activating a configuration, creating one, or editing
one is not something Back/Forward can do or undo; those are forward-nav
side effects with their own recorded locations, not history traversal.

If `Back` has no history to pop (`can_go_back()` is false) while a detail view
is open, it falls back to closing the detail view and returning to the
resource list instead of doing nothing. If the popped location's detail
target no longer exists (e.g. a deleted instance), `apply_location` resolves
to the parent resource list and shows a status message naming the missing
resource instead of restoring a dead detail view.

History is reset (`NavigationState::reset`) whenever the active gcloud context
changes: configuration activation, and any reload path that lands on a
different context than before. A location captured under one gcloud context
is not meaningful once the context has changed, so the stacks are cleared
rather than carried forward.

Keys: `Esc` goes back to the prior view from anywhere in the Resources
workspace (list or detail); `q` goes back while a detail view is open; `Alt-
Left` and `Alt-Right` are explicit Back/Forward regardless of view. The footer
only advertises `Alt-Left`/`Alt-Right` hints when `can_go_back()` /
`can_go_forward()` are true, so there are no dead-key hints.

## Remembered Last View

`RememberedViews` (`app/state/remembered_view.rs`) is a pure
`GcloudConfigurationName -> ResourceServiceKind` map: which resources service
was active the last time each gcloud configuration was, so nuvem can land on
it again — at startup and on every later switch into that configuration —
instead of always opening the same service everywhere.

`select_service` and `switch_context` are each called from several reducer
call sites (configuration activation, the navigator, back/forward navigation,
service-key presses). Recording "the service changed" or applying "what was
remembered" at each of those sites by hand would be the same hazard
`NavLocation::capture` (above) exists to avoid: a call site added later
compiles and passes every existing test while silently going unrecorded.
Instead both directions happen in exactly one place, `reduce`
(`src/app/reducer/root.rs`), which already uses this before/after idiom for
navigation history: a probe of `(active configuration, active service)` is
taken before the inner reducer runs and compared with the same probe after.

- The configuration changed → **land**: the new configuration's remembered
  service becomes active, or `default_service` if it has none yet. Nothing is
  learned from a landing, so this branch never writes back the value it just
  read.
- The configuration is the same but the service changed → **learn**: the map
  is updated and `Effect::SaveRememberedViews` is emitted once.

The reducer never touches the filesystem itself. It emits the effect; a
`RememberedViewStore` injected into `RuntimeSession` (the same pattern as
`ClipboardWriter`) does the actual write through `config::state::save`, so the
choke point stays testable with no I/O and the durability contract stays owned
by `config/state.rs` alone.

The write does **not** run on the run loop. `config::state::save`
creates a temp file, fsyncs it, renames it and fsyncs the directory; that is a
reasonable price for a command the user asked for by name, which is what the
other writers on the inline dispatch arm — `SaveLogEntries`,
`CopyToClipboard` — are. This effect is not one: it fires on ordinary
navigation, so on a networked or synced `$HOME` it put an fsync pair in the
path of every service switch for something the user never requested and could
not attribute. `dispatch_effects` therefore hands the map to a single
background writer (`RememberedViewWriter`) and returns.

Ordering is the constraint that fixes the writer's shape, because each save
carries the *whole* map: two saves landing out of order is a lost update, and
`config/state.rs` bounds last-write-wins to *across* processes, not within
one. So it is one channel with one consumer, running each save to completion
before taking the next — writes reach the disk in dispatch order by
construction. The outcomes come back on the session's ordinary result channel
in that same order, which is what keeps `remembered_view_saves_suppressed` a
single ordered transition rather than a race between two failures. Both exit
paths flush the writer before the process ends, so the eager saving does
not quietly become "saved unless you quit".

## Effects And Services

Use an effect/result boundary for side effects:

```rust
pub enum Effect {
    LoadConfigurations,
    ActivateConfiguration { name: GcloudConfigurationName },
    CreateConfiguration { draft: GcloudConfigurationDraft },
    UpdateConfigurationContext { draft: GcloudContextEditDraft },
    LoadContextPicker {
        kind: ConfigurationContextPickerKind,
        context: GcloudContext,
    },
    RefreshComputeInstances { context: GcloudContext },
    RefreshGkeClusters { context: GcloudContext },
    LoadComputeInstanceDetail {
        context: GcloudContext,
        instance: ComputeInstance,
    },
    StartComputeInstance {
        context: GcloudContext,
        instance: ComputeInstance,
    },
    StopComputeInstance {
        context: GcloudContext,
        instance: ComputeInstance,
    },
}

pub enum EffectResult {
    ConfigurationsLoaded {
        configurations: Vec<GcloudConfiguration>,
    },
    ConfigurationActivated {
        configuration: GcloudConfiguration,
    },
    ConfigurationActivationReloadFailed {
        configuration: GcloudConfiguration,
        error: AppError,
    },
    ConfigurationsReloadedWithWarning {
        configurations: Vec<GcloudConfiguration>,
        error: AppError,
    },
    ConfigurationCreatedReloadFailed {
        configuration: GcloudConfiguration,
        error: AppError,
    },
    ConfigurationContextUpdateReloadFailed {
        configuration: GcloudConfiguration,
        error: AppError,
    },
    ContextPickerLoaded {
        configuration: GcloudConfigurationName,
        kind: ConfigurationContextPickerKind,
        items: Vec<ConfigurationContextPickerItem>,
    },
    ContextPickerLoadFailed {
        configuration: GcloudConfigurationName,
        kind: ConfigurationContextPickerKind,
        error: AppError,
    },
    ComputeInstancesLoaded {
        context: GcloudContext,
        instances: Vec<ComputeInstance>,
    },
    ComputeInstancesLoadFailed {
        context: GcloudContext,
        error: AppError,
    },
    ComputeInstanceDetailLoaded {
        context: GcloudContext,
        detail: ComputeInstanceDetail,
    },
    ComputeInstanceDetailLoadFailed {
        context: GcloudContext,
        instance_name: String,
        instance_zone: String,
        error: AppError,
    },
    ComputeInstanceActionSucceeded {
        context: GcloudContext,
        instance_name: String,
        action: ComputeInstanceAction,
    },
    ComputeInstanceActionFailed {
        context: GcloudContext,
        instance_name: String,
        action: ComputeInstanceAction,
        error: AppError,
    },
    Failed {
        error: AppError,
    },
}
```

Services perform the operational work:

- `services/gcloud/` handles auth, project context, and common gcloud/API plumbing
- `services/gcloud/configurations.rs` wraps native configuration commands
- `services/gcloud/context_credentials.rs` turns the active gcloud configuration
  into credentials for the native clients, so a reading is aimed by the
  configuration the operator selected rather than by machine-wide ADC
- `services/gcloud/discovery.rs` wraps typed project, region, and zone discovery
  commands and parsers
- `services/resources/` handles service-specific resource loading
- `services/runtime/` dispatches effects but should not implement service details

A detail load is held to the resource it asked about. The reducer drops any
detail result whose resource is not the pane's current target, because such a
result is usually stale: the operator moved on before it returned. A load that
answers with a document about some *other* resource would be dropped the same
way, and with no failure to follow, its pane would wait forever.
`app::effect::DetailRequest` records what each detail effect asked for, and the
runtime passes every result through `DetailRequest::check`, which turns such an
answer into the load's own failure, naming both resources.

### Native clients and the CLI backend

Lists and detail loads run on the official Google Cloud client libraries, with
the two exceptions named below: BigQuery, which is read through `bq`, and
label-mode reads. `src/services/sdk/` holds one module per service —
`compute`, `disks`, `vpc`, `gke`, `cloudsql`, `secrets`, `pubsub`, `iam`,
`cloud_run`, `functions`, `logging`, `finops`, `storage`, and `discovery` —
over a shared `mod.rs`. `SdkClients` is the per-configuration client cache that
`sdk_client_cache!` builds: a client is bound to the credentials it was
constructed with, and those credentials are bound to one configuration name,
so keying the cache by name makes it impossible to serve a reading with a
client built for the configuration the operator just left. Clients are
constructed on first ask, because construction costs a TLS setup and one token
acquisition that a session which never opens that service should not pay.
Effect threads are blocking threads, so they drive the async calls with
`block_on`, or with `block_on_cancellable` where abandoning the read is the
right answer to the operator navigating away — dropping the future *is* the
cancellation, where the CLI path had to kill a process group. `detail_json`
renders the JSON tab for every pane through both redaction passes (KMS key
material, and Cloud SQL's certificate and private-key fields), so a pane
cannot leak by forgetting to ask for one.

Credentials come from the selected configuration, not from the machine.
`services/gcloud/context_credentials.rs` implements `CredentialsProvider` on
top of `gcloud config config-helper` for the active gcloud configuration:
Application Default Credentials sit beside `configurations/` rather than
inside any one of them, so a self-authenticating client would aim at a project
the operator did not choose. The token is held in memory until its expiry
(with a one-minute margin), never written to disk, and cached per
configuration name so a context switch cannot be served a token minted for the
configuration just left. The `Authorization` header it produces is marked
sensitive, so a bearer token cannot surface in a log line.

`gcloud` keeps everything that is not a read: configuration management (list,
activate, create, update), every lifecycle mutation, the tunnel and SSH
launches including `cloud-sql-proxy`, plugin runs, `container clusters
get-credentials` for the k9s workflow, the `gcloud auth list` diagnostics
probe, and BigQuery, which is read through `bq`. Project, region, and zone
discovery is native (Resource Manager `searchProjects`, and the Compute
`regions` and `zones` clients).

`src/services/resources/` keeps its CLI implementations and stays in the tree
for two reasons: `--demo` serves embedded fixtures through them, and they are
the fallback whenever a session has no native clients — the test harnesses,
and label-mode reads, which the native path cannot narrow (see below).
Whichever backend answers, it receives the selected configuration context
explicitly from services rather than reading global process state inside
widgets or reducers.

`docs/decisions/0001-native-sdk-for-read-paths.md` records the decision, the
per-call measurements behind it, and what it cost in dependencies.

Compute Engine reads run on the native client in
`src/services/sdk/compute.rs`: an aggregated `instances` list walk for the
table, capped at the same 500 rows (`RESOURCE_LIST_LIMIT`) the CLI path asks
for, and an `instances.get` for the read-only detail view, whose JSON tab is
the model rendered through `detail_json`. No server-side filter is pushed on
that path: the applied `/` filter narrows the loaded rows on the client,
except in label mode, which has nothing to narrow client-side and is routed
through `gcloud` instead. GKE and Cloud SQL split their reads the same way.

Every reload path — manual `r`, the poll ticker, a service switch, the
post-action follow-up, the end-of-batch refresh, and the reload a finished
plugin run triggers — reads the refreshed service's *own* applied filter
through `reducer::resources::active_filter_for`, which is keyed on the service
being reloaded rather than the active one. Neither substitute is safe: another
service's term would narrow the wrong stored list, and `None` would silently
disarm the filter, because a reload *replaces* the stored rows and label mode
leaves no client-side narrowing to re-apply — the table would widen to the
whole project while the tools line still claimed the filter was in force. See
`services/resources/list_filter.rs` for the never-drop-a-row superset gate
that governs the CLI backend's server-side list filter, and for the prefix
grammar of the multi-mode filter that supersedes its bare-substring term.

The same typed command runner serves the CLI backend: `gcloud --configuration
<active-config> compute instances list --format=json --limit=500` for list
refreshes and `gcloud --configuration <active-config> compute instances
describe <name> --zone <zone> --format=json` for the detail view. That backend
is what the test harnesses run on, and what a label-mode list refresh falls
back to. Lifecycle actions go through the client library whenever the session
holds clients, which every running session does, demo included; the CLI
backend's `gcloud compute instances start|stop|... <name> --zone <zone>` is
reached only by a session with no clients, which in practice means a test
harness.
The runtime maps command and JSON parse failures to
`ComputeInstancesLoadFailed` or `ComputeInstanceDetailLoadFailed` so reducers
can stale, clear, or update only rows and details for the matching active
context and selected instance zone. Confirmed action failures map to
`ComputeInstanceActionFailed`; matching-context action success and failure both
close confirmation, mark the resource list stale, and clear rows. A successful
action also emits `RefreshComputeInstances`, which runs the active-context list
command once in the background. A matching successful result restores fresh
rows and clears the status; a failed result leaves rows stale/empty and shows a
degraded error. Manual `r`, the post-action refresh, the poll ticker, and
focus/scope auto-load all run through the same async background list-refresh
path (see the Runtime Layer section above), and stale action or follow-up
results cannot replace newer context/resource state.

The `/` filter text is not a bare substring, and both backends read the same
grammar. It lives in `nuvem_domain::gcp::filter` (`term` selects regex, `-f`
fuzzy, `-l k=v[,k=v]` label, `!` inverse; an empty remainder after any prefix
means "no filter"), and both layers above it read the same parse:
`app::state::resources` compiles it into a `CompiledFilter` and evaluates it
over loaded rows, while `services/resources/list_filter.rs` translates the
same `FilterQuery` into a `gcloud --filter`. It sits in the domain crate
precisely so neither of those layers has to import from the other, and so the
client predicate and the server gate can never disagree about what an input
means.

The native path pushes nothing, deliberately: the Compute API does not speak
gcloud's filter language. The `~` regex form the pushdown emits is rejected
outright, and the nearest API form that does parse returns *zero* rows when two
fields are joined with `OR`, so the alternative to an unoptimised listing is a
silently empty one. Regex, fuzzy, and inverse therefore narrow on the client,
which is where they already narrowed, since the client predicate re-runs over
whatever the server returns. The one visible difference is a project holding
more than the 500-row `RESOURCE_LIST_LIMIT` that compute, disks, and VPC still
apply: the cap now bites before the narrowing rather than after it, so a match
beyond the cap is never fetched, where a pushed expression would have reached
it. Label mode is the exception, because it narrows nothing on the client (see
below), so those reads take the CLI path instead.
`ResourcesState::filter_mode` reports the draft's mode while the filter input
is open and the applied text's mode otherwise, so the UI chip can label a
half-typed input.

The pushdown machinery in `services/resources/list_filter.rs` therefore serves
the CLI backend — demo mode, the test harnesses, and label-mode reads — and
its never-drop-a-row invariant still governs that path. `pushdown_expression`
dispatches on the parsed mode: only regex and label can reach the server;
fuzzy, inverse, an empty filter, and a parse error all return `None` and leave
the narrowing (or the lack of it) to the client.

Label mode is server-authoritative. Almost none of the list row structs
(`ComputeInstance`, `GkeCluster`, `CloudSqlInstance`, `CloudRunService`,
`StorageBucket`) carry labels — only the `*Detail` structs do — so there is
nothing client-side to match against. `CompiledFilter::Label` therefore keeps
every row, uniformly, including for `Secret`, whose list row does carry them:
one row-keeping rule beats a per-service one, and the pushdown is what makes
the mode narrow at all. All narrowing happens in the pushed expression:
`labels.k = "v"` for GCE instances, `resourceLabels.k = "v"` for GKE
clusters, `settings.userLabels.k = "v"` for Cloud SQL instances, and
`labels.k = "v"` for Secret Manager secrets, `labels.k = "v"` for Cloud
Functions, `labels.k = "v"` for Persistent disks, `labels.k = "v"` for
Pub/Sub topics, and `labels.k = "v"` for Pub/Sub subscriptions, AND-joined
across pairs, with
`nuvem_domain::gcp::filter::label_filter_field` as the single source of those
raw paths. Keeping every returned row is trivially safe under
the never-drop-a-row invariant — the client cannot drop a row the server did
not return — but it also means label filtering exists only where the pushdown
does. Cloud Run exposes no user labels in list output, so
`label_filter_field` returns `None` and `ResourcesState::filter_error` reports
`label filtering is not supported for Cloud Run` instead of an unfiltered
table that would read as "every row has that label". Cloud Storage never
reaches that branch at all — it sits outside the grammar (see below), so
`filter_error` short-circuits to `None` before the input is even parsed. On
the native read path this is also why a label-mode read is routed through
`gcloud` rather than the client library: `list_filter::label_mode_needs_gcloud`
answers true for exactly the applied filters whose only narrowing is the pushed
expression, and the runtime's list helpers take the CLI path for those. The
same accessor reports
`label selector is not expressible as a gcloud filter` when a key or value
falls outside the `is_server_expressible` charset, or when a key contains `.`
— the charset admits a dot, but a key is interpolated *unquoted* after the
label field, so `a.b` would emit `labels.a.b = "c"` and gcloud would read it
as a nested field path; GCP label keys cannot contain `.` at all, so such a
selector is always nonsense. `is_expressible_label_selector` is shared by the
state layer and the pushdown so that inline message and the pushdown's `None`
can never diverge. Because gcloud does the matching, label *values* are
case-sensitive, unlike every client-side mode.

The opaque-label gate is pattern-aware. Asking whether the lowercased term is
a *substring* of a derived display label (`running`, `ha (regional)`,
`public`, ...) is exact for a literal but wrong for a pattern: `r.n` is no
substring of `running`, yet it matches the `Running` status cell, so pushing
it down would drop that row. `pattern_matches_opaque_label` compiles the term
through the client's own `compile_filter` and asks whether it *matches* any
enumerated opaque label, so the gate and the predicate cannot disagree; a term
that will not compile is treated as gated, since there is nothing safe to
push.

Regex-mode pushdown, on that CLI backend, is a superset even though the client
reads the term as a pattern. `is_server_expressible` admits only ASCII
alphanumerics and
`.` `_` `-`, so the one surviving metacharacter is `.`, and `.` is
position-independent: any unanchored match inside a display value is also a
match inside any raw value that contains that display value as a substring,
which is exactly what the verbatim-superset field rule guarantees. Anchors
(`^`, `$`), quantifiers, alternation, and character classes are all rejected
by the charset gate, so no pattern that could match a display value while
failing on a superstring of it ever reaches the server.

GKE layers one further gate inside `load_clusters`: a term made only of digits
and `.` is never pushed. `currentNodeCount` is neither a pushed raw field (a
`~` regex clause on an integer field risks a gcloud runtime error) nor an
enumerated opaque label, so it is a hole in the two-rule partition above. The
gate covers dots as well as digits because `.` matches any character, so
`1.27` can match the node count `1127`.

Cloud Storage stays out of the grammar entirely. It shares one filter string
between the bucket list and the object browser, and the object browser already
pushes that string down as a server-side object prefix through
`is_safe_remote_prefix`; feeding grammar prefixes into that path would fight
the prefix pushdown for no benefit. Storage keeps its plain substring filter,
`filter_mode` reports `regex` for it, `filter_error` is always `None`, and the
tools line renders no mode chip. So a grammar prefix typed on Storage is
matched *literally*: `StorageState::visible_buckets` reads `-l env=prod` as
that exact substring, and `-f api` and `!x` likewise. Such an input normally
selects nothing, and the user sees an empty table with no chip and no `⚠` —
there is no "label filtering is not supported" message on this path, because
the grammar is never parsed here.

A filter that will not parse or compile narrows nothing rather than matching
nothing: the applied pattern is compiled once per apply into a
`CompiledResourceFilter` (`app/state/resources/filter_cache.rs`, the table's
counterpart of the log viewer's `CompiledTextFilter`) whose fallback is
`CompiledFilter::None`, so the table shows every loaded row while
`ResourcesState::filter_error` carries the message inline — never an empty
table, never a panic. An empty
table behind a bad regex is indistinguishable from a correct empty result, so
showing too much and saying why is the safe failure. The regex message keeps
the engine's own cause (`invalid regex: unclosed group` for `a(b`), reduced to
a single line because it renders inside the tools line. The `⚠` describes the
*applied* filter, so the UI suppresses it once the draft diverges from what is
applied.

The configuration service wraps the gcloud commands that still shell out. Configuration management has no native equivalent; project, region and zone discovery runs on the native clients, and the discovery commands listed last are the fallback taken when no native clients are configured:

- `gcloud config configurations list --format=json`
- `gcloud config configurations activate <name>`
- `gcloud config configurations create <name>`
- `gcloud --configuration <name> config set account <account>` when creating a
  configuration and an account is provided
- `gcloud --configuration <name> config set project <project>` when creating a
  configuration and a project is provided
- `gcloud --configuration <name> config set compute/region <region>` when
  creating a configuration and a region is provided
- `gcloud --configuration <name> config set compute/zone <zone>` when creating a
  configuration and a zone is provided
- `gcloud --configuration <active-name> config set/unset project` when editing
  the active context's project
- `gcloud --configuration <active-name> config set/unset compute/region` when
  editing the active context's region
- `gcloud --configuration <active-name> config set/unset compute/zone` when
  editing the active context's zone

Project, region, and zone discovery is native. The context pickers load
through `services/sdk/discovery.rs`: projects from Resource Manager's
`searchProjects`, and regions and zones from the Compute `regions` and `zones`
clients, scoped to the selected project. The load is cancellable, so a picker
the operator closes drops its request instead of waiting for a process to die.
`services/gcloud/discovery.rs` keeps the equivalent `gcloud --configuration
<active-name> projects list --format=json`, `compute regions list`, and
`compute zones list` commands, and answers whenever the session has no native
clients.

When possible, pass `--configuration <name>` for reads and writes that should be
scoped to a specific configuration. Avoid relying on implicit active global
state except for the explicit activation workflow.

### Interactive-process launches (GCE SSH)

Most effects run in the background and report an `EffectResult` the run loop
can apply whenever it arrives. An interactive SSH session cannot work that
way: the child process needs the real terminal, and only the run loop (which
owns `ratatui::DefaultTerminal`) is allowed to hand it over and take it back.
`Effect::LaunchInteractiveProcess { command: InteractiveCommand }`
(`InteractiveCommand::GceSsh { context, instance }` and
`InteractiveCommand::K9s { kube_context }`) exists to route that
one case around the normal `dispatch_effect` path: `dispatch_effects` stores
it in `RuntimeSession.pending_interactive_command` instead of spawning a
background task, and after each key-driven `dispatch_action` the run loop
calls `run_pending_interactive` to take and execute it synchronously before
drawing the next frame.

`services/runtime/interactive.rs` is the tty-independent seam that makes this
testable: `InteractiveProcessSpec` (program, args, description) is passed to
the OS directly via `SubprocessRunner` — no shell, so there is no argv
interpolation to escape or exploit. `gce_ssh_spec` builds the spec for
`gcloud --configuration <name> compute ssh <instance> --zone <zone>` from the
configured gcloud binary. `InteractiveLifecycle` (`RatatuiLifecycle` in
production) brackets the child with `before_launch` (save the panic hook,
`ratatui::restore()` to hand back cooked mode/main screen/cursor) and
`after_launch` (`ratatui::init()`, clear, reinstall the panic-restore hook).
`execute_interactive_process` sequences `before_launch -> run -> after_launch`
and is the unit-tested orchestrator; both the runner and the lifecycle are
traits so tests use fakes to assert argv and lifecycle-call ordering without a
real terminal or subprocess. `tmux_detected_in` reports whether `$TMUX` is set
so the runtime can log the environment, but the direct-terminal path above is
what actually runs — a fire-and-forget `tmux new-window` launch is deferred to
a follow-up because it cannot report an exit status back into the app.

Because the launch runs synchronously inside the run loop before the next
frame, there is no window for the active context or selection to change out
from under it, and the existing background-result generation guards still
protect any results that were queued during the child's drain. The outcome
maps to a single status line: `Succeeded` → `"<description> finished"`;
`ExitedNonZero { code }` → `"<description> exited with status <code>"`;
`TerminatedBySignal { signal }` → `"<description> terminated by signal
<signal>"`; `MissingBinary` → `"<description> failed: binary not found"`;
`LaunchFailed { message }` → `"<description> failed: <message>"`. Cancelling
the SSH preview (`Esc`) never launches anything and shows `"SSH launch
cancelled"` instead.

The "Open in k9s" workflow layers on this same seam rather than forking a
second interactive runtime. Pure planning lives in `nuvem-domain::gcp::k9s`
(`resolve_launch_target`, the deterministic `gke_<project>_<location>_<cluster>`
context name, `get_credentials_argv`/`k9s_argv`, and `resolve_credential_step`).
Choosing the palette command emits `Effect::PrepareK9sLaunch`, whose handler
runs a best-effort `kubectl config get-contexts` probe
(`services/runtime/k9s::resolve_existing_contexts`) to decide reuse-vs-create —
a probe used only for the confirmation wording; its absence or failure never
blocks the launch. Confirming emits `Effect::RunK9sCredentials`, which runs
`gcloud … get-credentials` as a captured, non-interactive pre-step when the
context is missing; a credential failure surfaces its error and prevents the
launch. On success the reducer re-checks read-only mode — a config reload can
turn it on while `get-credentials` is in flight — and refuses the launch if it
is on; otherwise it emits `Effect::LaunchInteractiveProcess { command:
InteractiveCommand::K9s { kube_context } }`, resolved to `k9s --context <name>`
via `k9s::k9s_spec` and run through the shared seam above. (Read-only mode's
user-facing contract is documented in `docs/guides/read-only-mode.md`.)

## Events And Polling

Model external changes as typed events.

Examples:

```rust
pub enum ServiceEvent {
    GcloudConfigurationChanged {
        previous: Option<GcloudConfigurationName>,
        current: GcloudConfiguration,
    },
    ProjectContextChanged(ProjectContext),
    ResourcesRefreshed {
        service: ServiceKind,
        resources: Vec<ResourceSummary>,
    },
    ResourceStatusChanged {
        id: ResourceId,
        status: ResourceStatus,
    },
    ServiceHealthChanged {
        service: ServiceKind,
        health: HealthState,
    },
}
```

Background pollers should observe Google Cloud state and send events through
channels. They should not mutate `AppState` and should not know about UI layout.

For `nuvem`, polling must be careful because cloud APIs can be slower, rate
limited, or partially unavailable. The app should support:

- manual refresh
- background refresh per active service/workspace, driven by a poll ticker
  (`Config::poll_interval_secs`, disabled when set to `0`) and by auto-load on
  focus/scope change, with at most one scheduled list request per
  service/context in flight
- stale data or stale/empty state when a context-scoped flow deliberately
  invalidates rows while work is in flight
- loading indicators
- degraded health when auth, quota, API, or CLI dependencies fail
- full resource reload after the active gcloud configuration changes

Do not represent "failed to load" as a healthy empty resource list; use the
explicit stale/degraded state when a failed load clears rows.

Switching configurations is a context change, not a normal row update. The
runtime should cancel or ignore in-flight resource refreshes from the old
configuration, clear or mark old resource rows stale, re-read the active
configuration properties, and then reload the active workspace under the new
context. To make repeated project/region hopping feel instant, the state layer
retains each context's loaded resource rows in a bounded per-context cache
(keyed by the full `(configuration, project, region/zone)` context) as it
switches away; switching back to a recently visited context restores its
cached rows as stale-but-visible so the workspace shows data immediately while
the freshness system drives a background refresh.

## UI Layer

The UI layer renders state and nothing else.

Recommended entry point:

- `src/ui/mod.rs` checks terminal size
- it chooses the active workspace
- it builds render projections
- it delegates to smaller modules

Use a small-terminal fallback. This is not cosmetic. It prevents broken layouts
when the terminal is too small for the shared frame.

The current application-frame layout is:

```text
header: app/view title and prioritized active configuration/project context
breadcrumb: current workspace and detail path
body: active workspace content
status: operation phase, status message, and last-refresh time
footer: mode-aware key/action hints
overlay: help, command palette, confirmations, previews, and context picker
```

`ui::render` owns the frame regions; workspace renderers own only the body
rectangle. Keep reusable display helpers under `ui/shared/` and semantic
visual roles under `ui::theme`.

Configuration UI should be reachable from both a dedicated workspace and the
command palette. The minimum useful flow is:

- show all native gcloud configurations
- highlight the active configuration
- show account, project, region, and zone columns when available
- activate the selected configuration with confirmation
- create a new configuration through an edit form
- edit the active configuration's project, region, and zone through an edit form
- discover project, region, and zone values from that form with a field-scoped
  picker overlay
- show degraded state if `gcloud` is missing or returns malformed output

The configurations workspace is the dedicated surface for that flow. It shows
name/account/project/region/zone columns, active and selected markers,
client-side filtering over those context fields, confirmed activation, reload
warnings, empty/error/filtered-empty status text, and a configuration create
form. It also edits the active configuration's project, region, and zone
through `gcloud config set/unset` calls scoped with
`--configuration <active-name>`. An inactive selected configuration is
activated from the list, or from a command palette opened with `:`, and only
after a confirmation overlay shows its target context and the exact activation
command. Very small terminals show a larger-terminal message and withhold the
activation until that preview fits. Editing an inactive configuration, and
editing accounts, are not supported. The active-context edit form opens
project, region, and zone discovery pickers with Ctrl-Space; picker results
are filtered and selected locally, and a separate form submission performs the
update. Picker loading, empty, filtered-empty, degraded, retry, and
cancellation states live in the overlay, and late results are ignored when
the target form or picker has changed.

The Resources workspace covers every service in the resource registry. The
inventory itself lives in `ResourceServiceKind::ALL` in `nuvem-domain`, which
the `service_kinds!` macro generates from the enum's own variant list so a
caller that must cover the whole set walks the enum instead of re-listing it.
The service registry in `src/app/service_registry.rs` carries one descriptor
per kind — label, category, aliases, availability, target — and
`docs/reference/services/` describes what each service shows; tests hold both in
parity with `ALL`, so neither drifts out of it silently. This document keeps
no list of its own, and neither does any renderer.

The workspace loads rows for the active configuration, renders
service-specific rows, sorts loaded rows client-side, and narrows them through
the filtering behavior described above — client-side for regex, fuzzy, and
inverse, while label mode narrows on the server and is routed through the
gcloud path for exactly that reason. It distinguishes initial, healthy empty,
stale, filtered-empty, and failed-refresh states with visible text, and opens
read-only tabbed detail views. Lifecycle actions for GCE and Cloud SQL
instances are reached from the resource command palette; selecting one opens a
confirmation overlay with the active configuration, project when known, the
target resource, its location, the impact, and the API call the action will
make (`Call: compute.instances.stop zones/<zone>/instances/<name>`). Very
small terminals show a larger-terminal message and withhold the action until
the preview fits. After a successful action, the stale rows stay cleared while
a context-scoped list refresh runs once in the background; success restores
fresh rows, while failure keeps the stale/empty state and shows a degraded
error. Manual `r`, the poll ticker, focus/scope auto-load, and this
post-action refresh all run through the same unified async background
list-refresh path per active service; manual `r` supersedes an in-flight
scheduled refresh via the per-batch generation stamp described in the Runtime
Layer section, and a load abandoned by navigating away is refetched on return
rather than landing on empty rows and a press-`r` prompt.

The header and confirmation dialogs must show the active configuration name and
project. A user should never have to infer which cloud context an action will
touch.

Do not:

- mutate app state from render functions
- call services from widgets
- duplicate filtering or selection logic inside widgets
- embed gcloud-specific business rules inside generic list/detail widgets
- assume large terminals

## Input Layer

`src/input/` translates raw terminal input into semantic actions.

Follow the layering split:

- `input/mod.rs` defines context and helper types
- `input/keymap.rs` maps `crossterm` key events to actions

Input mapping should answer "what did the user ask to do?" It should not answer
"how should state change?" The reducer answers that.

Mode-aware input is mandatory. Normal mode, filter mode, command palette mode,
confirmation mode, help mode, edit mode, and configuration-context-picker mode
should each have explicit key mapping behavior. The picker owns `/`,
Backspace, Enter, Esc, `j`/`k`, Up/Down, and `r` while Ctrl-C remains global.

Initial key model:

- `Ctrl-C`: quit from any mode
- `?`: help
- `h`: home workspace
- `c`: gcloud configurations workspace
- `g`: Resources workspace
- On Home, `/` searches the service registry, `j/k` or arrows move through
  matching services, and `Enter` opens the selected service directly
- In Resources, `v` shows or hides the service sidebar and `Tab` moves focus
  between the sidebar and resource content, including while detail is open
- `j/k` and arrows: move selection in the configurations workspace, resource
  list, focused Resources sidebar, or command palette
- `r`: refresh the active workspace
- `Y`: copy the last recorded gcloud diagnostic's sanitized, bounded
  technical detail to the clipboard (available globally, like `r`/`?`); with
  no diagnostic recorded yet, sets a "No diagnostic to copy" status message
  instead
- `a`: open the create form from the configurations workspace
- `e`: edit the active gcloud context from the configurations workspace
- `:`: open the command palette from the configuration or resource list
- `/`: search services on Home or in the focused Resources sidebar; edit the
  resource list or configuration list filter when content is focused
- `u`: clear the active configuration filter from the configurations workspace
- `←`/`→`: move the selected column in the resource list
- `s`: sort by the selected resource column (press again to reverse direction)
- `w`: toggle wide columns in the resource list
- `Tab`: move focus between the Resources sidebar and resource content; in
  create/edit forms, move to the next field
- `Shift-Tab`: move to the previous create/edit form field
- printable characters and `Backspace`: edit the active filter draft or filter
  the command palette while it is open
- `Enter`: open the selected Home/sidebar service directly, create a
  configuration, save an active-context edit, request activation for the
  selected configuration, apply a resource or configuration list filter draft,
  or open selected resource detail; in the command palette, choose the
  selected command; in a configuration activation or GCE action confirmation,
  confirm only when the full command preview is visible
- `y` and `n`: confirm or cancel a configuration activation confirmation; `y`
  confirms only when the full command preview is visible
- `[` and `]`: cycle resource detail tabs
- `Esc`: close help, cancel a configuration activation, cancel create/edit forms
  or filter drafts, dismiss an active modal/mode, or — anywhere in the Resources
  workspace — go back through navigation history (see "Navigation History")
- `Alt-Left` / `Alt-Right`: go back/forward through navigation history from
  anywhere in the Resources workspace (see "Navigation History")
- `q`: quit from home and list views, or return from resource detail

Avoid hidden key behavior that is not reflected in footer/help text.

Every `KeyCode::Char` arm in `keymap.rs` must guard with `is_plain_text_key`
unless it deliberately wants a modifier — and an arm that does must compare
modifiers exactly (`key.modifiers == ...`), never `.contains`, or multi-modifier
chords such as `Ctrl-Alt-<x>` reach it too. Without the guard the arm matches on
the character alone, so `Ctrl-<letter>` silently reaches whatever plain-letter
arm shares that character — which is how `Ctrl-S` (the Logging save key) also
sorted the resource list. Splitting a composite arm such as
`Char('j') | Down` to add the guard must keep the non-`Char` half dispatching.
`tests/keymap_help_parity_test.rs` sweeps every printable-ASCII character and
every non-`Char` key code, crossed with the Ctrl/Alt/Super chord-modifier sets,
in every help context with no allowlist: a failure means either a real binding
missing its catalog entry, or an unguarded arm. Do not add an allowlist to
silence it. SHIFT is character-forming, not a chord modifier, and the non-`Char`
control keys (Enter, Esc, Tab, …) deliberately dispatch the same action under
any modifier — both are asserted as positive invariants in that test, with the
reasoning recorded there.
App keys that resolve *after* the `[hotkeys]` override table (`Ctrl-S`,
`Ctrl-T`, `Ctrl-X`) must also be reserved in `config/hotkeys.rs`, or a user
binding wins and steals them silently; see `docs/configuration/hotkeys.md` for the
user-facing rule.

Later command palette defaults should include:

- `Gcloud: Switch Configuration`
- `Gcloud: Create Configuration`
- `Gcloud: Refresh Configurations`
- `GCP: Switch Project` only if project switching remains useful inside the
  active configuration

## GCP Service Model

Model Google Cloud services as first-class domains, but keep their UI consistent.

### Service registry and navigator flow

The service registry is the declarative catalog for service identity, labels,
categories, aliases, availability, and optional resource targets. Home and the
Resources sidebar use the same registry-backed navigator, so search and direct
selection do not need a separate list of services in either renderer.

The registry-to-navigator-to-projection-to-render flow is:

```text
service registry -> navigator state/reducer -> navigator projection -> Home/sidebar renderer
                                      \-> typed ResourceServiceKind selection -> existing effects
```

Navigator state and its reducer own the search draft, applied search, selected
descriptor, sidebar visibility, and focus. The navigator projection converts
that state into selectable entries, category headers, availability text, and a
viewport for Home or the sidebar. Selecting an available target resolves to a
typed `ResourceServiceKind`, after which the existing resource state and
effects load or refresh the service-specific resource behavior. Renderers only
consume the projection; they do not inspect registry descriptors or dispatch
service effects directly.

This keeps service-specific resource behavior behind typed state/effect
boundaries while allowing unavailable services to remain visible and
searchable. Home or switcher branching is not a service integration point.

Suggested starting services:

- Native gcloud configurations and active context
- Projects visible to the active configuration
- Compute Engine instances
- GKE clusters
- Cloud Run services
- Cloud SQL instances
- Cloud Storage buckets

Each service should define:

- typed resource identity
- selected gcloud configuration requirements
- summary row fields
- detail model
- status model
- list/describe/logs operations if supported
- operation safety rules
- degraded-state behavior

### Cloud Storage: a bespoke hierarchical object browser

Cloud Storage buckets is the one implemented service whose
resource state does not fully fit the flat list/detail schema above. The
bucket list itself is flat and reuses the same shared per-service collection
state as every other service's rows (instances, clusters); but rows under a
bucket are hierarchical, so `StorageState` pairs that generic bucket
collection with a separate bespoke object-browser sub-state (`prefix_path`,
loaded `prefixes`/`objects`, a continuation token, and per-scope
problem/requester-pays/partial-page-error flags). `Enter` on a bucket
switches `StorageView` from `Buckets` into the browser; `Enter` on a
synthetic prefix row descends one level and pushes onto `prefix_path`, while
`-`/`Backspace` pops it back off, regenerating the breadcrumb from the
remaining path (returning to the bucket list once the path is empty). This
is still a service-specific resource behavior reached through the same typed
`ResourceServiceKind::CloudStorage` selection as any other service — the
registry/navigator boundary above is unchanged — but the shape of the
object-browser state behind that selection is bespoke rather than reusing
the shared flat-row model.

Object listing is bounded, on-demand pagination: each `n` keypress requests
one more fixed-size page (`STORAGE_PAGE_SIZE`, 100 objects) and appends it,
preserving the current selection; there is no unbounded load of an entire
bucket. On the native path that request is one `list_objects` call carrying
the current prefix, a `/` delimiter, and the page token the service issued for
the previous page. The delimiter makes the service group one level for us —
it answers `prefixes` and `objects` directly instead of a flat array the
browser would have to group by object key — and because the token is the
service's own cursor, continuation is exact: the next page begins where the
previous one ended. A loaded page's filter can be local (client-side, over
already-loaded rows) or, when the filter text is also a safe prefix, remote
(re-requested scoped to that prefix); the projection labels which scope is
active.

Demo mode and the CLI fallback go through `gcloud storage objects list`, which
accepts a page token as input but never emits one. There the bound is
`--limit`, folders are grouped back out of the returned object keys, and every
page is its own last page, so continuation on that path is best-effort. The
page-token/append/selection behavior itself is modeled at the domain/reducer
layer and is fixture-tested (see `tests/gcs_parse_test.rs`,
`tests/gcs_service_test.rs`, and the `cloud_storage_*` cases in
`tests/reducer_resources_test.rs`) rather than verified against live changing
bucket state. The browser also distinguishes problem states that a flat list
does not need: empty bucket, filter with no match, permission denied,
requester-pays billing, a bucket-level retention policy, and a partial page
failure that keeps already-loaded rows when a later page fails.

### BigQuery: two drill-down levels, no detail pane

BigQuery is the second service whose selection hides several lists, and it is
the cheap version of the shape above. `Enter` on a dataset row switches
`BigQueryView` from `Datasets` to `Tables` and issues one
`Effect::LoadBigQueryTables` for exactly the dataset the view now records;
`Enter` on a table row switches it on to `Schema` and issues one
`Effect::LoadBigQuerySchema` for exactly that table. `Esc` ascends one level
per press, and so do `-`/`Backspace`, which the reducer answers for BigQuery
before the Cloud Storage `ascend` path it was written for. The levels are a
fixed hierarchy rather than a path, so there is still no path to push, no
breadcrumb to regenerate and no pagination: `BigQueryDrilldownState` holds the
entered dataset, the tables listed for it, the entered table and the columns
listed for it, and each level's own cursor, filter and failure. Arriving at a
level clears what lives below it, so leaving discards a level's rows rather
than caching a scope, while the list behind the one being left stays
populated for the `Esc` that comes back to it.

The schema level's identity is the drill generation alone, with no listing
epoch beside it. The epoch exists to separate two `bq ls` calls made without a
view transition between them — what `r` does at the tables level — and
`reload_bigquery_schema` answers `r` here by re-entering the level, which is a
transition and mints a generation. So `matches_bigquery_schema_scope` is one
equality, and a second conjunct would be one no path could make false on its
own.

A landed tables listing then fans out: one `Effect::LoadBigQueryTableStats` per
listed row, each answered by a `bq show` for that row's table, each merged onto
its own row by `merge_bigquery_table_enrichment` — which is not the listing
setter, because a listing resets the cursor and clears the problem and an
enrichment must do neither. Every listing request mints a `BigQueryListingEpoch`
and its fan-out carries it alongside the drill generation, so
`matches_bigquery_enrichment_scope` drops an answer to a listing that has been
replaced, or to a drill that has been left. The calls run off the run loop
behind `BIGQUERY_ENRICHMENT_CONCURRENCY` permits, four at a time, and come
back on `QueuedEffectResult::BigQueryEnrichment`, which applies no generation
guard: the ordinary background queue would discard an outstanding answer the
moment any unrelated operation bumped its own generation, and the reducer's
scope check is the guard that belongs here. Nothing bounds the number of calls
but the listing's own `-n 500` cap, and a refused one leaves that row's three
stats `TableStat::Unavailable` — which the cells paint as a dash and never as
a zero — while the rest of the listing stands.

That scope check has an operator-visible consequence at the schema level, and
it is accepted rather than fixed. `Enter` on a table bumps the generation, and
the `Esc` back bumps it again while `restore_invariant(Tables)` clears only the
schema — so the tables rows survive the round trip, but the fan-out answers
still outstanding carry the generation from before it and
`matches_bigquery_enrichment_scope` drops every one. The rows the fan-out had
reached keep their numbers; the rows it had not sit dashed on a list that reads
as settled, until `r` re-issues the listing on a fresh epoch. Accepting a
stale-generation enrichment instead would reintroduce exactly what the check
exists for — an answer about one drill merged onto another's rows — so the
behaviour is documented in the README beside the other two meanings of a dash.
The calls themselves are not cancelled either: `supersede_bigquery_enrichments`
fires only on `Effect::LoadBigQueryTables`, so entering a table leaves the
remainder of the fan-out running four at a time, holding its permits and
spawning `bq` processes alongside the schema's own `bq show`, and every one of
their answers is discarded.

All three views live under the single `ResourceServiceKind::BigQueryDatasets`,
for the reason the object browser has no kind of its own: a tables view is
reachable only from a dataset row and a schema view only from a table row, so a
second kind would offer either in the navigator, the palette and the jump
picker as a destination selectable with nothing chosen above it. None of them
has a detail pane — `Detail` and `DetailTab` are `()` on the datasets
collection, and the schema is a third list rather than a pane over the second.
That is why the enrichment fan-out still drops the `schema` its answers carry:
entering a table re-issues its own `show` for the one table being opened, which
costs one call at the moment the drill is already waiting, where retaining
every listed row's schema would cost unbounded memory for a view nobody may
open. `Enter` on a column row opens nothing — a column is the bottom of the
hierarchy — and the keymap refuses it there rather than leaving it to the
reducer to no-op. The keymap `Context` carries the depth as well as the flag:
`resources_drilldown_open` says a drill is on screen and
`resources_drilldown_opens_another_level` says whether anything is under it,
both set from the `ResourcesState` methods of the same names.

The depth exists because `HelpContext` derives from that context: one flag
with no depth cannot tell a level that opens another from one that does not,
so the help overlay would teach `Open / enter` on a column row, where nothing
opens. It resolves to `ResourceDrilldown` or `ResourceDrilldownLeaf`, whose
catalogues differ by exactly that binding. The frame footer never had the
problem — it reads live state through `drilldown_opens_another_level` rather
than a context — so both surfaces answer the same question from the same
method.

The catalogue could not be narrowed on its own: `keymap_help_parity_test`
refuses a key that dispatches with no catalogue entry for the context, so the
keymap arm had to stop answering first. The reducer's own refusal
(`open_selected_bigquery_table` returns `None` from a column row) is still
there, which is why this changed nothing an operator can see.

Two consequences are worth naming because they are what a reader checks. The
tables list has no sort field at all (its catalog's sort type is `Infallible`),
so the tools line drops both the sort segment and the `←/→ col · s sort`
hints rather than painting an arrow over rows that arrive in `bq ls` order —
those keys still move the datasets collection behind the drill, and the schema
list has no sort field either, for the same reason plus one of its own: schema
order is what keeps a record's children under it. And `r` inside a drill
re-issues the call the view on screen is showing as well as the datasets
refresh, which is what makes the failure state's "Press `r` to retry." a
statement about the call that failed. At the tables level that is the listing,
and with it goes its whole fan-out again on a fresh epoch, so the stats already
on screen are dropped rather than left standing as the answer to a refresh; two
levels down it is the one table's `bq show`, and a `bq ls` issued from there
would land a tables listing while the operator is reading columns. At most one
of the two fires: each retry returns `None` unless its own level is on screen.

The resource workspace owns the service-specific list/detail content inside the
shared application frame.
Render-facing resource projections are built in src/app/projection.
The `project_resources` function
constructs a `ResourceWorkspaceProjection` with shared shell metadata and a
typed service-specific list or detail projection. In list mode, the list
projection contains the sorted visible rows; in detail mode, it remains empty
because the detail projection is the only rendered surface, while the shared
visible-row count still reflects the active filter. Detail projections borrow
typed detail data and errors from state rather than copying them. Resource
list/detail widgets consume these projections and do not inspect service state
to derive display values. Command and confirmation overlays remain outside
this list/detail projection boundary. Do not force every service into an
identical detail schema if the resource models are meaningfully different.

Detail-pane bodies are built one level below that boundary, in
src/app/projection/detail_body.rs. `project_detail_body` mirrors what the
renderer chooses to draw and returns a `DetailBody`: a `Vec<DetailLine>` where
each line pairs its display text with the optional `CopyableField` that line
carries. The renderer consumes `display_lines()` and never derives detail text
itself; the reducer consumes `fields()`, which lists every copyable line as
`(line index, payload)` in display order. Per-field copy selects over that
`fields()` vector with a single cursor, `detail_field_index`, on
`ResourcesState` (`n`/`p` move it, `y` copies the field under it). The cursor
indexes the `fields()` vector, not `lines`, and a stale cursor clamps to the
last field on both sides — the reducer resolves what `y` copies and the
renderer resolves which line to highlight through the same clamp, so the
highlighted line and the copied line are always the same line. Bodies live in
`src/app` rather than the renderer because the reducer must reason about
copyable values without depending on `src/ui`.

`ui::resources::render_detail` resolves that selected line once per frame and
hands it to whichever detail renderer draws the pane, which styles it with the
shared selected-row style. Keeping the selection on screen is a render-time
computation (`detail_scroll_for_selection`) layered on top of the stored
`detail_scroll`; the stored scroll is never rewritten, so leaving a selection
and coming back lands where the operator left it.

### VPC networking: three flat kinds, not a tree

VPC networks are the counter-example to the two sections above. A
network *looks* like the parent of its subnets, and the resource model
deliberately does not say so: `gcloud compute networks subnets list` and
`firewall-rules list` are flat, project-wide calls, so nesting either under a
network would be a display choice with no call behind it. Each is its own
`ResourceServiceKind` — the Pub/Sub shape, not the BigQuery one — which is
also what lets all three be jump *targets*, since a drilled level is not
addressable as one.

The networks table is list-only: `Detail` and `DetailTab` are both `()`,
because `compute networks describe` returns the fields the list call already
returned. Nothing is pushed to the server: `MODE`, `ROUTING` and `SUBNETS` are
each derived — from `autoCreateSubnetworks`, from `routingConfig.routingMode`
and from the *length* of the `subnetworks` array — so none of them is a
verbatim substring of a raw field, and narrowing stays wholly client-side.
`label_filter_field` answers `None` because the `compute/v1` networks resource
carries no labels at all.

The parser reads those API fields and never gcloud's synthesised
`x_gcloud_subnet_mode` / `x_gcloud_bgp_routing_mode`, which carry the very
strings the two columns paint. The `x_` prefix is gcloud declaring them its own
presentation layer; the pin that keeps a later edit from reaching for them
feeds the parser a row where the two disagree.

The URL-tail helper (`services::resources::vpc::resource_id`) lives in this
module for the two kinds that come after networks: a subnet's `region` and
`network`, and a firewall rule's `network`, all arrive as full self-link URLs,
and a second copy of the shortening is how two tables come to disagree about
what one network is called.

The firewall rules table is the third kind, and its two derived cells are the
reason it is worth reading the parser before the projection. There is no action
field on the wire: the API sends an `allowed` array or a `denied` array, never
both, and `FirewallAction::from_api` collapses which-one-arrived into a single
value at the parser boundary. Both of the shapes the API is documented never to
send — neither array, and both — resolve to `Unspecified`, which paints a dash;
guessing there would put `ALLOW` over a rule that blocks. The `disabled` flag
takes no column of its own and rides the identity cell as a badge instead,
because the identity column is the one `plan_table` may never drop and a
disabled DENY rule that lost its badge on a narrow terminal reads as a live
one.

`L` is refused for networks and for firewall rules, and offered for subnets
only. Cloud Logging attributes both flow logs and firewall hit logs to
`gce_subnetwork`: a rule's hits are logged against the subnetwork the traffic
crossed, with the rule named in the payload, so a rule-scoped query is
well-formed, cheap, and always empty. Offering it would turn "we cannot ask
this question" into "the answer is nothing", which on a DENY rule reads as
"nothing was blocked".

The shared resource table lives in src/ui/shared/table.rs and consumes a TableModel built in src/app/projection/table.rs.
Each configurable service's columns come from a single catalog in
src/app/projection/columns.rs: one catalog entry carries both a column's spec
and the function that extracts that column's cell, and the header row and every
data row are produced by one walk over the same slice, so a header and its cells
cannot desync. The Cloud Storage object browser is not a configurable resource
list and keeps its hardcoded columns in the projection. BigQuery's tables
drill-down is not configurable either — `[columns]` is keyed by
`ResourceServiceKind` and the drill-down has none — but it does keep a catalog
(`BIGQUERY_TABLE_COLUMNS`), because the header-and-cell-from-one-element
property is worth having whether or not a layout can move the columns.
Columns declare minimum,
preferred, and priority widths; plan_table hides the lowest-priority columns
first when space is tight and truncates cells with an ellipsis marker instead
of corrupting neighbors. The smallest priority value is protected from hiding,
and that is always NAME: every catalog gives NAME priority 0, and a configured
column list re-derives priorities from display position with NAME pinned to 0
wherever the user placed it (apply_display_priorities), so no configuration can
produce a table whose rows cannot be told apart. Status cells map to shared
StatusSemantic text/icon/color. A wide-columns toggle (w) reveals every column,
and the sort indicator (with reversible direction) plus a selected-column cursor
let users sort any column directly.

Services should be lazily initialized like `tgcp`, but initialization must be
keyed by active gcloud configuration as well as project. If the selected
configuration changes, initialized services either reinitialize immediately or
mark themselves stale and reload on next focus. This avoids showing data from
configuration `dev` while the header says `prod`.

## Safety For Operations

A k9s-style cloud TUI can easily become dangerous if destructive operations are
too quick.

Use explicit command workflows for operations such as:

- start or stop an instance
- delete
- restart
- scale
- change active project
- activate gcloud configuration
- create or edit gcloud configuration
- switch account through configuration properties
- modify IAM

Rules:

- destructive actions require confirmation
- gcloud configuration, project, and account context must be visible during confirmation
- commands should produce typed effect results
- failed commands should produce useful status, not panic
- command output should be captured and summarized in app state
- long-running operations should show pending/running/completed/failed state

The app should make impossible or unsafe workflow combinations unrepresentable
where practical. Prefer typed modal/workflow enums over multiple booleans.

### Marks and batch actions

A batch action applies one lifecycle action to the marked rows of the active
service. The pipeline is written once, generically, and is not aware of any
particular service.

**Marks.** Each service's row collection owns its own `marks:
BTreeSet<ResourceRowId>` plus a `mark_anchor`. `ResourceRowId { name, location }`
(`location` is the zone for GCE, the region for Cloud SQL) is positional-free
identity, so marks survive sorting, filtering, and list reloads instead of
tracking row indices. `marked_count` therefore counts rows currently hidden by a
filter too, and `unmark_visible` exists precisely so a user can clear only what
they can see. Marks are available on every service that *has* batch actions —
Compute Engine and Cloud SQL today. `toggle_mark`, `range_mark`, `clear_marks`,
and `unmark_visible` return early for GKE, Cloud Run, Cloud Storage, Secret
Manager, and Cloud Functions: those stay read-only until they gain lifecycle
actions, and a mark you cannot act on would be a dead control.

**The batchable-service seam.** `BatchableService` (`src/app/state/resources/batch.rs`)
sits on top of the `ResourceService` trait and adds only what a batch
needs: an associated `Action`, `wrap` (tag it as a `BatchAction` variant),
`is_eligible` (Cloud SQL consults availability as well as status, since
`Failover` needs a regional instance), `row_ref` (project a row into a
`BatchRowRef { name, location, status_label, command_preview }`), and
`skip_reason` (explain why an ineligible row was skipped). `BatchRowRef`
is why no batch type is generic over the row type: the confirmation modal, the
progress panel, and the runtime all consume the same pre-projected shape, so
there is one rendering path and one fan-out for every service.

`skip_reason` is the one hook with a default implementation, and the default is
a trap worth stating explicitly: it returns
`SkipReason::Ineligible(row_ref.status_label)`, which is the whole story only
for a service whose eligibility depends on status alone. **A service that gates
on anything else — availability, quota, tier — and does not override
`skip_reason` will report its skips as `ineligible (<status>)`, naming a state
that reads as eligible.** Cloud SQL overrides it for exactly that reason: a
zonal instance targeted by `Failover` is `Runnable`, so the default would have
rendered `ineligible (Runnable)`. Its override returns
`SkipReason::RequiresHighAvailability` (rendered
`ineligible (requires an HA (regional) instance)`) whenever the row's status
*would* have passed and only availability failed, and falls back to the status
label otherwise.

**The pipeline.**

```text
marks -> start_batch_confirmation::<S> -> BatchActionConfirmation (eligible / skipped, ack)
      -> Effect::RunResourceBatch { action: BatchAction, targets: Vec<(BatchRowRef, u64)>, concurrency }
      -> RuntimeSession::dispatch_resource_batch -> one BatchTargetRunner, one bounded semaphore
      -> per-target action results -> batch workflow -> one completion refresh
```

The `u64` beside each target is the id of the operations-log entry the
confirmation seeded for it. The confirmation builds that `(target, id)` list
once and hands the same list to the effect and to the batch-progress workflow,
so the two cannot disagree about which id belongs to which target, and each
target's result settles its own entry rather than whichever entry for that
instance happens to be pending.

Three safety properties hold for every batchable service, and are the reason the
pipeline is shared rather than copied per service:

- **Eligible-vs-skipped split.** `start_batch_confirmation::<S>` partitions the
  marked rows by `S::is_eligible`; each skipped row is carried with the
  `SkipReason` its service's `S::skip_reason` returns
  (`Ineligible(status_label)` by default, `RequiresHighAvailability` for a
  zonal Cloud SQL row targeted by `Failover`) and listed in the modal, so a row
  that will not be acted on is shown rather than silently dropped, with a
  reason that does not read as eligible. A confirmation with no eligible target
  never opens.
- **Typed-count acknowledgement.** A `BatchAction` whose `is_destructive()` is
  true gets `BatchAck::CountRequired`: the user must type the eligible-target
  count before the batch can be confirmed. Non-destructive actions get
  `BatchAck::Simple`.
- **Bounded concurrency.** `dispatch_resource_batch` selects the
  `BatchTargetRunner` once per batch and creates exactly one `Semaphore`, sized
  by the effect's `concurrency` (`0` means "substitute the session's configured
  default"). Cancellation is checked both before and after permit acquisition,
  and a target that loses its `BatchTargetClaim` to a cancellation landing after
  that second check bails too (see the batch-cancellation discussion below), so
  a batch detached by a context or scope switch starts no further work. Each
  target runs on `spawn_blocking` with a join-error fallback, so a target whose
  blocking task panics still resolves as failed instead of being stranded
  `Pending` and blocking the workflow from ever completing.

`apply_batch_effect_result` fires the one post-batch reload, derived from the
still-live workflow's `action()` (the refresh tears the workflow down, so the
action has to be read first) — `RefreshCloudSqlInstances` for a Cloud SQL batch,
`RefreshComputeInstances` for a Compute one. It matches the two-variant
`BatchAction`, deliberately *not* the five-variant `ResourceServiceKind` it maps
to, so the match needs no catch-all: a new variant is a compile error here
rather than a panic in the main run loop, which would abort the whole TUI.
Exactly one refresh per batch, after the last target resolves, not one per
target.

A batch result also names its own service. `resolve_batch_target(service, name,
location, status)` ignores a workflow another service owns, so a Cloud SQL
result — which identifies its instance by name alone — can never resolve an
identically named target of a Compute batch. For the same reason each service's
`mark_*_stale_and_clear` closes only the confirmations *it* owns
(`close_confirmations_owned_by`): the single-row and batch confirmation slots
are shared, but each teardown discards only its own service's rows. The batch
*workflow* slot is shared the same way and follows the same rule: a load result
dismisses a completed workflow only via
`clear_completed_batch_workflow_owned_by(<the service that loaded>)`, so a
Compute refresh landing mid-flight cannot dismiss a Cloud SQL batch's progress
panel before the Cloud SQL rows it is waiting on arrive, or vice versa.

**A detached batch settles its operations-log entries, at the strongest claim
it can prove.** Confirming a batch seeds one operations-log entry per eligible
target up front (`operations.begin(action.label(), target.name)` in the
`ConfirmBatchAction` arm of `src/app/reducer/resources.rs`), and each entry is
normally resolved by that target's action result. Detaching the batch removes
both ways an entry could ever be resolved: the token stops unstarted targets
from sending a result at all, and the generation bump drops the results of the
ones already running. So whatever `detach_and_settle_batch`
(`src/services/runtime/session.rs`) leaves `Pending` stays `Pending` for the
life of the process — detachment is the last moment at which anything true can
be said about these entries, and it says it.

A batch is detached on two paths, and both settle:

- **Cancellation.** A context or scope switch takes the batch away with nothing
  taking its place. `reduce`'s follow-up calls `cancel_batch`, which is
  `detach_and_settle_batch(BatchDetachReason::Cancelled)` followed by
  `clear_batch_workflow()`.
- **Supersession.** The operator confirms a second batch while the first is
  still running. `dispatch_resource_batch` calls
  `detach_and_settle_batch(BatchDetachReason::Superseded)` before it stamps and
  fans out its own targets. Nothing is cancelled from the operator's point of
  view — the outgoing batch simply loses every way it had to report, which is
  exactly the condition above.

`BatchDetachReason` is the *only* thing that differs between the two paths, and
it selects wording, never outcomes: an entry's outcome is what that target's
claim proves, and why the batch went away cannot change what was proved about
it. The reason exists because the entry is also read by a person. On the
supersession path the operator cancelled nothing — they confirmed a
replacement — so a detail reading "the batch was cancelled" would name the
wrong actor. Each reason therefore owns both of its detail strings
(`unissued_detail`, `issued_detail`), and the four are pinned as two pairs:
`tests/batch_cancel_evidence.rs` for the cancellation wording,
`tests/batch_supersession_evidence.rs` for the supersession wording, each
also asserting the other path's phrasing is absent so the two can never be
reworded into one.

That second path is why the settling half is split out. Tearing the
batch-progress workflow down is right when the batch goes away with nothing
replacing it and wrong when a new batch is taking its place: the reducer
installs the incoming batch's workflow in the same `ConfirmBatchAction` arm
that seeds its entries, before `Effect::RunResourceBatch` is dispatched, so a
clear inside the settling half would destroy the workflow of the batch the
operator just confirmed — no progress overlay for it, and no end-of-batch
reload, because `apply_batch_effect_result` reads the workflow to fire that
reload. While the two halves were one function the supersession path settled
nothing at all, and a second batch stranded the first one's entries `Pending`
for the life of the process.

Two terminal outcomes, chosen by what is provable rather than by what is
convenient:

- **`Cancelled`** — the target never issued its call. Nothing changed
  outside the app and there is nothing for the operator to re-check.
- **`Unknown`** — the call was already issued and its result was then
  discarded. GCP may well be carrying it out.

`Unknown` rather than `Cancelled` for a started target is the whole point.
"Cancelled" asserts *the command did not run*; rendering that next to an
instance GCP is in the middle of stopping is the UI-contradicting-reality
defect class this codebase rules against, and it is worse than saying nothing
because the operator would act on it. `Failed` is equally unavailable: the
command may have succeeded. So the `Unknown` entry's detail points at the
operator's next move — refresh this service to see the resource's actual state
— and deliberately names no key, because refresh is rebindable
(`RebindableAction::Refresh`) and a hardcoded key would be a second lie stacked
on the first. The Home card styles the two accordingly (`src/ui/home/mod.rs`):
`Cancelled` muted, `Unknown` on the stale/warning role, neither on the loading
role that would repaint a settled entry as work still in flight.

**Where the target is claimed is load-bearing, and the claim is a race, not a
flag.** Each in-flight target owns an `Arc<BatchTargetClaim>`
(`InFlightBatchTarget`) — an `AtomicU8` holding `Idle`, `Started` or
`Cancelled`. Its task claims it *after* the post-permit cancellation check and
*before* the `spawn_blocking` that issues the command. Neither side of that
point is safe: claim it earlier and a target that then bails at the check is
reported `Unknown` for work provably never done; claim it later — after
`spawn_blocking`, or once the command returns — and there is a window in which
the command is in flight but unclaimed, so a cancellation landing in that window
reports `Cancelled` for a command GCP is executing. That whole gate —
cancellation check, permit, cancellation check again, claim — is
`claim_batch_target_slot`, a named function rather than the opening lines of the
spawned task so that unit tests can drive the ordering directly: the window
between the permit arriving and the second check cannot be hit through the effect
loop, because the state that distinguishes it spans no await point.

Position alone is not enough, because the worker and `detach_and_settle_batch`
run on different threads of the multi-thread runtime. A flag the worker *sets* and the
cancellation *reads* is two observations of a moving value: the worker can be
descheduled between clearing its cancellation check and setting the flag, during
which the cancellation sees it unset, writes `Cancelled` — "the command did not
run" — and the worker then wakes and runs it. What removes that window is not
timing but mutual exclusion: both sides claim with a compare-exchange from
`Idle`, so exactly one succeeds and the loser learns it lost. The worker that
loses returns without issuing anything (a second, stricter bail than the check
above, which can only see a cancellation that already happened); the
cancellation that loses reports `Unknown` instead of `Cancelled`. Nothing here
leans on `CancellationToken`'s internal ordering. Per-target and lock-free
rather than a shared `Mutex<HashSet>` is also structural: with no guard, there
is no guard to accidentally hold across the task's await points.
`dispatch_resource_batch` replaces `batch_targets` wholesale per batch and
`detach_and_settle_batch` takes it, so a stale claim from an earlier batch can
never turn a plainly cancelled entry into a spurious `Unknown`.

**Each in-flight target carries its own operations-log id.**
`detach_and_settle_batch` never reads the batch-progress workflow, because the workflow is not guaranteed
to still be there: `mark_all_stale_and_clear` nulls it whenever a configuration
is activated, *including* when the new configuration resolves to the same
context and the reducer therefore takes no cancelling branch at all. A
cancellation that had to look its ids up on the workflow would then find nothing
and strand every entry `Pending` for the life of the process. Holding
`operation_id` on `InFlightBatchTarget` removes the dependency: the session
settles what it dispatched, whatever the reducer did to the view state in
between.

`detach_and_settle_batch` sweeps *every* target of the batch it detaches, not
just the unresolved ones — deciding which are unresolved is exactly the workflow
lookup being avoided. That is safe because resolution is pending-only (below): a target
that already reported its own outcome is left as it reported it.

Resolution is id-addressed and pending-only: `OperationsLog::resolve_by_id`
writes a record only while it is still `Pending`, so a real result that landed
first is authoritative and can never be overwritten by this sweep, and a
`(kind, target)` lookup can never settle an unrelated single-row action's entry
that happens to share both fields. Its return value is ignored on purpose — the
log is bounded and evicts from the back, so an id can outlive its record and a
miss is a no-op, not an error.

The reducer half of that contract — every minted entry settles once the result
carrying its id is processed, whatever the interleaving of single-row actions,
batch confirmations and configuration switches — is stated once as a property
over generated sessions in `tests/operations_settlement_property_test.rs`,
rather than as one more example test per stranding path. The runtime half, for
cancelled and superseded batches, stays pinned by
`tests/batch_cancel_evidence.rs` and
`tests/batch_supersession_evidence.rs`.

This deliberately changes Compute's observable operations-log output. An earlier
change held Compute to strict behaviour preservation and this gap was documented rather
than closed for exactly that reason; the cancellation work lifts that constraint, because a
correct Cloud SQL cancellation and a Compute one that strands entries `Pending`
would be two behaviours out of one shared path. It remains distinct from the
`spawn_blocking` join-error fallback above, which covers a target that *did*
start and then panicked while the batch was still live.

**Adding a batchable service.** The hard parts are genuinely shared and are not
duplicated per service: the confirmation state and its ack rules, the progress
panel, `Effect::RunResourceBatch` and the reducer's `ConfirmBatchAction` arm,
the fan-out in `dispatch_resource_batch`, and the three safety properties above.
What a third service does touch, verified against the code:

- `batch.rs`: one `BatchableService` impl, one `BatchAction` variant, and an arm
  in each of the four `BatchAction` methods that actually match on the variant:
  `label`, `is_destructive`, `service_kind`, and `service_label`. The
  user-visible strings that name a service — `title`, `modal_border_title`,
  `small_terminal_message`, `progress_small_terminal_message` — are `format!`ed
  from `service_label` and need no new arm, which is the point of routing them
  through it.
- `session.rs`: an arm in `BatchTargetRunner::for_action` and one in the
  completion-refresh match. Both match `BatchAction` with no default, so both
  are compiler-enforced.
- `resources/mod.rs`: the six mark-dispatch matches on `active_service`
  (`toggle_mark`, `range_mark`, `clear_marks`, `unmark_visible`, `marked_count`,
  `visible_marked_count`), which route Compute and Cloud SQL to the generic
  `*_for::<S>` helpers and no-op every other service — a missing arm silently
  leaves the service unmarkable rather than failing to compile. Plus its
  `mark_*_stale_and_clear` calling `close_confirmations_owned_by`, and an arm in
  `ResourceActionConfirmation::service_kind` if it also gains a single-row
  confirmation.
- `projection/frame.rs`: the mark-capable gate in `resource_list_title`, or the
  `Marked: n` segment never appears.
- `projection/mod.rs`: the service's table rows must set `marked` from state
  (`marked: state.is_*_marked(..)`) instead of the hardcoded `false` the
  read-only services use.
- `state/command.rs` and `reducer/command.rs`: its lifecycle palette
  descriptors, their context gating, and the arm that starts a batch
  confirmation when marks exist and a single-row one otherwise.
- `reducer/root.rs`: its action-result arms passing their `ResourceServiceKind`
  to `resolve_batch_target`.

## Error And Degraded-State Model

Represent external failures explicitly.

Important degraded states:

- `gcloud` binary missing
- native gcloud configuration list unavailable
- active gcloud configuration missing or unreadable
- not authenticated
- active project missing
- insufficient IAM permission
- API disabled
- quota/rate-limit failure
- network failure
- resource-specific API failure
- stale cache only
- partial service refresh

The UI should distinguish:

- loading
- healthy empty
- stale data
- degraded data
- failed operation

Do not collapse these states into a single "no resources" message.

See "Diagnostics classifier" above for how these degraded states are
classified (`GcloudErrorKind`), sanitized (`sanitize_detail`),
carried (`GcloudDiagnostic`), rendered consistently (`"{label} — {user
action}"` headline plus an inline bounded detail preview, never raw
stderr/Debug), copied (`Y` / `SystemAction::CopyDiagnostic`), and gated
against pointless auto-retry (`Retryability`, `RuntimeSession`'s background
retry gate).

## Testing Strategy

Use tests to protect boundaries that are easy to break in TUIs.

Recommended tests:

- domain tests for identity parsing and status normalization
- domain tests for gcloud configuration parsing and context projection
- reducer tests for selection, filtering, sorting, modal transitions, and resource reconciliation
- reducer tests for configuration activation/create workflows and stale resource invalidation
- keymap tests for normal and modal input behavior
- effect tests for success and failure result mapping
- service tests with fixtures for `gcloud config configurations list
  --format=json`, `compute instances list`, `compute instances describe`, and
  native start/stop action commands
- resource service tests that assert native gcloud command construction through
  a fake command runner
- projection tests for visible rows and selected detail data
- rendered-table tests that check configured columns against the painted
  terminal buffer, not just the projected model
- runtime cancellation tests for background task shutdown
- terminal/bootstrap tests where practical
- property tests for parsers or reconciliation logic when inputs can arrive in many shapes
- credential tests that drive `ConfigurationCredentials` through a fake
  `CommandRunner`: a token is minted once and reused while it stays valid, a
  token inside the one-minute expiry margin is refreshed rather than handed
  out, a different configuration name is never served the previous
  configuration's token, and nothing in the test reaches the network
- projection parity tests that build a row from a client model and the same row
  from the equivalent gcloud JSON and assert the two agree. The modules under
  `src/services/sdk/` reuse the CLI modules' `parse_status` and share
  `timestamp_to_string` for exactly this reason: one normalization, so a row
  cannot mean one thing on the native path and another on the gcloud path

Keep fixtures under `tests/fixtures/`.

Never require real Google Cloud credentials for normal test runs. Service tests
should use fixture JSON, fake command runners, or mock clients. The native
clients must never be constructed against a real endpoint in a test either: the
test harnesses leave `SdkClients` as `None`, so every read falls back to the CLI
implementations under `src/services/resources/` driven by a fake runner.

Native configuration tests should use a fake command runner. They must not
activate, create, or edit real local gcloud configurations during test runs.

A fake runner that parks a `spawn_blocking` thread must bound that wait. An
unbounded `Condvar::wait` there is uncancellable, so a test that panics before
releasing the gate leaves `Runtime::drop` blocked on it and wedges the whole test
binary: every result still prints `ok`, then the CI job dies on its timeout. Give
each gate a deadline and panic with a message naming the waiter and what it was
stuck on; format that message and drop the `MutexGuard` *before* panicking, or the
poisoned mutex buries the sibling waiters' own diagnostics under `PoisonError`.
The `gate_runner_*`, `blocking_list_runner_*`, and `gated_runner_*` tests in
`tests/runtime_batch_effect_test.rs`,
`tests/runtime_compute_action_effect_test.rs`, and
`tests/runtime_configuration_effect_test.rs` pin this.

Prefer a deterministic signal to a wall-clock bound: gate the fake runner and
release it from the test rather than sleeping and asserting on `elapsed()`. Where
no deterministic signal exists, a timing bound may only prove *this does not
hang* — keep the margin orders of magnitude above the real cost so a loaded CI
runner cannot trip it, and comment what the bound proves and why the margin is
safe.

Assert column alignment on the characters read back from the terminal buffer,
not on the projected `TableModel`. The model is only half the pipeline:
`plan_table` still assigns widths and `render_table` still pairs each cell with
a layout entry, so a defect confined to the render layer leaves the model
coherent while the screen shows a value under the wrong header. Bound the
anti-truncation check to the table pane rather than the whole buffer row — the
preview pane paints its own `Zone:` and `Status:` lines on those same rows, so
an unbounded search is satisfied by the preview while the table cell beside it
reads `asia-sou…`. The row *lookup* needs the same bound, not only the value
check. Keep the coverage multi-row and per-service: one row cannot catch a
defect that misaligns rows 2..n, and one service cannot catch a defect in
another service's catalog. The six
`*_paints_every_value_under_its_own_*_header` tests in
`tests/rendered_column_alignment_test.rs` pin this, one per covered table
surface and deliberately separate `#[test]` functions rather than one loop, so a
single surface's breakage is attributable. They do not yet reach every rendered
table, and BigQuery is not a special case. `render_table` has three call sites
(`ui/resources`, `ui/tunnels`, `ui/logging`), and the surfaces they draw with
no alignment case are: secrets, Cloud Functions, persistent disks, Pub/Sub
topics, Pub/Sub subscriptions, service accounts, the IAM policy, the BigQuery
datasets table, the BigQuery tables drill-down, the tunnels list and the Cloud
Logging table. Deliberately not a count: a number here would be a claim about
the whole binary maintained in a paragraph that cannot see it, and it goes stale
the moment a table is added or covered. The members are named instead, so the
next reader can check without searching, and finding one more makes the sentence
more true rather than false. The Cloud Storage object browser is
the load-bearing case: its headers (`storage_object_columns`) and its cells
(`storage_object_rows`, both in `src/app/projection/mod.rs`) are two
hand-maintained parallel arrays, whereas every other table draws both sides from
one walk over one catalog through `projection::columns::table_parts` and is
aligned by construction. Keep that case if the file is ever trimmed.

## Dependency And Tooling Practices

Standard verification commands:

```text
cargo check --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --locked
cargo deny check
```

Recommended baseline dependencies:

- `ratatui` for TUI rendering
- `crossterm` with event-stream support
- `tokio` for async runtime and background tasks
- `tokio-util` for cancellation tokens
- `clap` for CLI parsing
- `serde` and `serde_json` for gcloud/API JSON
- `thiserror` for typed service errors
- `anyhow` at process/runtime boundaries
- `tracing` and `tracing-subscriber` for file logging
- `dirs` for platform paths
- `tempfile` for tests
- `proptest` when parser/reconciliation behavior warrants it
- `cargo-deny` policy via `deny.toml`

Pin a Rust toolchain with `rust-toolchain.toml`, and keep CI aligned with local
verification commands.

## Documentation To Keep Current

The documentation in the tree, and what each file owns:

```text
README.md                    what nuvem is, how to install it, first run
docs/getting-started/        install, first run, reading the screen
docs/guides/                 task-shaped pages, one per workspace or workflow
docs/configuration/          config.toml, one page per table
docs/plugins/                the plugin surface and plugins.toml
docs/reference/              every key, flag, palette command and service
docs/troubleshooting/        error categories and common problems
ARCHITECTURE.md              this document: contributor-facing structure
CONTRIBUTING.md              development setup and the gates a change must pass
SECURITY.md                  how to report a vulnerability
CODE_OF_CONDUCT.md           the conduct expected of contributors
CHANGELOG.md                 what changed, release by release
docs/decisions/              numbered decision records, starting with
                             0001-native-sdk-for-read-paths.md
```

Keep the README user-focused. Keep this document and the decision records
contributor-focused.

When behavior changes, update the doc that owns that behavior in the same
change. A decision record is different in kind: it states what was decided and
why, so revise it only when the decision itself changes or the record turns out
to have been wrong, never to track how the code has drifted since. Documentation
is only useful if it describes the code that actually exists.

## Native Gcloud Configuration Workflow

`nuvem` is built around native gcloud configurations first.

Core commands:

- load configurations on startup
- refresh configurations manually
- activate an existing configuration
- create a new configuration
- set account/project/region/zone during create
- edit the active configuration context's project, region, and zone, unsetting
  blank submitted values

Recommended domain shape:

```rust
pub struct GcloudConfiguration {
    pub name: GcloudConfigurationName,
    pub is_active: bool,
    pub account: Option<AccountId>,
    pub project: Option<ProjectId>,
    pub region: Option<Region>,
    pub zone: Option<Zone>,
}

pub struct GcloudContext {
    pub configuration: GcloudConfigurationName,
    pub account: Option<AccountId>,
    pub project: Option<ProjectId>,
    pub region: Option<Region>,
    pub zone: Option<Zone>,
}
```

Activation flow:

1. user selects an inactive configuration in the configuration workspace or
   command palette
2. reducer stores a pending snapshot for activation confirmation
3. UI renders a centered confirmation overlay with target, account, project,
   region, zone, and the `gcloud config configurations activate` command it
   will run
4. `Enter` or `y` confirms only when the full preview fits; `Esc` or `n` always
   cancels without invoking gcloud
5. confirmation reducer emits `Effect::ActivateConfiguration`
6. runtime dispatches `Effect::ActivateConfiguration`
7. gcloud service runs the native activation command
8. runtime reloads the native configuration list
9. runtime accepts the reload only when the requested configuration is the sole
   active row; otherwise it uses `ConfigurationActivationReloadFailed` with the
   known target and a warning
10. effect result updates active configuration state
11. reducer invalidates config-scoped resource state

Create flow:

1. runtime dispatches `Effect::CreateConfiguration` with a draft
2. gcloud service creates the native configuration and sets provided properties
3. runtime reloads the native configuration list
4. reducer applies the reloaded native state or shows a warning if property setup
   or reload fails

The current implementation exposes this through a create form in the Configurations
workspace. Any explicit post-create activation choice remains future UI work.

Active context edit flow:

1. user presses `e` in the Configurations workspace while an active native
   configuration exists
2. reducer opens an edit form prefilled from the active configuration's project,
   region, and zone
3. while a project, region, or zone field is focused, Ctrl-Space opens a
   cancellable discovery picker scoped to the active configuration
4. picker selection updates only the local draft; a second Enter submits the
   draft
5. runtime dispatches `Effect::UpdateConfigurationContext` with the submitted
   draft
6. gcloud service runs scoped `config set` commands for nonblank values and
   scoped `config unset` commands for blank values
7. runtime reloads the native configuration list
8. reducer applies the reloaded native state, or preserves a fallback active
   context with a warning if the reload fails after native state changed
9. reducer invalidates config-scoped resource state when the active context
   changes

This flow edits only the active context's project, region, and zone. Editing
inactive configurations and account/name/auth fields remain future UI work.

Cache keys should include the active configuration name and project:

```text
configuration:<name>:project:<project>:service:<service>:resource:<kind>
```

This prevents resource rows, logs, and details from leaking between
configurations.

## Adding A New Google Cloud Service

The variant itself is declared inside the `service_kinds!` invocation in
`crates/nuvem-domain/src/gcp/resource.rs`, which generates
`ResourceServiceKind::ALL` from that same variant list. There is no separate
constant to extend, so a variant cannot be added in a way that leaves it off
`ALL` — the enumeration every per-service walk in the workspace iterates.

Then use this checklist:

1. Add a descriptor to the service registry, including its label, category,
   aliases, and identity.
2. Define availability and target behavior for the descriptor, including an
   unavailable reason or a typed resource target when appropriate.
3. Add typed resource state and effects/results when the service is live.
4. Add boundary tests for the descriptor, navigator, reducer, projection,
   effect dispatch, and renderer behavior as applicable.
5. Do not add Home or switcher branching for the service; the registry and
   typed `ResourceServiceKind` selection are the integration points.
6. Add domain types under `crates/nuvem-domain/src/gcp/`, **and assert their
   labels and `from_api` mappings from that crate's own tests**
   (`crates/nuvem-domain/tests/`), not from the root suite, so the crate's
   own test binary pins its behaviour and the domain crate can be checked
   on its own.
7. Add the read paths for the service in both backends. The native module goes
   under `src/services/sdk/` — the list and get calls against the official
   client library, the model-to-row and model-to-detail mapping, and the JSON
   tab rendered through `detail_json` — and its client type is registered in
   the `sdk_client_cache!` invocation in `src/services/sdk/mod.rs`, so the
   per-configuration cache builds and holds it like every other client. The
   CLI implementation goes under `src/services/resources/`, where it backs
   demo mode, the label-mode reads that must be answered by gcloud, and any
   context with no native clients configured.
8. Add or extend `ServiceEvent`, `Effect`, and `EffectResult`.
9. Make service loading accept an explicit `GcloudContext`.
10. Add reducer handling for loaded rows, selection, loading, and degraded states.
11. Ensure cache keys include configuration and project context.
12. Add stale-state behavior for gcloud configuration changes.
13. Add projection helpers if UI needs derived rows.
14. Add typed service-specific row/detail projections when UI needs derived display data.
16. Add projection tests for shared shell metadata and service-specific display values.
17. Add UI modules only for service-specific display differences.
18. Add keymap entries only for service-specific commands that make sense.
19. Add fake-runner service tests with fixture JSON for parsing, and a native
    mapper test that builds a client model and asserts the row.
20. Add reducer/projection tests for visible behavior.
21. Update help/footer text and architecture docs, including a line in the
    verified module tree for every new file under `src/` or
    `crates/nuvem-domain/src/`.

Do not add service behavior by letting widgets call `gcloud` directly.

Several of these sites answer `&[]` or `None` rather than failing to compile
when they are missed. `tests/service_kind_exhaustiveness_test.rs` walks `ALL`
over the ones the compiler cannot see — registry entry, load effect, column
catalog, navigator entry, jump fields, and collection isolation — and fails
once listing every unresolved `kind — site` pair, so its output is the
remaining worklist rather than one item per run. Sites whose per-kind dispatch
is an exhaustive `match` are deliberately absent from it; the compiler already
holds those.

## Definition Of Done For Early Features

A feature is not done until:

- state ownership is explicit
- IO is isolated behind services/effects
- native gcloud configuration context is visible and correctly scoped
- UI render code is pure relative to app state
- input mapping is mode-aware
- empty, loading, failed, and degraded states are represented
- tests cover the reducer or service behavior that changed
- footer/help text matches real key behavior
- verification commands pass
- docs are updated when the architecture or user workflow changes

## Shared Application Frame

The application has one shared frame around every normal workspace. Rendering
is split between application-level metadata, workspace content, and overlays:

```text
RootState -> AppFrameProjection -> shared header/breadcrumb/status/footer
RootState -> workspace projection/state -> body renderer -> full-frame overlays
```

`services::runtime` draws through `ui::render`. The top-level renderer first
handles the below-minimum terminal fallback. Otherwise it projects the root
state, splits the terminal into frame regions, renders shared chrome, renders
the active workspace body, renders the shared status and footer, and finally
applies the active global overlay.

## Shared frame ownership

`ui::render` owns frame orchestration. `ui::shared::frame::split_frame`
allocates five vertical regions:

- a two-row header;
- a one-row breadcrumb line;
- a flexible body;
- a two-row operation status area; and
- a one-row footer.

At exactly `60x12`, the fixed regions leave six rows for the body. The shared
frame owns all of these regions; workspace renderers receive only the body
`Rect` and must not render application chrome into it.

`ui::shared::frame` renders the shared regions from `AppFrameProjection`:

- The header shows the `nuvem` title and view title, followed by the active
  gcloud context. Configuration and project are prioritized; account, service,
  region, and zone are added when width permits.
- The breadcrumb line shows the projected navigation path.
- The status area shows the operation phase, label, optional status message,
  and a live "Updated: {age}" relative-time readout (for example "just now",
  "12s ago", "3m ago") computed from the last-refresh timestamp and refreshed
  by a 1s redraw tick, alongside the existing loading/refreshing/stale/degraded
  phases.
- The footer shows the projected key/action hints, and pins `:` (command
  palette) and `?` (help) right-aligned beside them. See "Pinned footer keys".

`ui::shared::layout` owns the terminal-too-small message. Shared reusable
formatting and widgets live in `ui::shared::components`; shared semantic
colors live in `ui::theme`.

## Projection boundaries

`RootState` is application state. It contains the current workspace, command
palette state, configuration state, resource state, and system operation state.
It is not a render tree and does not own terminal regions.

`app::projection::project_app_frame` is the boundary for application chrome.
It maps root state to `AppFrameProjection`, including the view title,
breadcrumbs, active context, service label, operation state, refresh time, and
footer hints — the view's own in `footer_hints`, the pinned globals in
`footer_pinned_hints`. Shared frame code consumes this projection without reaching into
workspace-specific state.

`app::projection::project_resources` is the resource-workspace boundary. It
maps typed resource state to display-ready service rows, detail data, filtering
and sorting metadata, loading/stale state, and the active detail mode. The
resource renderer does not need to know how those values were loaded or
reduced. Configurations currently render their state directly in
`ui::configurations`; if a future configuration projection is introduced, it
should end at the workspace body boundary in the same way.

Render-facing resource projections are built in src/app/projection. Add typed service-specific row/detail projections when UI needs derived display data.

## Workspace ownership

Workspace modules own body content only:

- `ui::home` renders the Home overview body inside the supplied body
  rectangle, using the home overview projection.
- `ui::resources` renders a list body for every resource service — one match
  arm per `ResourceServiceKind` — and a detail body for the six services that
  have a detail pane: GCE instances, GKE clusters, Cloud SQL, Cloud Run,
  Cloud Storage, and Secret Manager. The rest are list-only, showing a preview
  line in place of a detail pane. Both bodies draw inside the supplied body
  rectangle, using the resource projection.
- `ui::configurations` renders configuration lists, create/edit forms, and
  configuration-specific content inside the supplied body rectangle.
- `ui::logging` renders the Cloud Logging list and entry-detail bodies inside
  the supplied body rectangle, using the logging projection.
- `ui::tunnels` renders the managed-tunnel list body inside the supplied body
  rectangle, using the tunnels projection. It reads the clock once and passes
  that `Instant` into `project_tunnels`, so the projection stays pure and its
  AGE column is testable without sleeping.

Workspace code may own body layout, list blocks, rows, empty/loading/stale
states, and detail content. It must not duplicate the header, breadcrumb,
operation status, or footer, and it must not treat the whole terminal as its
content area.

Two modals draw their chrome from a workspace module even though they are
centered over the whole frame rather than the body rectangle: the configuration
context picker and the related-resource jump picker. That split — workspace-owned
chrome, `ui::render`-owned decision to draw — is described once, under "Overlay
precedence". Global overlays otherwise remain the responsibility of
`ui::render`.

To add a future workspace, add its state and reducer/input behavior, add any
workspace projection in `app::projection`, add its view metadata and footer
hints to `project_app_frame`, and branch in `ui::render`. Its renderer should
accept `(frame, body_area, session)`, render only within `body_area`, and reuse
`ui::shared::components` and semantic theme functions. This keeps new
workspaces inside the existing application chrome instead of duplicating it.

## Overlay precedence

After the normal frame and workspace body have been rendered,
`render_active_overlay` checks overlays in this order and returns after the
first match:

1. help;
2. context picker;
3. resource-action confirmation (shared across GCE and Cloud SQL actions);
4. batch-action confirmation (likewise shared across batchable services);
5. configuration-activation confirmation;
6. plugin confirmation (a plugin's argv, awaiting the operator — its `Enter`
   runs, which is why it outranks the copy-only preview below it);
7. command preview (the GCE SSH command line);
8. GCE SSH launch preview;
9. k9s launch confirmation;
10. tunnel launch preview;
11. the related-resource jump picker (`J`);
12. command palette; and
13. the in-flight batch progress panel, which is last because it is a
    render-only glance rather than a mode: it never claims input, and shows
    only once nothing above it claims the frame.

The tunnel launch preview sits where it does deliberately. It is a launch
confirmation like the two entries above it, so it belongs with them rather than
ahead of the confirmations that guard cloud mutations; and it must outrank the
command palette, because the palette is one of the ways it is opened — a
palette that could paint over the preview it just produced would hide the argv
the user is being asked to approve.

The jump picker sits directly below it for the same reason the launch modals do:
it is a modal that owns the keyboard while it is up, so its rank here has to
match its rank in `services::runtime::input_context` — whichever overlay renders
is the one that must own the keys. It stays ahead of the command palette because
the palette must never paint over a modal that is currently claiming input.

These are full-frame overlay responsibilities because they receive the full
terminal area and render above the already-rendered frame. Help is a
full-screen replacement: it paints its help content over the full terminal
without using `Clear`. Fitting modal overlays use a centered modal area and
`Clear` only for that area before rendering semantic modal chrome. The first-
match rule is the precedence contract; new global overlays must be inserted
deliberately into this order and must not be rendered from a workspace body
renderer.

The configuration context picker and the related-resource jump picker are the
split cases. Their state, their chrome, and their picker-local controls belong
to a workspace module — `ui::render` calls
`configurations::render_context_picker_overlay` and
`resources::render_jump_picker_overlay` rather than drawing either — but the
*decision* to draw them is made here, in the first-match chain, so they obey
the same precedence contract as everything else in the list.

## Semantic styles

Use functions from `ui::theme` by visual role rather than inline colors:

- `app_title` and `view_title` identify application and view titles;
- `border` and `focused_border` distinguish ordinary and focused containers;
- `selected_row` and `inactive_selected_row` identify selection state;
- `active_tab` and `inactive_tab` identify detail tabs;
- `status_healthy`, `status_loading`, `status_stale`, and `status_error`
  identify operation/state feedback;
- `muted` de-emphasizes secondary metadata;
- `key_hint` emphasizes footer keys; and
- `overlay_border` identifies modal overlays.

Shared helpers such as `bordered_block`, `modal_block`, `status_line`,
`selected_row_style`, and the loading/stale/error helpers should be preferred
so new UI follows the same semantic mapping.

## Responsive rules

The global minimum is `60x12`. If the terminal is narrower than 60 columns or
shorter than 12 rows, `ui::render` draws only the framed
`nuvem needs at least 60x12` message and returns. No workspace, shared frame,
or global overlay is rendered in that case.

At `60x12` and above, the shared frame is always allocated. Text is fitted to
its cell width with `fit_text`; the header prioritizes configuration and
project, and optional context fields yield when space is exhausted. Body
content must use the supplied flexible area and fit its rows to that area.

Overlays have additional content-fit rules after the global minimum check
above. Help requires `73x22` and is rendered as a full-screen replacement
without `Clear`; only after the below-`60x12` fallback has been ruled out, a
terminal smaller than `73x22` shows the help-specific full-screen fallback
message. Thus, that help fallback applies only to terminals at least 60 columns
by 12 rows but smaller than 73 columns by 22 rows. Fitting confirmation and
SSH preview modals clear only their centered modal area. If either modal's
content cannot fit, its small-terminal fallback clears the full terminal and
shows a larger-terminal fallback message.

"Cannot fit" is measured after wrapping, not before. Every line of a
confirmation or preview is wrapped to the modal's inner width by
`ui::mod::modal_lines`, and the fit test is then purely a row count. Before that
each line had to fit the inner width on a single row, and the ninety-column
`Command:` line of a GCE lifecycle confirmation therefore refused to draw below
a frame of about 115 columns — so an operator on an 80x24 or 100x30 terminal
could neither confirm the action nor read what they would have confirmed, and
the action was unavailable. Refusing to confirm a command the operator cannot
see is the rule and it is unchanged; wrapping is what lets that rule be kept
without also refusing commands they could perfectly well read across two rows.

The wrap is `ui::shared::components::wrap_exact`, which only ever *splits*:
`wrap_exact(text, w).concat() == text` for every input, stated over generated
ones by `tests/wrap_exact_property_test.rs`. Deliberately not the prose wrapper
in `ui::shared::help`, which normalises and collapses runs of whitespace — right
for a paragraph, and wrong for a line carrying the exact argv a confirmation is
about to run.

The batch *progress* panel is the one modal still measured unwrapped. Its gate
reads only the first three lines and lets the rest run past the bottom, so
widening what counts as fitting would let it open on a terminal too short to
show the `+N more` line that says how many targets are not listed — trading an
honest refusal for a panel that under-reports a running batch. The command palette and context
picker likewise use `Clear` for their fitting modal areas; the context picker
shows a resize fallback in that area when its modal chrome cannot fit.

## Home Overview Dashboard

The Home workspace is a read-only dashboard: context health, per-service
inventory, and a recent-operations log. It follows the same projection
boundary discipline as the resource workspace and never triggers loads on its
own.

`app::projection::project_home_overview` (`src/app/projection/home.rs`) is the
render-facing boundary for Home. It maps `RootState` into a
`HomeOverviewProjection` containing:

- `context`: the shared `FrameContextProjection` reused from
  `project_app_frame`, so Home shows the same context summary as the header;
- `gcloud`: a `GcloudHealthProjection` built from
  `SystemState::diagnostics` — availability, active account, a formatted
  problem string, and whether a probe is currently in flight;
- `inventory`: one `InventoryCardProjection` per `ResourceServiceKind::ALL`
  entry, in that enum's own declaration order rather than from a hand-written
  list — that list is how three services shipped with no Home entry at all —
  so Home shows one line per service (GCE instances, GKE clusters, Cloud SQL,
  Cloud Run, Cloud Storage, Secret Manager, Cloud Functions, Persistent disks,
  Pub/Sub topics, Pub/Sub subscriptions, BigQuery datasets, Service accounts,
  IAM policy, VPC networks, VPC subnets, Firewall rules), each with a `ResourceFreshness`
  (`NotLoaded`, `HealthyEmpty`, `Loaded`, `Stale`), a row count, status counts
  where the service has a lifecycle status, and an optional formatted problem;
  and
- `operations`: one `OperationLineProjection` per entry in
  `SystemState::operations`, most-recent-first.

The health card answers for the `gcloud` CLI and nothing more. Its probe runs
`gcloud auth list` scoped to the active configuration (see *Bounded background
probe*), so it reports whether the binary is reachable and whether that
configuration has an active account. It does not exercise the native clients: a
read that fails because an API is disabled, a permission is missing, or a token
could not be minted never darkens the health card. That failure surfaces on the
service's own inventory line instead, through the `GcloudDiagnostic` the failed
load stored in `ResourcesState::service_diagnostic`. A healthy context card
above a problem line is a consistent reading of both, not a contradiction.

Inventory freshness and counts are derived entirely from already-loaded
resource state, through the kind-taking accessors
`ResourcesState::has_loaded_list`, `service_stale`, `service_row_count` and
`service_status_counts` — each exhaustive over `ResourceServiceKind`, so a new
service has to answer them before it compiles. The count comes from
`service_row_count` rather than from summing `service_status_counts`, because
a service with no lifecycle-status shape (Cloud Storage, Secret Manager) sums
to zero however many rows it holds and the freshness rule would read that as
`HealthyEmpty`. Projecting Home never issues an `Effect` and never reads
anything that is not already sitting in `RootState`; if a service has never
been loaded, its line simply shows `NotLoaded` instead of triggering a fetch.

`src/ui/home` owns rendering the Home body inside the supplied `body_area`,
same as any other workspace renderer: a context card, an inventory block, and
an operations card, each skipped when the allotted area is too small to be
useful. It does not render shared frame chrome and does not reach past the
`HomeOverviewProjection` into raw state.

The inventory is one compact line per service in a single bordered block, not
one bordered card per service in a column split. The split shape drew two cards
and discarded every entry past the second with no panic, warning or test, and
no tuning of it holds seven services at 80 columns. The block asks the layout
for one row per service plus its borders and yields what it cannot get to the
operations card; where that leaves lines undrawn, the last row reads `+N more`,
so painted lines plus the counted remainder always equal the registry. A
service line carries the label, its freshness, then either its status counts or
its problem headline — the headline never yields to the counts — and appends
the sanitized detail preview only when all of it still fits on the row.

### Operations log and diagnostics state

`SystemState` (`src/app/state/system.rs`) owns two pieces of Home-relevant
state:

- `operations` (`src/app/state/operations.rs`) is a bounded, most-recent-first
  `OperationsLog` of in-app operations (currently GCE and Cloud SQL mutations,
  single-row and batch alike, plus plugin runs). `begin` records a new
  `Pending` entry and evicts the oldest entry once the log exceeds its fixed
  capacity and returns its id.
  `resolve_by_id` is the only resolver: it settles one specific record, and
  only while that record is still `Pending`. Every producer of an outcome holds
  the id `begin` handed it — the action effect carries it to its result, and
  each batch target carries its own — because a `(kind, target)` match is
  ambiguous whenever a batch and a single-row action are in flight over the
  same instance, and would settle whichever entry was begun last. The same
  resolver carries the two outcomes no action result ever produces —
  `Cancelled` and `Unknown`, the terminal states of a cancelled batch (see the
  batch-cancellation discussion under "Batch actions" for which target gets
  which, and why). This keeps the operations log independent of the
  resource/effect machinery that originally triggered each mutation.
- `diagnostics` (`src/app/state/diagnostics.rs`) is `OverviewDiagnostics`, a
  small state machine (`NotRun` -> `InFlight` -> `Complete`) around the last
  `GcloudDiagnosticsReport`. `begin` marks a probe in flight before it is
  dispatched; `complete` records the report and returns the phase to
  `Complete`. The reducer uses `is_in_flight` as a no-refanout guard so a
  second probe is never queued while one is outstanding.

### Diagnostics classifier

`nuvem_domain::gcp::diagnostics` (`crates/nuvem-domain/src/gcp/diagnostics.rs`)
defines `GcloudErrorKind`, `classify_gcloud_stderr`, `Retryability`,
`sanitize_detail`, `GcloudDiagnostic`, `DiagnosticProblem`, and
`GcloudDiagnosticsReport`. This is domain code with no UI or runtime
dependencies, so the Home diagnostics probe, the resource loaders, and any
service-specific error surface all reuse the same classifier, sanitizer, and
carrier to explain a degraded state consistently instead of showing raw
gcloud stderr or a `{error:?}` Debug string.

**Taxonomy.** `GcloudErrorKind` has 12 variants, each mapped to one
acceptance category: `MissingExecutable` (gcloud CLI not installed/on PATH),
`Unauthenticated` (expired/missing credentials), `MissingProject`,
`PermissionDenied`, `ApiDisabled`, `Quota` (rate limit), `Network`
(unavailable/timeout), `InvalidInput`, `InvalidQuery` (unparseable
filter/query, shared with the Cloud Logging viewer from issue #30), `Parse`
(schema drift), `Cancelled`, and `Unknown` (fallback). `classify_gcloud_stderr`
does case-insensitive
substring matching over gcloud stderr text, most-specific-first.
`MissingExecutable`, `Cancelled`, and `Parse` are never text-classified —
they are known from the error's origin (CLI spawn failure, cancellation
token, JSON parse error), not from stderr content. Each kind carries a
`label()` (e.g. `"permission denied"`) and a `user_action()` — a concise,
imperative remediation hint (e.g. `"Check IAM roles for this account and
project"`, `` "Run `gcloud auth login` to reauthenticate" ``).

**Retryability.** `pub enum Retryability { Transient, Persistent }`, via
`GcloudErrorKind::retryability()`. `Transient` (`Network`, `Quota`,
`Unknown`) is safe for the background poll ticker to retry automatically.
`Persistent` (everything else — auth, permission, config, parse, cancelled,
missing-executable) is a config/logic error that a bare retry cannot fix;
auto-retrying just hammers gcloud. See "Background retry gate" below for how
`RuntimeSession` consumes this.

**Sanitization chokepoint.** `pub fn sanitize_detail(text: &str) -> String`
is the single function that guarantees the anti-leak acceptance criterion:
it redacts credential-shaped tokens with `[redacted]`, collapses whitespace,
and bounds the result to `DETAIL_MAX_LEN` (200 bytes) with a trailing
ellipsis. A whitespace-separated token redacts when either branch of the
private `looks_like_credential` fires. The first is the token's own value
shape: a `Bearer` prefix, a `ya29.` substring, JWT-looking `eyJ...` runs,
`AIza...` API keys, `1//...` refresh tokens. The second is a key/value pair
whose key *names* a credential: at any occurrence of a word in
`CREDENTIAL_KEY_WORDS` (`access_token`, `refresh_token`, `id_token`,
`api_key`, `apikey`, `password`, `secret`, `authorization`), matched against a
hyphen-normalised copy of the token so `--api-key=zzz` reads the same as
`--api_key=zzz`, the rest of the key and an optional quote are consumed, a
separator (`=` or `:`) is required, and the value after its own optional quote
must be non-empty. A token that merely *mentions* a credential word carries
nothing and survives: an operator-authored plugin label like
`Describe db-password`, templated against a Secret Manager row name, reaches
the UI intact, and `password=` with no value stays legible.
Every surface that shows a technical detail string to the user — the
Home health card and inventory lines, the resource detail pane, the clipboard
copy text —
goes through this chokepoint; nothing renders raw stderr or a `Debug`
`AppError` directly.

**`GcloudDiagnostic`** (`kind`, `service`, sanitized-and-bounded
`command_summary`, `exit_code`, sanitized-and-bounded `detail`) is the
canonical carrier built once per failure, in the reducer (`build_diagnostic`
in `src/app/reducer/root.rs`),
from an `AppError`. It exposes:

- `headline()` — `"{label} — {user action}"`, the recovery hint that is
  always visible wherever a degraded state is shown (the Home health card and
  its inventory lines, resource list/detail/context-picker degraded states,
  the shared
  status area). This replaced the older `"{label}: {message}"` shape so
  every surface reads consistently.
- `detail_preview()` — the sanitized, bounded one-line technical detail,
  shown inline alongside (never instead of) the headline, only where the
  rendering surface has enough width (see "Consistent rendering" below); no
  full collapsible overlay panel.
- `copy_text()` — a multi-line sanitized block (service, label, command,
  exit code, action, detail) for the copy-diagnostic key.

**Known gap: Cloud SQL action failures lose command summary, exit code, two
error-kind hints, and detail budget.** Both the single-row path and every batch
target funnel through one function per service in
`src/services/runtime/session.rs`, and the two do not build their `AppError` the
same way. `run_instance_action_result` (Compute) converts with `error.into()`
and then attaches `"gcloud compute instances {verb}"` via
`AppError::with_command_summary`. Its Cloud SQL twin,
`run_cloud_sql_action_result`, builds `AppError::gcloud(error.to_string())`,
which flattens the error to its rendered text. Relative to Compute a failed
Cloud SQL action therefore loses four things, not one:

- **`command_summary`** — never attached, so the "which command was this" line
  is missing from the headline, the detail preview, and `copy_text()`.
- **`exit_code`** — `AppError::gcloud` hardcodes `None`, whereas
  `From<CommandError>` carries `exit_code` through from `CommandError::Failed`.
  `copy_text()` pushes its `exit code {code}` line only when `exit_code` is
  `Some`, so the line is not rendered blank — it is absent entirely from every
  copied Cloud SQL action failure, while the Compute equivalent has it.
- **`kind_hint` for `MissingExecutable` and `Cancelled`** — `From<CommandError>`
  sets those two hints from the variant itself. Without a hint, `AppError::kind()`
  falls back to `classify_gcloud_stderr`
  (`crates/nuvem-domain/src/gcp/diagnostics.rs`), which has no rule matching
  either variant's message, so both classify as `Unknown`. A cancelled Cloud SQL
  action reports "Retry, or copy the details to investigate" instead of "The
  operation was cancelled; retry when ready", and a missing gcloud binary does
  not say to install it.
- **Detail budget** — for `CommandError::Failed`, `From<CommandError>` uses the
  raw diagnostic as the detail — `failure_diagnostic(stdout, stderr)`, which is
  stderr unless the child left it blank, as `bq` does on every call nuvem makes
  of it — whereas `error.to_string()` renders the variant's
  `Display` and so prefixes it with `"command exited unsuccessfully: "`. That
  31-byte wrapper is noise the user did not need, and it is counted against
  `DETAIL_MAX_LEN` (200 bytes) by `sanitize_detail`, so long Cloud SQL stderr is
  truncated 31 bytes earlier than the identical Compute stderr would be.

The classified kind for a genuine `CommandError::Failed` (the common case) is
unaffected, and no detail is leaked unsanitized — the loss there is budget and
noise, not correctness. This is a known asymmetry, not a design decision.
Closing it is more than a one-liner: there is no
`impl From<CloudSqlLoadError> for AppError`, so the fix has to match the
`Command`/`Parse` variants the way `load_cloud_sql_instances` in the same file
does — but *not* convert the way it does. That function's `Command` arm also
builds `AppError::gcloud(error.to_string())`, i.e. exactly the lossy shape
described above; the action path should instead pass the inner `CommandError`
through `.into()` (preserving `exit_code`, `kind_hint`, and the raw stderr
detail) and then attach the command summary. Worth doing when Cloud SQL
diagnostics are next touched — including for the load paths.

`DiagnosticProblem` (`kind`, `message`) is the older, narrower carrier kept
for the gcloud availability/auth probe (`GcloudDiagnosticsService::probe`,
which predates `GcloudDiagnostic` and still stores a raw, unsanitized
message); `impl From<&GcloudDiagnostic> for DiagnosticProblem` bridges the
two so existing accessors (`ResourcesState::compute_problem`/`gke_problem`)
keep working. Any projection reading a `DiagnosticProblem.message` directly
(the Home health card) re-sanitizes it through `sanitize_detail` before
display, since that field is not guaranteed sanitized at the source.

**Consistent rendering (AC #5/#6).** `HomeOverviewProjection`'s
`GcloudHealthProjection` and `InventoryCardProjection`
(`src/app/projection/home.rs`) both carry `problem: Option<String>` (the
headline), `user_action: Option<&'static str>`, and
`detail_preview: Option<String>`, sourced from
`ResourcesState::service_diagnostic` for inventory cards. `src/ui/home`
renders the headline unconditionally. The health/context card adds the detail
preview as a second, muted line only when that card's area is at least
`DETAIL_PREVIEW_MIN_WIDTH` columns wide — the same responsive-width-guard
pattern used elsewhere in the UI (e.g. `WIDE_LAYOUT_MIN_WIDTH`). An inventory
service line has no second line to spend, so it appends the preview inline
after the headline and only when the whole of it still fits on the row; the
headline takes the row's columns first and a preview that would be cut is not
started. The resource list/detail views and the configuration context-picker's
degraded state render the same `diag.headline()` from the reducer's
`build_diagnostic`, so every surface — status area, Home, resource
list/detail, context picker — shows the identical `"{label} — {user
action}"` shape.

### Pinned footer keys

The footer is one width-budgeted row and the binding catalog reaches 35
bindings in the resource list alone, so it is curated per view rather than
complete — the help overlay (`?`) is what is complete. What the footer must
never drop is the pointer to it. Until this pass `? Help` appeared in exactly
one footer in the application (Home) and `:` in none, so leaving Home lost both
the overlay that teaches every key of the current view and the palette that
runs every command by name.

`PINNED_BINDINGS` (`src/app/projection/binding.rs`) is those two keys and only
those two. Every other global is either situational (`Y` has a diagnostic to
copy or it has not) or destructive (`q`, `Ctrl-C`), and a permanent hint costs
width in every view at once. They are projected into
`AppFrameProjection::footer_pinned_hints`, separate from the view's own hints,
and `ui::shared::frame::render_footer` lays them out right-aligned, with
whatever width they end up taking removed from the budget the view hints are
then fitted into.

`fitted_pins` gives the pins an allowance and never more. Normally that
allowance is the slack the view's own hints leave over, so a row with room for
both keeps both, a row with room for one keeps help, and no view hint is ever
paid for a pin that could have fitted in the space going spare. The exception
is a row the view's hints fill on their own: there they are going to be
truncated whether the pins are present or not, so the pins take a bounded share
of it (`PINNED_HINT_SHARE`, a third) rather than collapsing to nothing for a
truncation that was happening anyway. Within the allowance they yield before
they take — dropped from the front, palette first and help last, and the
survivor gives up its label before it gives up itself, so a cramped footer ends
in a bare `?` rather than in nothing. Help is last out because it is the pin
that repays the space: it names the overlay that lists every key the view has,
the palette's included. Home's own hint list therefore no longer carries
`GLOBAL_HELP` — listing it twice cost the width that keeps `q Quit` on screen
at the 60-column minimum frame size.

`footer_pinned_hints` is empty while a mode owns the keyboard
(`frame::a_mode_owns_the_keyboard`): under any overlay, in a filter, or in a
form, `?` and `:` are characters or do nothing, and a hint is a promise about a
key. That predicate mirrors the overlay ladder in `ui::render_active_overlay`
and the text-entry sub-views each `*_metadata` builder early-returns for; it is
deliberately not derived from `transient_footer_hints`, which covers only the
subset of those modes that has a specialised footer of its own.

**Palette rows name their key.** `binding::command_key` answers "which key runs
this command in the view underneath the palette", and
`ui::render_command_palette` renders it as the row's right-hand column, so a
command looked up by name once is a key learned. It answers two questions and
both matter: whether the command has a key at all (most do not — the lifecycle
actions and field copies are palette-only by design), and whether that key
dispatches *here*, which is decided against `context_bindings` rather than
against a hand-kept list, so a key taught in a palette row is one the
keymap-parity harness has already proved reachable in that context.
`binding_for_command` is exhaustive with no catch-all: a new descriptor cannot
be added without answering the first question.

**Copy key.** `Y` (`SystemAction::CopyDiagnostic`, global reach like
`r`/`?`) copies `SystemState::last_diagnostic().copy_text()` — the sanitized,
bounded technical detail — to the clipboard via the existing
`Effect::CopyToClipboard`, so users can share troubleshooting text without
the UI ever holding or transmitting raw stderr. With no diagnostic recorded,
it sets a "No diagnostic to copy" status message instead of emitting an
effect. The binding is registered once in the catalog
(`GLOBAL_COPY_DIAGNOSTIC` in `src/app/projection/binding.rs`) so the help
view's Global section always teaches it; the footer (a single
width-budgeted row) only shows it when there is a diagnostic to copy, and
always appends it last so it is the first hint dropped on a narrow
terminal — it never displaces higher-priority hints like navigation
history.

**Background retry gate (AC #4).** `RuntimeSession::poll_active_resource`
(`src/services/runtime/session.rs`) consults a per-scope gate
(`gated_scope: Option<(ResourceServiceKind, GcloudContext)>`), set whenever
a `ComputeInstancesLoadFailed`/`GkeClustersLoadFailed` result for the active
scope has `Retryability::Persistent`. While gated, the poll ticker silently
skips scheduling another background refresh for that `(service, context)`
scope — it does not keep hammering gcloud on an error a retry cannot fix.
`Transient` failures are not gated and keep retrying every tick exactly as
before this existed. Manual refresh (`r` / `ResourcesAction::Refresh`) and
any context or active-service switch always proceed regardless of the gate,
and both clear it. The gating decision itself is a pure, unit-tested
function, `should_auto_retry(Option<Retryability>) -> bool`, so the
threading-sensitive `RuntimeSession` state only has to record the last
failure's retryability and ask the pure function what to do with it.

### Bounded background probe

The gcloud availability/auth probe behind Home's context-health card runs
through `RuntimeSession::dispatch_diagnostics_probe`
(`src/services/runtime/session.rs`), following the same bounded-background-work
pattern as the context picker's discovery calls:

- a `tokio::sync::Semaphore` with a single permit (`diagnostics_slots`) bounds
  the probe to one in-flight run; `dispatch_diagnostics_probe` uses
  `try_acquire_owned` and returns immediately if a probe is already running,
  so pressing `r` on Home repeatedly cannot queue multiple probes;
- the acquired `OwnedSemaphorePermit` is moved into the spawned blocking task
  and dropped when the task finishes, releasing the slot; and
- the dispatched probe captures the current `context_generation` and tags its
  queued result with it. `process_next_effect_result` only applies a
  `Diagnostics` result if the captured generation still matches the session's
  current `context_generation`; a result for a context the user has since
  navigated away from is silently dropped instead of overwriting the newer
  context's diagnostics.

Together, the semaphore, the `OverviewDiagnostics::is_in_flight` guard in the
reducer, and the context-generation check keep the probe to at most one
useful in-flight run per active context, matching the "no-refanout" budget
used elsewhere in the runtime.

## Adaptive Resource Layout

`ui::resources::WIDE_LAYOUT_MIN_WIDTH` (`100`) is the body-width threshold at
which the Resources workspace switches from a full-screen single pane to a
split layout: a left list pane and a right preview/detail pane
(`split_wide`, 55%/45%). The check runs against the body `Rect` width, so it
only applies once the shared frame's `60x12` global minimum is already
satisfied; below `100` columns of body width (including the `60x12` minimum),
Resources renders full-screen exactly as before this feature, in both list and
detail view mode.

In the wide layout, the left pane always renders the resource list — even
while in Detail view mode, where a list-only projection is derived from
session state so the left pane keeps showing rows alongside the open detail.
The right pane renders one of two things depending on view mode:

- List view mode: a summary preview of the currently selected row
  (`project_resource_preview`), built from the same already-loaded row data
  as the list itself.
- Detail view mode: the full tabbed detail view, the same content that would
  otherwise occupy the whole body.

Neither pane triggers an extra service call to populate the wide layout: the
list, the preview, and the detail all read from resource state that a normal
(narrow) refresh or detail load already populated. The wide layout is purely
a rendering-time split of already-loaded data, not a new data-loading path.

## Cloud Logging Viewer

`Workspace::Logging` is a top-level workspace, a peer of Home, Configurations,
and Resources rather than a Resources sub-view. Press `l` from anywhere to
open it, or reach it through the command palette's `Open project logs` entry
(aliases `logs`, `logging`). Both are project-wide: `l` never consults the
selection.

Resource-scoped reads have one entry point in the keymap (`L`, in the resource
list and the resource detail pane) and one per service in the command palette.
The palette entries are not a hand-written list: `CommandDescriptor::all`
expands `OpenLogsForSelected(ResourceServiceKind)` over
`ResourceServiceKind::ALL`, and each label reads `Open logs for selected
{selection_noun}` from the same noun the rest of the app uses for a selection,
so a service added to the registry cannot be left without its palette entry
(whether that entry is offered is still the per-service gating in
`context_commands`). The one
exception is `OpenLogsForSelectedCloudRunRevision`, declared on its own because
a revision is a sub-view of a Cloud Run service rather than a
`ResourceServiceKind` and so has no member of `ALL` to be generated from. Both
routes call the single resolver
`app::reducer::resources::selection_log_query`, so the key and the palette can
never disagree about what "the selection" is: the resolver answers from the
*view* (a nested Cloud Run revision beats its parent service, a browsed Cloud
Storage bucket beats the bucket-list cursor, an open detail pane beats the list
cursor still movable behind it) rather than from the raw list cursor. It
returns either a `LogQuery::Resource` carrying concrete resource identity
(instance ID; cluster name and location; `<project>:<instance>` database ID;
service/revision and region; bucket name; secret name and project; function
name and region) or the message explaining what was missing, which the caller
sets as an error status — a jump that cannot resolve never switches workspace
and is never silent.

The query scope is therefore never a free-typed string.
`LogQuery::render_filter` is the single place that turns query + resource
identity into the filter clause used by every read, and its exact output is
shown in the workspace header (project-wide reads, which carry no filter
clause, show a `(all recent project logs)` placeholder). Each
`LogResourceKind`'s rendered filter is pinned by an exact-string test: a wrong
filter does not error, it returns zero rows, which on screen is
indistinguishable from "this resource has no logs".

### Read-only log-read service

Log reads go through `SdkLoggingService` (`src/services/sdk/logging.rs`), which
calls the Logging API's `entries.list` with the credentials of the active
gcloud configuration. Logging is the one native read path whose filter is
pushed to the server, and the reason is that the expression this application
builds *is* the API's own filter language: `gcloud logging read` forwarded it to
the same `filter` field this client sets, so nothing is translated or dropped on
the way down.

It exposes two operations, whose semantics are exactly the ones the CLI path
established:

- `read_recent`: a fresh page, used when there is no cursor yet or when the
  time range or the severity floor changes. The `--freshness` window has no API
  equivalent, so the selected bucket becomes a `timestamp>=` clause instead.
  Entries come back newest-first and the service reverses them to ascending
  (oldest -> newest) before returning, so the buffer convention
  `entries.back() == newest` (and therefore the tail cursor) holds everywhere.
- `read_since`: an incremental page filtered by `timestamp>="<cursor
  timestamp>"`, used for tailing once a cursor exists. The bound is inclusive,
  so no entry sharing the cursor's timestamp can be skipped; the ones already
  buffered are discarded by the insertId dedup.

Both take a `severity_floor` and AND its `severity>=<LEVEL>` clause into the
filter (see *Narrowing the window* below), so the narrowing happens server-side
rather than over an already-fetched page. Both backends assemble that filter
through the same `compose_filter` helper in
`src/services/resources/logging.rs`, so a clause cannot be composed one way on
one path and another way on the other. An entry with no `insertId` is counted
as dropped rather than returned on either path, because a tail cannot
deduplicate it.

`GcloudLoggingService` (`src/services/resources/logging.rs`) is the same two
operations over `gcloud --configuration <config> logging read`, behind the same
`CommandRunner` seam used by every other CLI resource service. It is what demo
mode serves fixtures through and what a session with no native clients falls
back to, and it is fake-runner testable without a live `gcloud`.

Neither backend can do anything but read: the native path calls only
`entries.list`, the CLI path invokes only `logging read`, and neither has a
path to mutate log sinks, metrics, or exclusions, which stay out of scope.

JSON parsing (`parse_log_entries_json`) is partial-tolerant: each array
element is parsed independently, and an entry that is missing required fields
(for example `insertId`) is counted as `dropped` rather than failing the
whole page. A page with some malformed entries still surfaces the entries
that did parse, plus a `PartialParse { dropped }` status instead of an error.

### Bounded polling tail

The Logging workspace reuses the existing poll tick (`Config::poll_interval_secs`)
rather than adding a second timer. `RuntimeSession::poll_active_tail`
(`src/services/runtime/session.rs`) is called every tick alongside
`poll_active_resource` and delegates the decision to the pure
`should_tail` (`src/services/runtime/tail.rs`), which is unit-testable
without async plumbing. `should_tail` skips the tick unless: the Logging
workspace is active, the tail is `Running` (not paused), no background
refresh is already in flight (`pending_background_refreshes`, the same
single-slot counter the Resources poller uses), an active gcloud context
exists, and the quota backoff counter has reached zero. With no cursor yet it
requests a first page (`LoadFirstPage`); with a cursor it requests an
incremental tail (`Tail`).

`pending_background_refreshes` bounds Logging (like Resources) to at most one
in-flight `gcloud` call at a time, so pressing keys or repeated ticks cannot
queue overlapping requests.

`LoggingState` (`src/app/state/logging.rs`) owns the tail's data structures:

- **Cursor boundary**: the cursor is the newest entry's timestamp plus every
  insert id sharing that timestamp (GCP log timestamps are not unique), so
  the next `read_since` call cannot re-fetch entries already seen at that
  timestamp.
- **insertId dedup**: `entries` (a `VecDeque`, oldest to newest) and `seen`
  (a `HashSet<insert_id>`) are mirrored in lockstep. An incoming entry is
  appended only if its `insert_id` is not already in `seen`.
- **Ring-buffer cap**: after appending, entries are evicted from the front
  until the buffer is at most `LOG_BUFFER_CAP` (1000); an evicted id is
  removed from `seen` too, so it is treated as unseen if it ever reappears.
- **Skipped-tick quota backoff**: `record_error` doubles `backoff_ticks`
  (starting from at least 1) on a `Quota` error, capped at
  `LOG_BACKOFF_MAX_TICKS` (8); other error kinds leave backoff untouched.
  `decrement_backoff` drains exactly one tick per poll call regardless of the
  tail decision, so a backoff of N produces N fully skipped ticks before
  tailing resumes.

### Shared error classification

Logging failures are classified with the same `classify_gcloud_stderr` and
`GcloudErrorKind` (`crates/nuvem-domain/src/gcp/diagnostics.rs`) used
elsewhere in the app, including issue #31's degraded-state work — there is no
parallel Logging-specific error classifier. `GcloudErrorKind::InvalidQuery`
was added to this shared enum for malformed Logging filter syntax, so a bad
filter is reported the same way permission, API-disabled, quota, network, and
missing-project failures are everywhere else in the app.

### Untrusted content sanitization

Log text originates from user/application-controlled log payloads, not from
`gcloud` metadata the app trusts. `sanitize_log_text`
(`crates/nuvem-domain/src/gcp/logging.rs`) strips control characters (mapping
tab to a space and dropping newlines, carriage returns, and other control
characters) from every rendered text field derived from `textPayload` or
`jsonPayload`, so untrusted log content cannot inject terminal control
sequences or otherwise alter the rendered UI.

### Status model

`LoggingStatus` distinguishes: `Initial` (never loaded), `Loading` (fetch in
flight), `Loaded` (has rows), `HealthyEmpty` (fetched successfully, zero
rows), `Degraded { kind: GcloudErrorKind }` (fetch failed, last-known entries
stay visible), and `PartialParse { dropped }` (some entries in the last page
failed to parse). `record_error` sets `Degraded` without clearing existing
entries, so a transient failure does not blank an otherwise-populated tail:
the projection keeps rendering the loaded rows and surfaces the degraded kind
only in the header status, showing the degraded body message solely when the
buffer is empty.

### Narrowing the window: severity floor and since buckets

Both narrowing axes are pushed into the `gcloud logging read` call rather than
applied to an already-fetched page, so a narrower view fetches less instead of
hiding more. Each is a `LoggingAction` whose reducer arm re-emits
`Effect::LoadRecentLogs` only when the window actually moved.

**Severity floor.** `LoggingState.severity_floor` is a `LogSeverity` that `s`
raises and `S` lowers one rank at a time. `LogSeverity::Default` is the
"no floor" sentinel: it contributes no clause, so an unnarrowed read's argv
stays byte-identical to what it was before the floor existed, and the header
shows it as `all` rather than as the severity name `DEFAULT`. Any other value
contributes `severity>=<LEVEL>`, ANDed in by `compose_filter` alongside the
query's own clauses. The steps saturate (`raise` at `EMERGENCY`, `lower` at the
sentinel) and `set_severity_floor` reports whether the floor moved, so a
clamped key press neither clears the buffer nor triggers a refetch that would
return exactly the same entries.

**Since buckets.** `LogTimeRange` is seven presets, declared shortest first;
that declaration order is both the `T` cycle order and the digit order:

| Key | Since | `--freshness` |
| --- | ----- | ------------- |
| `0` | 1 minute | `1m` |
| `1` | 5 minutes | `5m` |
| `2` | 15 minutes | `15m` |
| `3` | 30 minutes | `30m` |
| `4` | 1 hour (default) | `1h` |
| `5` | 6 hours | `6h` |
| `6` | 24 hours | `24h` |

`LogTimeRange::from_bucket_digit` maps a raw key press to its bucket and yields
`None` for anything else (including non-ASCII digits), so `7`-`9` stay unbound
rather than snapping to a nearest bucket; `bucket_digit` is its inverse and is
what the header prints beside the label (`Range: 1h [4]`), which is how the
direct-addressing keys stay discoverable. `label()` is the `--freshness`
argument verbatim, so the header shows exactly what is sent to the CLI.
Re-selecting the active bucket is a no-op, for the same reason a clamped
severity step is.

### Search grammar and why its default is literal

`/` opens the client-side search over the buffer, parsed by the log surface's
`TextFilter` (`crates/nuvem-domain/src/gcp/text_filter.rs`): a bare term is a
case-insensitive substring, `~term` a fuzzy subsequence, `/pattern/` a
case-insensitive regex, and a leading `!` inverts whichever mode was selected.
`visible_entries` matches it against each entry's message, severity label, and
resource type.

The grammar is only discoverable from the header, so the header carries it:
with nothing applied it spells the sigils out
(`Search: none  (text | ~fuzzy | /regex/ | !invert)`), and once a pattern is
applied that hint gives way to the mode the pattern actually selected
(`Search: [ERROR] (substring)`), so a typed sigil is confirmed rather than
guessed at.

That the log viewer's default is a literal substring — where the resource
table's filter picks its own default for its own surface, and where the k9s
convention is to default to regex — was a deliberate ruling, not an accident.
Searching for a bracketed level such as `[ERROR]` is one of the most common
log searches, and under regex `[ERROR]` is a character class: it would
silently match every line containing an `E`, an `R`, or an `O`, and would
never match the brackets that were typed. A wrong-but-plausible result set is
worse than an empty one, so the reading that cannot surprise is the default
and regex is the explicit opt-in.

The two grammars now share their machinery without sharing that ruling.
`crates/nuvem-domain/src/gcp/matcher.rs` holds the primitives both of them
compile down to — a case-insensitive `Matcher` (`Everything`, `Substring`,
`Fuzzy`, `Regex`) and a `Predicate` that pairs one with the inverse flag — so
`!` is implemented exactly once and cannot come to mean one thing on the log
surface and another on the resource table, and a regex that fails to compile
reports through one shared one-line reason helper on both. `text_filter.rs`
and `filter.rs` are left as what they always should have been: parsers that
decide *which* primitive an input selects. **That shared layer is not a step
towards one default — the defaults stay distinct deliberately.** An
unprefixed table filter is a regex, an unprefixed log search is a literal
substring, for the `[ERROR]` reason above, and `tests/grammar_reconcile_evidence.rs`
asserts both surfaces side by side so that a later "simplification" into a
single default fails a test instead of quietly changing what operators' log
searches mean.

`TextFilter` is compiled once per apply, not once per use: `LoggingState`
holds a private `CompiledTextFilter` pairing the applied source with the
filter it compiled to, and `visible_entries` — which runs on every render,
selection resync, copy and export — reads that cache instead of re-parsing.
`regex::Regex` implements neither `PartialEq` nor `Eq`, which the state
derives, so the newtype hand-writes both over its source alone; that is sound
only because its fields are private and the constructor is the only way to
build one. The projection needs no compile at all: it names the mode from the
pure-syntax view. Everything filter-derived is read back through accessors on
the state (`applied_filter`, `applied_filter_error`) rather than through
public fields, so nothing can drift out from under the cache. A pattern that
fails to parse hides every row (showing everything would silently contradict
what was typed) and the cache's error is what the header reports instead of a
mode.

### Saving the visible entries

`Ctrl-S` dumps exactly what the viewer currently shows. The reducer renders
`LoggingState::visible_export_lines` — the filtered entries, oldest first, one
`<timestamp> <SEVERITY> <resource_type> <message>` line each, every
log-derived field passed through `sanitize_log_text` — and emits
`Effect::SaveLogEntries { lines }`. An empty result emits no effect at all
(rather than an empty file) but still sets a status message, so the key press
is never silent.

The runtime resolves the destination as a `log-dumps` directory beside the
app's own log file (`Config::log_file`'s parent), and
`log_export::write_log_dump` writes `logs-<timestamp>.log` there, creating
missing parents and returning an absolute path so the reported location does
not depend on the process's working directory. That writer takes its
directory, filename timestamp, and already-rendered lines as arguments and
knows nothing about app state, config, or clocks, which is what makes it
testable against a plain temp directory.

## Managed Tunnels

`Workspace::Tunnels` lists the long-lived children nuvem starts on the user's
behalf: `gcloud compute start-iap-tunnel`, an `ssh -N -L` local forward, and
the Cloud SQL Auth Proxy. Everything below exists to keep one promise — a
tunnel child is never leaked and never stuck.

### Why a tunnel is not an interactive process

The app already runs children: GCE SSH and k9s go through
`Effect::LaunchInteractiveProcess` and the seam described under
"Interactive-process launches". A tunnel does not fit there, for two reasons
that are properties of the child rather than of the code.

An interactive child *owns the terminal* for its whole lifetime, and only the
run loop — which holds `ratatui::DefaultTerminal` — may hand the terminal over
and take it back. That is why the interactive seam is synchronous inside the
run loop and why it can only ever have one launch outstanding. A tunnel
**never owns the terminal**: it is a silent forwarder whose entire output is a
line stream the app reads and classifies. It also
**outlives the keystroke that started it** — the user starts it, keeps browsing
resources, switches configurations, and kills it minutes later — where an
interactive launch is over before the next frame is drawn.

Extending `InteractiveProcessRunner` to cover this would have meant teaching
it two lifetimes and two terminal disciplines at once, and every existing
caller would have paid for a concurrency model it does not need. A separate
seam keeps the interactive path exactly as simple as it was.

### The four layers, and what each may not do

- `nuvem_domain::gcp::tunnel` is pure planning: which kind of tunnel a
  resource needs (an instance with no external IP is only reachable through
  IAP, so it gets an IAP tunnel; anything with a public address takes the
  cheaper `ssh -L`), the exact argv for each kind, the default local and remote
  ports, whether a kind announces its own readiness, and how one line of child
  output classifies. It touches no filesystem, no network, and no
  `std::process`. Everything fragile about a tunnel — argv shape, banner
  wording, error detection — is therefore exercised by unit tests instead of by
  running real tunnels.
- `services::runtime::tunnel` owns supervision: spawning, the output stream,
  local-port allocation, and the shutdown primitive. It makes no policy
  decisions about what a tunnel *is*; it spawns a `TunnelSpec` the domain built
  and feeds lines back to the domain classifier.
- `app::state::tunnels` owns what the app believes is running. No method there
  reads the clock — `started_at` is supplied by the caller at insert time — so
  every transition is deterministic and the AGE column is testable without
  sleeping.
- `ui::tunnels` renders and nothing else, from the tunnels projection, inside
  the body rectangle it is given.

The supervisor reports through the **existing** effect-result channel; there is
no second event path. Tunnel results are the one result kind exempt from the
context/operation generation guards: those guards exist to drop stale data for a
view the user has navigated away from, and a tunnel is not a view. It is a
process addressed by an id that is never reused. Gating its terminal status on
the active context would strand a row in `Starting` with nothing left that could
ever resolve it.

### Why `TunnelControl::shutdown` is synchronous

`TunnelControl::shutdown` blocks, takes an explicit grace, and returns an
outcome. It is not `async`, and that is the load-bearing decision in the whole
subsystem: **`Drop` cannot await**. An async shutdown awaited from a destructor
would simply never run, and every tunnel would survive the app that started it.

Two paths, one synchronous primitive:

- graceful quit shuts every registered tunnel down from `spawn_blocking`, so
  the terminate-wait-kill sequence completes without blocking the reactor;
- `TunnelRegistry`'s `Drop` calls the same shutdown directly, as a best-effort
  blocking **backstop** for the paths quit does not cover — a panic, an early
  return, a torn-down session.

Because a destructor must not linger, the grace on the drop path is short and
the sequence is bounded by construction: signal, poll up to the grace, escalate
to a kill, then a brief reap window. Nothing in it can wait indefinitely.

Neither is the guarantee left resting on controls never panicking. The registry
shuts down in id order, so an unwind escaping one control would abandon the rest
of the loop and leave every higher id registered and never signalled — the leak
this subsystem exists to prevent — and from `Drop` it would unwind out of a
destructor, which aborts the process when a panic is already in flight. Each
per-control shutdown therefore runs under `catch_unwind`, and a caught unwind is
recorded as a `Failed` shutdown carrying the panic's reason: the loop continues
to every remaining tunnel, and that control stays registered (see "A stop that
fails keeps its handle") so quit or a later `Drop` can try again. No shipped
`TunnelControl` panics today; this is what keeps "never leaked" from resting on
that staying true.

### The kill runs off the run loop

Stopping a tunnel is dispatched, never awaited inside `dispatch_action`. The
sequence behind `TunnelControl::shutdown` is `SIGTERM`, poll up to the grace,
`SIGKILL`, reap — seconds of blocking for a child that ignores the polite
signal. Awaiting that from the action path would hold the run loop across the
whole sequence, so the frame that shows the row as `Stopping` would not be
drawn until after the child was already gone and every keystroke in between
would queue up behind it. **Do not "simplify" this back to an inline await.**

Nothing new was built to move it: the reducer already marks the row `Stopping`
before it emits `Effect::StopTunnel`, so the UI is honest from the first frame,
and the outcome comes back as the same `TunnelStopped` / `TunnelFailed` result
the supervisor already sends through the existing effect-result channel. A
second `K` during that window is a no-op — `kill_selected` only acts on a
`Starting` or `Active` row.

Dispatching it that way makes the blocking task the *only* thing that can still
resolve the row, so its join is awaited (from a spawned task, never from the run
loop) and a join error is turned into a terminal `TunnelFailed` carrying the
error. Otherwise a stop that panicked would leave the row in `Stopping` forever:
not terminal, so `d` refuses to dismiss it, its port stays in `claimed_ports`
for the rest of the session, and the `Starting | Active` guard refuses a second
`K` — nothing short of a restart. `Failed` rather than `Stopped` because a
panicked stop means the child's fate is genuinely unknown, which is the same
reason a reported `TunnelShutdown::Failed` fails the row. No shipped
`TunnelControl` panics today; the handling is what keeps "never stuck" from
resting on that.

The one hazard this creates is a leak, and it is closed by *not* deregistering:
the stop clones the control out of the registry and leaves it registered for the
whole in-flight window. Deregistering first — which is what
`TunnelRegistry::shutdown` does — would hand the blocking task the only
remaining handle to that child, so a quit or a `Drop` during the stop would find
an empty registry and reap nothing. Two callers reaching the same control is
safe: `shutdown` is idempotent by contract, and the second one gets
`AlreadyExited`. The cost is that a stopped tunnel's control stays registered
until quit, which is already true of a tunnel that exits on its own.

### Why the kill targets the process group

On unix the child is spawned into a session of its own and shutdown signals the
whole **process group**, not the child. `gcloud` is a **wrapper script** that
execs Python: signalling the direct child alone would reap the script and leave
the real tunnel running, which is exactly the leak the registry exists to
prevent.

Process groups are a unix concept. The non-unix arm falls back to killing the
**direct child**, which cannot reach anything that child spawned, so a non-unix
build cannot promise the no-leak invariant. That fallback is deliberately loud
rather than quietly degraded: it warns at spawn time and again at shutdown, and
its doc comment says plainly that it is **best-effort**.

### The port probe is advisory, not a reservation

Local-port allocation prefers a conventional port for the SQL proxy (5432,
3306, 1433 by engine), walks a short way up from it when taken, and otherwise
asks the OS for an ephemeral port. Ports already handed to live tunnels are
tracked separately from what the OS reports free, because a tunnel that is
still starting has not bound its port yet and the OS would happily call it
available twice.

The probe is **advisory, not a reservation**. Every answer is a snapshot: the
probe binds a listener and drops it again, so
**between the probe and the child's** own `bind()` another process can take
the port. There is no way to
hand an already-bound socket to `gcloud`, `ssh`, or `cloud-sql-proxy`, so the
race is irreducible.

The port the allocator picked is written into `TunnelsState`
**before the child is launched**, as its own `TunnelPortAllocated` result rather
than folded into readiness. A starting child has not `bind()`ed yet, so the OS
still reports its port free; `claimed_ports` is the only thing that stops a
tunnel started seconds later — while the first one is still coming up — from
being handed the same port. Publishing the allocation after readiness would
leave that window unprotected for the whole multi-second startup, which is
precisely when a user starts a second tunnel. The status is untouched by that
result: knowing the port is not the same as being up.

It is therefore surfaced rather than hidden, and what the user sees depends on
the kind:

- An IAP tunnel or the SQL proxy has not reported ready when it tries to bind,
  so its bind error is fatal on sight: the row lands in `Failed` carrying the
  child's own `stderr`.
- An `ssh -L` forward is ready the moment it spawns, and `ssh` on its own would
  print `bind [127.0.0.1]:5432: Address already in use` and **keep running** —
  leaving an `Active` row on a forward that **carries nothing**. Two things
  prevent that. The argv carries `-o ExitOnForwardFailure=yes`, so a forward
  that cannot bind actually terminates; and a bind failure is in the
  always-fatal tier below, so the row lands in `Failed` carrying that line
  whether or not the child gets around to exiting.

The app does not silently retry on another port behind the user's
back: "the port I asked for was taken" is the true and useful message, and a
tunnel that quietly moved would leave every client pointed at the wrong place.

### Why readiness detection is bounded

IAP tunnels and the SQL proxy print a banner when they start listening, so the
supervisor waits for it; `ssh -N` prints nothing at all on success, so a healthy
spawn is the only signal that will ever arrive and it is reported ready
immediately.

Waiting for a **banner** is waiting on someone else's string. Banner wording
drifts across `gcloud`, `ssh`, and `cloud-sql-proxy` versions, so a classifier
that recognises today's phrasing will eventually meet a version that phrases it
differently. The supervisor therefore arms a single **readiness deadline** for
the whole startup — created once, never re-armed per line, so a chatty child
that never comes up cannot push it out forever. When it expires the tunnel is
marked `Failed` with the last captured `stderr` lines and the child is shut
down. It **never waits forever** on a banner this version does not print.

This is the same reasoning as the **bounded-gate** rule in the Testing Strategy
section: an unbounded wait does not fail, it hangs, and a hang carries none of
the information a failure does. The deadline is a parameter of the supervisor
rather than a constant, so tests drive it deterministically at a small value.

### Two tiers of error line

The line classifier is pure — "what does this line look like?" — and the policy
of what to do about it lives in the supervisor, in two tiers, because one rule
cannot serve both halves of a tunnel's life.

**Before readiness, every error line is fatal.** The classifier is deliberately
liberal there: a bare `error` or `failed` anywhere in a line is enough. That
liberality is the same defence as the readiness deadline — a banner this
version does not print must not hang the tunnel in `Starting` forever.

**After readiness, only the always-fatal tier still fails a tunnel.** That tier
is narrow on purpose: it is the lines that mean the tunnel
**cannot carry traffic** at all — a bind failure (`bind [`…, `bind: `…, address
already in use), a permission or IAP denial, a host-key or authentication
failure. Nothing
in it can be printed by a child that is working.

The broad markers stay out of it, because a healthy child prints them.
A working `ssh -L` prints
`channel 3: open failed: connect failed: Connection refused` for a single
refused forwarded connection, and `gcloud` prints `...failed.` spinner text;
tearing the tunnel down for either would kill a forward the user is actively
using. So the broad tier is fatal **only before readiness**.

The always-fatal tier is also the *only* error detection an `OnSpawn` kind ever
gets. `ssh -N` prints nothing on success, so an `ssh -L` forward is reported
**ready from the instant it spawns** — `!is_ready` is never true for it, at any
point in its life, and without tier one a forward that could not bind or could
not authenticate would show `Active` forever with the reason never displayed.

A broad-tier line on a ready tunnel is not silently discarded either. It is
sent as a notice (`TunnelNotice`) — a diagnostic, not a verdict. The notice
writes the line into the row's **MESSAGE column**
and leaves the status exactly where it was, and it is logged at `debug`. The
alternative — keeping it only in the supervisor's output tail — makes it
unreachable, because that tail is read only by the readiness-deadline arm, which
cannot run once the tunnel is up: a healthy-then-broken tunnel would end
`Stopped` with no diagnostics at all.

### Late results, and why terminal is terminal

The supervisor runs in a task of its own, so a result it sent just before the
child died can land after the row has already reached `Failed` or `Stopped` — a
`TunnelReady` racing the kill path's `TunnelStopped` is the ordinary case, not a
corner. **`Failed` and `Stopped` are terminal is terminal**: no result reopens
them.

Applying a late result to a terminal row would flip a reaped tunnel back to
`Active`; the row would lie, its port would stay **promised to the allocator**
forever, and the row would be **undismissable** because a live tunnel may not be
dismissed. `Stopping` is not terminal either, so moving a terminal row into it
does the same damage.

The guard is one accessor rather than a check repeated at call sites: every
mutator on `TunnelsState` goes through `live_entry_mut`, so it cannot be
forgotten. `apply_exited` makes the same decision from the reducer, where the
*reason* also depends on the status the row was in.

### The `SIGTERM`/`SIGHUP` backstop, and why `SIGINT` is not in it

Quit and a panic were already covered (`shutdown_all_tunnels` and
`TunnelRegistry`'s `Drop`). A signal was not, and it used to be harmless:
foreground children shared the terminal's process group and the OS reaped them.
A tunnel child is deliberately put in a session of its own so the whole group
can be signalled — which also means the terminal's own `SIGHUP`
**never reaches them**. Closing the terminal with tunnels open would strand
children holding loopback ports. The run loop therefore selects on `SIGTERM`
and `SIGHUP` and routes them through the same `QuitRequested` path the `q` key
takes, rather than growing a second shutdown sequence to keep in step.

`SIGINT` is deliberately excluded. The terminal is in **raw mode**, so `Ctrl-C`
arrives as an ordinary key event and is already handled by the **keymap**;
installing a handler for it would give one keystroke two different quit paths.

A backstop that could not be installed does not stop the app from starting: it
warns, and its `received()` future then stays pending forever, so the `select!`
arm simply never wins instead of spinning.

### A stop that fails keeps its handle

`TunnelControl::shutdown` can report `Failed` — a child that could not be
signalled, or that was still there after the grace. The row is then marked
`Failed` carrying that reason, not `Stopped`.

`Failed` is not chosen for being less terminal than `Stopped`: both are
terminal, so either way the port is released and `d` dismisses the row. What it
buys is that the control **stays registered**, so quit and the `Drop` backstop
can still reach a child that is probably still alive, and that
**a stop that did not work** says so in the MESSAGE column instead of reporting
a clean stop the user would believe. The residual is real and worth stating: in
the window between the failed stop and whatever finally reaps the child, the
released port can be handed to a new tunnel while the old child still holds it —
the new tunnel then fails to bind and says so, which is worse than a clean stop
and better than a row that lies.

### Tunnel state is not context-scoped

Resource rows are scoped to the active gcloud context and dropped when it
changes, because they are a cached view of that context. Tunnels are
deliberately **not** context-scoped. A tunnel is a child process, and it keeps
running when the user activates a different configuration; clearing the list on
a context change would **hide a live process** and leave nothing in the UI that
could stop it.

Entries therefore survive context switches, and each one records the
`GcloudContext` it was started under rather than borrowing whichever context
happens to be active now — so the list can always say which project a running
forward actually points into. Terminal entries (`Failed`, `Stopped`) release
their local port for reuse but stay in the list until the user dismisses them:
the failure message is the entire reason the row is worth showing.

# Configuration

nuvem reads an optional, versioned TOML configuration file for personal
presentation and navigation preferences: refresh cadence, the startup
service, sidebar visibility, theme, extra service aliases, rebindable
hotkeys, each service's table columns and default sort, and the
related-resource jumps `J` offers. Nothing in this file can run a command.
Executable declarations live in a separate file,
[`plugins.toml`](plugins.md), which nuvem reads only from your platform
config directory (or an explicit
`--plugins <path>`) and never from the directory a `--config` path points at —
so a `config.toml` you were handed, synced, or checked into a repository cannot
bring commands with it.

**Location:** `dirs::config_dir()/nuvem/config.toml` —
`~/.config/nuvem/config.toml` on Linux,
`~/Library/Application Support/nuvem/config.toml` on macOS. Override the path
with `--config <path>` (useful for testing or per-project configs).

**Precedence** (lowest to highest): built-in defaults < config file <
CLI flags. Every field in the file is optional; an absent field keeps its
built-in default, and a CLI flag (`--poll-interval`, `--configuration`, ...)
always wins over both.

Run `nuvem info` to see the resolved config, plugin, log, and cache paths plus
the active theme, without starting the TUI:

```
nuvem 0.1.0
config:  /home/you/.config/nuvem/config.toml (loaded)
plugins: /home/you/.config/nuvem/plugins.toml (2 loaded)
log:     /home/you/.cache/nuvem/nuvem.log
cache:   /home/you/.cache/nuvem
theme:   default
```

`config:` reports `loaded`, `not found`, or `error: <message>` for a file that
exists but failed to parse. `plugins:` reports `not found` for a file that is
absent — the normal case — or how many entries loaded from one that is
present, so a file nuvem never opened is distinguishable from one that
declares nothing. Any field-level warnings (see below) are also printed to
stderr and listed under `nuvem info`.

### Schema (v1)

```toml
version = 1                     # schema version; missing = 1; unknown = rejected

[general]
poll_interval_secs = 5          # refresh cadence in seconds; 0 disables background refresh
default_service = "gce"         # fallback service for a configuration with no remembered view: "gce" | "gke" (restart-required)
sidebar_visible = true          # service navigator sidebar shown by default
theme = "default"               # "default" | "no-color" | "high-contrast"
read_only = false               # refuse cloud-resource mutations (see Read-only mode above);
                                 # OR'd with --read-only, so this can only turn it on

[aliases]                       # extra service aliases: alias -> service id
vms = "gce"
k8s = "gke"

[hotkeys]                       # rebindable normal-mode action -> single key
home = "H"
refresh = "R"

[favorites]                     # scopes to order first in the context picker
projects = ["acme-prod"]        # matched against project id
regions  = ["europe-west1"]     # matched against region name
zones    = ["europe-west1-b"]   # matched against zone name

[theme.overrides]               # frame color per gcloud configuration
prod = "red"                    # configuration name -> border color

[theme.project_overrides]       # frame color per project, whatever reaches it
acme-prod = "red"               # project id -> border color

[columns.compute]               # per-service table columns and default sort
columns = ["name", "status", "zone", "machine-type"]
sort = "status"                 # must be sortable and present in `columns`
sort_direction = "desc"         # "asc" (default) | "desc"; needs a `sort`

[[jumps]]                       # one related-resource jump `J` offers
from = "compute"                # source service
to = "compute:network"          # destination "<service>:<tab>"
name = "{name}"                 # templated target name; default "{name}"
location = "{zone}"             # templated target zone/region; default: the row's own
label = "Network of {name}"     # picker wording; default "<tab> of <name>"
```

### Themes

Three built-in themes cover the semantic roles used throughout the UI
(titles, borders, selection, status colors, key hints):

- `default` — the standard color scheme.
- `no-color` — no foreground/background colors; relies on the `[OK]`/`[ERR]`
  text markers plus bold/underline/reverse-video emphasis. Use this for
  accessibility or terminals without color support.
- `high-contrast` — bright, maximally-distinct colors with bold emphasis, for
  low-vision or high-glare environments.

#### Frame overrides

`[theme.overrides]` and `[theme.project_overrides]` recolor the pane borders
while a dangerous context is active, so it is unmistakable at a glance —
`prod = "red"` in the schema example above is the whole feature.

The two tables differ in what they match, and the difference matters:

- **`[theme.overrides]`** keys on the **gcloud configuration name**, matched
  verbatim against what `gcloud` reports. It is only as reliable as your
  naming convention: if the configuration that reaches production is called
  `default`, `work`, or `acme`, an override written for `prod` never fires.
- **`[theme.project_overrides]`** keys on the **project id**, the same
  vocabulary `[favorites].projects` uses. Every configuration pointing at that
  project gets the frame regardless of what it is called, and a project id is
  the same string for everyone who can reach it, where a configuration name is
  a local, renameable, per-machine alias.

Prefer the project table for the guardrail itself, and the configuration table
for colouring one particular way in. When both match the active context, the
**configuration name wins**: several configurations can point at one project,
so naming one of them is the more specific statement, and the project override
is the catch-all for every configuration you never named.

Each value is a color: a name (`red`, `light-red`), a hex triplet
(`#ff0000`), or a 256-color index (`9`).

Project overrides live in their own table rather than nested inside
`[theme.overrides]`: a gcloud configuration may legitimately be *named*
`projects`, and reserving that key inside a table of configuration names would
leave that configuration uncolourable. Writing `[theme.overrides.projects]`
anyway is reported, with the table that works.

Patterns are deliberately not supported. `*-prod` would reintroduce this
feature's worst failure one level up: a pattern matching nothing is a silent
no-op, and unlike an exact id it cannot be checked against the projects your
configurations actually reach.

The override replaces the *hue* of both border roles — focused and unfocused
alike, so the frame does not fall back to the base accent as you move between
panes. Everything else (titles, tabs, selection, status colors) stays on the
base theme picked by `[general].theme`, and bold/underline emphasis survives,
so an override on top of `no-color` or `high-contrast` does not undo that
theme's non-color cues.

Problems are reported as config warnings rather than being fatal, and each bad
entry is dropped on its own:

- a color nuvem cannot parse, a non-string value, or a blank configuration
  name or project id drops that entry and keeps its siblings.
- an override naming a configuration that does not exist, or a project id no
  configuration targets, is reported once the configuration list loads. Neither
  can be checked when the file is read, because the list comes from `gcloud`
  later in startup — and a silent no-op is the worst failure this feature has,
  since the frame then looks ordinary while you believe it is guarded.

### Hotkeys

Only the normal-mode global/navigation actions below are rebindable, each to a
single key (optionally `ctrl-<char>`):

| action             | default key |
|--------------------|-------------|
| `home`             | `h`         |
| `configurations`   | `c`         |
| `resources`        | `g`         |
| `refresh`          | `r`         |
| `help`             | `?`         |
| `quit`             | `q`         |
| `command_palette`  | `:`         |

`Ctrl-C`, `Ctrl-S`, `Ctrl-T`, `Ctrl-X`, `Esc`, `Enter`, `Tab` (and Shift-Tab),
and `Backspace` are reserved and cannot be used as override targets, for two
opposite reasons. `Esc`, `Enter`, `Tab`, Shift-Tab, `Backspace`, `Ctrl-C` and
`Ctrl-X` are keys an override could never reach: only single-character keys are
looked up in the override table, and `Ctrl-C` and `Ctrl-X` both resolve before
any lookup happens — a binding onto them would simply never fire. `Ctrl-S` and
`Ctrl-T` are the reverse: the override table is resolved *before* the app's own
keys, so a binding onto them wins and silently steals the Logging
save-to-file key or the Tunnels workspace key, with no error and no
way to tell. Validation therefore rejects both groups and says which case
applies.

Two non-rebindable global keys are *not* reserved and so remain shadowable:
`l` (Logging workspace) and `Y` (copy last diagnostic). Both are resolved
after the override table, exactly like `Ctrl-S` and `Ctrl-T`, so binding
`home = "l"` is accepted today and takes the key with the same silence.
Reserving them would break existing configs, so validation leaves them alone —
but avoid them unless you mean to give the key up. `Ctrl-X` is outside that
trade-off entirely: it resolves ahead of the override table, so no config can
reach it. Validation refuses a binding onto it rather than accepting one that
would never fire, and says why — it is the only key that stops a plugin that
never returns.
Rebinding onto a key
already used by the current view's own bindings (for example `v`, the
Resources sidebar toggle) is accepted for the same reason and shadows that
view's key while the rebound action's key is active in that context — avoid
reusing view-specific keys. All hotkey
validation (collisions, reserved keys, and keeping `quit`/`help` reachable)
runs before the TUI starts; footer hints and the help overlay always reflect
the effective (overridden) keys.

### Favorites

`[favorites]` takes three optional keys, each a list of strings: `projects`,
`regions`, and `zones`. A `projects` entry is matched against the project id;
a `regions` or `zones` entry is matched against the region or zone name.

The configuration context picker (opened with `Ctrl-Space` on the Project,
Region, or Zone field while editing the active context) lists favorited rows
first, ahead of the rest, keeping the order they already had in the
underlying list (not the order they were written in `[favorites]`); each
favorited row is marked with a leading `*`. Favoriting only reorders — it
does not exempt a row from the picker's own filter. A favorite that the
applied filter does not match stays hidden along with every other
non-matching row.

Validation drops the smallest broken piece and keeps the rest, the same as
`[aliases]`: a `[favorites]` section that is not a table is ignored in its
entirety with one warning; a key's value that is not an array (for example
`projects = "acme-prod"`) drops that whole key with a warning, leaving the
other keys untouched; and a value inside `projects`, `regions`, or `zones`
that is not a non-empty (once trimmed) string is dropped with a warning
naming the key, while every other value — in that key and in the other two —
is kept.

### Columns and sort

Every resource list draws from a fixed catalog of columns. A
`[columns.<service>]` block picks which of them that service shows, in which
order, and which one the list opens sorted by:

```toml
[columns.compute]
columns = ["name", "status", "zone", "machine-type"]
sort = "status"
sort_direction = "desc"

[columns.storage]
columns = ["name", "class", "location"]
```

Service names come from the same vocabulary as `[aliases]` values and are
matched case-insensitively, so each service answers to several spellings —
among them `compute` (or `gce`, `instances`, `compute_instances`),
`gke` (`clusters`, `kubernetes`, `gke_clusters`), `cloudsql` (`cloud_sql`,
`sql`, `database`, `postgres`, `mysql`), `run` (`cloudrun`, `cloud_run`,
`serverless`), `storage` (`gcs`, `bucket`, `buckets`), `secrets`
(`secret`, `secret manager`, `sm`), and `functions` (`gcf`,
`cloud functions`). The hyphenated forms
(`compute-instances`, `gke-clusters`, `cloud-sql`, `cloud-run`) and the service
labels shown in the sidebar (`GCE instances`, `Cloud SQL`, …) resolve too,
though a label needs quoting as a TOML key. A name that matches no service is
reported and skipped, leaving every real service alone.

The column names are the ids in the table below, and that table is what
`[columns]` accepts. A name is usually the header you already see in the
table, lowercased, but not always — a header abbreviates or renames where a
column key would not, so read the id off this list rather than off the
running table. Columns marked `*` cannot be sorted by — neither from `sort`
nor with `s`:

| service    | columns |
|------------|---------|
| `compute`  | `name`, `status`, `zone`, `machine-type`, `internal-ip`*, `external-ip`* |
| `gke`      | `name`, `status`, `location`, `version`, `nodes`, `endpoint`* |
| `cloudsql` | `name`, `engine`, `region`, `tier`, `availability`, `connectivity`*, `status` |
| `run`      | `name`, `status`, `region`, `url`, `latest-revision`, `traffic`* |
| `storage`  | `name`, `location`, `class`, `updated`, `versioning`*, `public-access`* |
| `secrets`  | `name`, `created`, `replication`, `labels`* |
| `functions` | `name`, `region`, `state`, `generation`, `updated`, `url`* |
| `disks`    | `name`, `location`, `status`, `orphan`*, `size`, `type`, `attached` |
| `pubsub-topics` | `name`, `subscriptions`, `retention`, `kms_key`, `labels`* |
| `pubsub-subscriptions` | `name`, `topic`, `delivery`, `state`, `ack_deadline`, `retention`, `dead_letter`, `labels`* |
| `bigquery-datasets` | `name`, `location`, `labels`* |
| `service-accounts` | `name`, `display_name`, `state`, `unique_id` |
| `iam-policy` | `name`, `role`, `kind`, `condition` |
| `vpc-networks` | `name`, `mode`, `routing`, `subnets` |
| `vpc-subnets` | `name`, `region`, `network`, `range`, `gateway`, `private_google_access` |
| `firewall-rules` | `name`, `network`, `direction`, `priority`, `action`, `targets`, `source_or_dest` |

Three tables are absent from that list because they are views of a service
rather than services of their own, and `[columns]` is keyed by service: the
Cloud Storage object browser, the BigQuery tables drill-down, and the BigQuery
schema view below it. Their columns are fixed and cannot be reordered,
subsetted or sorted by configuration.

Every setting is optional, so a block may set only `columns` and keep the
default sort, only `sort` and keep the default columns, or both; an empty block
changes nothing. Without a `sort` a list opens on `name`, ascending.

`columns` must list at least `name`: a table of `STATUS | ZONE` gives no way to
tell which instance a row is while `Enter`, `t`, `y` and the palette's lifecycle
commands still act on it. A `sort` column must be sortable, and must be one of
the columns listed when `columns` is set. `sort_direction` is `asc` (the
default) or `desc` and only
means something beside a `sort` — on its own it is rejected rather than quietly
ignored. A list may start with a non-sortable column; the `←`/`→` cursor then
opens on the leftmost sortable one instead, so `s` always has something to act
on.

A block is validated as a unit. An unknown or repeated column name, an empty
list, a list without `name`, an unusable `sort`, a misspelled setting key
(`column = [...]`), or a value of the wrong type drops that block whole and puts
that service back on its built-in columns and sort, with a warning naming the
service and the offending value:

```
columns.compute: unknown column "zzz"; ignoring this service's column settings
```

Every other service's block still applies, and no block is applied in part — an
unusable `sort` takes that block's `columns` down with it, because a list that
sorts by a column it does not show is a worse table than the default one. Column
warnings surface like any other config warning: in the startup and reload
status lines, on stderr and under `nuvem info`, and in the help overlay's
`Config problems` list.

Reload (`:` → **Reload configuration**) re-reads the blocks live, per service. A
service whose new block fails validation keeps the settings it is already
running — the last good table, not the built-in defaults — and its warning says
so, ending `ignoring this block, keeping the column settings already in use`
instead of claiming the fallback above (a service with nothing running yet
really does fall back, and keeps the fresh-load wording). Deleting a block
restores that service's defaults. A reload never refetches: the loaded rows, the
row selection, the applied filters, and the load state all survive it. Only the
layout itself moves — and with it the `←`/`→` cursor, and the sort under the
rules below.

`sort` is a *default*: it decides what a list opens on, and pressing `s`
overrides it for the session. A reload only re-installs a service's configured
sort when that service's own block changed, so a sort you set by hand survives a
reload triggered by something else (another service's block, a `[general]`
field); editing that service's block puts its configured sort back. If a reload
installs a list that no longer shows the sorted column, the sort falls back to
ascending `name` so the arrow never points off the table.

`[columns.storage]` configures the **bucket list**. The object browser behind
`Enter` is a detail mode with no column navigation and no sorting, so there is
nothing to configure there and it ignores the block.

`w` is unaffected and stays a *width preset*, not a column selection: the config
answers which columns exist for you, `w` answers how many of them fit right now.
Narrow (the default) drops columns until the row fits and reports the hidden
count in the tools line as `+N cols (w)`; wide keeps every column in the list
and marks the row `▸ more` when it overruns the terminal. Dropping the toggle
would either force every narrow-terminal user to hand-configure a short list or
silently change the default table for everyone who never opens the config file.

The two do interact in one place. For a service whose `columns` list is
configured, narrow mode drops the user's rightmost column first and works
leftwards — with one exception: `name` is never dropped, wherever it sits in
the list. A list that ends in `name` loses its *second*-to-last column first,
and `name` is the column still standing on the narrowest terminal. No column
list can leave you looking at rows you cannot tell apart while `Enter`, `t`,
`y` and the palette's lifecycle commands still act on the selected one — the
same reason `columns` must list `name` in the first place. Without a configured
list — including a block that sets only `sort` — the built-in per-column
priorities decide instead, which is what keeps an unconfigured table rendering
exactly as it did before.

### Jump rules

`J` opens the related-resource jump picker (see
[Related-resource jumps](features.md#related-resource-jumps)). Which jumps it offers is a
list of `[[jumps]]` rules — one array entry per row the picker can show:

```toml
[[jumps]]
from = "compute"
to = "compute:network"
name = "{name}"
location = "{zone}"
label = "Network of {name}"
```

| key        | required | meaning |
|------------|----------|---------|
| `from`     | yes      | the service the jump is offered from |
| `to`       | yes      | where it lands: a detail tab as `"<service>:<tab>"`, or a service list as `"<service>"` |
| `name`     | no       | template naming the resource to open; `"{name}"` by default |
| `location` | no       | template for that resource's zone/region/location; the source row's own by default, and not accepted at all when the destination identifies its rows by name alone |
| `label`    | no       | the picker row's wording; `"<tab> of <name>"` for a detail tab and `"<resource> <name>"` for a list, by default |
| `select`   | no       | narrows the destination list to a computed set instead of landing on one row |

`from` takes the same service vocabulary as `[columns]` and `[aliases]`
(`compute`/`gce`, `gke`, `cloudsql`, `run`/`cloudrun`, `storage`, …). A `to`
with a `:` in it is a service and one of that service's detail tabs, and its
service half takes that same vocabulary — `to = "gce:disks"` and
`to = "compute:disks"` name the same tab. The tab half is exact: lowercased with
spaces hyphenated, the tab names the `[`/`]` tab bar shows. The table below
spells each service the canonical way:

| service    | destination tabs |
|------------|------------------|
| `compute`  | `compute:summary`, `compute:network`, `compute:disks`, `compute:metadata`, `compute:json` |
| `gke`      | `gke:summary`, `gke:networking`, `gke:node-pools`, `gke:addons`, `gke:json` |
| `cloudsql` | `cloudsql:summary`, `cloudsql:connectivity`, `cloudsql:maintenance`, `cloudsql:backups`, `cloudsql:replicas`, `cloudsql:users`, `cloudsql:json` |
| `cloudrun` | `cloudrun:summary`, `cloudrun:traffic`, `cloudrun:revisions`, `cloudrun:configuration`, `cloudrun:conditions`, `cloudrun:json` |

A `to` with no `:` in it names a **service list** rather than a detail tab: the
jump switches to that service's table and puts the cursor on the row `name`
resolves to. Fifteen of the sixteen services are legal list destinations,
including the twelve that have no detail tabs at all, so `to = "vpc-networks"`
and `to = "secrets"` are rules you can write:

```toml
[[jumps]]
from = "firewall-rules"
to = "vpc-networks"
name = "{network}"
```

Cloud Storage has no detail tabs — its `Enter` opens an object browser, not a
tabbed pane — so `to = "storage:objects"` names nothing and a rule that writes
it says so rather than reporting an unknown tab; `to = "storage"` is a legal
list destination. Secret Manager, Cloud Functions and Pub/Sub topics are the
same shape: no `DetailTab` spelling, so no `"<service>:<tab>"` destination, but
a list destination like every other service. Cloud Functions has no detail pane
at all — by design, for the reason recorded with the function list under
[Home overview](features.md#home-overview).

The IAM policy table is the one service that is neither: a binding is
identified by its member *and* its role, since two bindings can share a member,
and no `name`/`location` pair renders a role. `to = "iam-policy"` is refused at
load with that reason rather than offered as a rule that lands nowhere. It stays
a legal `from`.

A list jump lands on a row, not on a filtered table. `to = "firewall-rules"`
with a `{network}` name does **not** show the rules on that network; it puts
the cursor on the rule *called* that.

`select` is how a rule asks for the other thing: a list that arrives **narrowed**
to a computed set. There is one computation, and it is the built-in
instance-to-firewall jump:

```toml
[[jumps]]
from = "compute"
to = "firewall-rules"
select = "targets-this-instance"
```

It shows the firewall rules that target the instance — the ones on its network
whose target tags or target service accounts it carries, **plus every rule on
that network that names no target at all**, since those apply to every instance
in it. A narrowing rule ignores `name` and `location`: the set is computed from
the instance rather than named by a template, so there is no single row for one
to point at, and an empty result is an answer (no rule governs this instance)
rather than a row that has gone.

The narrowed table intersects with `/`, so you can still filter inside it, and
the header says which instance it is narrowed to. What it does **not** narrow by
is whether a rule is in force: a DENY rule and a disabled rule both still target
the instance and both stay in the list. Hiding them would defeat the point of
opening it. `select` is only defined from `compute` to `firewall-rules`; any
other pairing is rejected at load.

A rule to a list whose rows are identified by name alone — a VPC network, a
secret, a topic — must not carry a `location`, and one that does is rejected at
load with that reason: the row identity has no location half for it to match.

`secrets`, `pubsub-topics`, `pubsub-subscriptions`, `service-accounts`,
`iam-policy`, `vpc-networks` and `firewall-rules` are the services with no
location field at all.
A secret, a topic, a subscription, a service account, the project policy, a
VPC network and a firewall rule are all global, so
`{location}` would have nothing to render, and
a `location` template on one of those rules is rejected with that reason rather
than silently producing an empty string.

`name`, `location` and `label` are templates: literal text with `{field}`
placeholders filled from the row you press `J` on. Which fields exist depends on
the **source** service, so a template is checked against `from`:

| `from`     | fields |
|------------|--------|
| `compute`  | `{name}`, `{zone}`, `{machine_type}`, `{status}`, `{internal_ip}`, `{external_ip}`, `{network}`, `{subnetwork}`, `{subnetwork_region}` |
| `gke`      | `{name}`, `{location}` |
| `cloudsql` | `{name}`, `{region}`, `{engine}`, `{tier}`, `{status}` |
| `cloudrun` | `{name}`, `{region}` |
| `storage`  | `{name}`, `{location}` |
| `secrets`  | `{name}` |
| `functions` | `{name}`, `{region}` |
| `disks`    | `{name}`, `{location}` |
| `pubsub-topics` | `{name}` |
| `pubsub-subscriptions` | `{name}` |
| `bigquery-datasets` | `{name}`, `{location}` |
| `service-accounts` | `{name}` |
| `iam-policy` | `{name}` |
| `vpc-networks` | `{name}` |
| `vpc-subnets` | `{name}`, `{location}`, `{network}` |
| `firewall-rules` | `{name}`, `{network}` |

`{network}`, `{subnetwork}` and `{subnetwork_region}` on `compute` are the only
fields that do not come from the row. A GCE instance's list row carries no network at all; both
values live on the detail pane, so a rule naming either resolves once that pane
is open and fetched, and offers nothing from the list. That is not a failure and
it is not the instance's fault, so `J` says which it is: **needs network, which
is on this GCE instance's detail — open it and try again**, as against the
ordinary **has no value for** an instance whose fetched pane really has no
network interface. An instance with several interfaces reports its first, the
same one `{internal_ip}` and `{external_ip}` already come from.

`{subnetwork_region}` exists because `{subnetwork}` does not identify a subnet
on its own: an auto-mode network calls every one of its subnets `default`, one
per region, so a rule keyed on the name alone would name as many rows as the
project has regions. The region is read out of the subnetwork's self-link, from
the segment following `regions` — **not** by stripping the last part off the
instance's `{zone}`. That guess produces the right answer on most topologies,
which is exactly what makes it dangerous; nothing in the API guarantees an
instance's subnet is in the region its zone belongs to. A link that names no
region leaves the field absent, so a jump keyed on it offers nothing rather
than landing on the wrong subnet.

There is no `{network_region}` beside it. A network's self-link is
`.../global/networks/<name>`: a network is global within its project, so the
name is the whole identity and nothing is discarded when it is shortened.

Substitution is a single pass: a value that itself contains `{` or `}` is
written out as it stands and never expanded again. There is no escape for a
literal brace.

A worked example — the built-in Network row is kept, the Disks row is reworded,
and every GCE instance also offers a row for the GKE cluster named after it
(which lands only under the conditions at the end of this section):

```toml
[[jumps]]
from = "compute"
to = "compute:network"

[[jumps]]
from = "compute"
to = "compute:disks"
label = "Disks attached to {name}"

[[jumps]]
from = "compute"
to = "gke:node-pools"
name = "{name}-cluster"
location = "{zone}"
label = "Node pools of the {name} cluster"
```

A `[[jumps]]` array **replaces the built-in set** rather than merging with it,
which is why the example repeats the Network rule: merging would leave no way to
remove a built-in jump. Without any `[[jumps]]` entry you get the built-in rules
(the table under [Related-resource jumps](features.md#related-resource-jumps)); with
`jumps = []` no jump is configured at all, and `J` then only reports that none
are configured for the service you are in.

Each rule is validated on its own. A rule with an unknown `from` or `to`, an
unknown key, a value of the wrong type, an empty or unbalanced template, or a
`{field}` its `from` service does not have is dropped with a warning naming its
index, and the rules beside it keep working:

```
jumps[2]: unknown destination tab "compute:netwrok"; ignoring this rule
```

Jump warnings surface like any other config warning: the first one in the
startup status line, all of them on stderr and under `nuvem info`. Reload (`:` →
**Reload configuration**) re-reads the whole array live.

A rule that is valid can still have nothing to offer for a particular row — no
config check can know that *this* instance has no external IP. When its `name`
or `location` renders empty for the selected row, that rule is
**not offered for that row**; the other rules still are, and when nothing at all
resolves the picker stays closed and the status line names the rule and the
field:

```
Jump "compute:network" needs external_ip, which this GCE instance has no value for
```

A `label` is wording rather than identity, so it is treated differently: one
that will not render for the selected row
**falls back to the built-in wording** (`"<tab> of <name>"`) and the jump is
still offered. A caption is not worth losing a jump that would land correctly.

The rendered `name` and `location` must also look like GCP resource names
(non-empty, at most 253 characters, `A-Z a-z 0-9 . _ -`), because they end up as
`gcloud describe` arguments. A rule whose rendered name fails that check is not
offered either, and says so:

```
Jump "compute:disks" renders an invalid name for this GCE instance
```

Keep `{status}` to a `label`. A status is not a resource name, so a rule that
selects on one names nothing that exists; and the statuses that are worded with
a space (`Unknown (...)`, Cloud SQL's `Pending create`) fail the check outright,
so such a rule is offered for some rows and refused for others.

Finally, a jump that *is* offered lands only on a resource
**already in the destination service's loaded list**, matched on both its
name and its zone/region — the picker never fetches a resource into existence.
When there is no such row the status line says which case it is:

```
Loading GKE clusters, so web-01-cluster cannot be opened yet — try again once they arrive
No GKE cluster loaded yet, so web-01-cluster cannot be opened
web-01-cluster is no longer available
```

The first is a destination service this session has never loaded, so the jump
starts its list load on the way past; repeat the jump once the rows are in and
it lands. The second is a list that is not loaded and is not being fetched —
a load that failed *in this configuration*, waiting on the retry backoff. A
failure inherited from a configuration you have since left does not count: that
backend has not been called here, so the jump fetches. The third is a list that
has been loaded and does not have that row in it. So the cross-service
rule above lands only once the GKE list has been fetched and holds a cluster
called `web-01-cluster` whose location is exactly the instance's zone — a zonal
cluster in `europe-west1-b`, not a regional one in `europe-west1`, since
`location = "{zone}"` hands over the zone verbatim. Cross-service rules are
worth writing where your names and locations line up across services; where they
do not, the picker row still appears and the jump reports instead of landing.

### Remembered last view

nuvem opens on the resources service you last had open in each gcloud
configuration — switch to Cloud SQL while `dev` is active, come back to `dev`
later (even in a new session, even after a crash), and it opens on Cloud SQL
again. Only the active service is remembered: sort order, filter text, the
open detail tab, and the selected row are not, and reset to their own defaults
each time.

This changes what `general.default_service` means. It is no longer an
override applied on every launch; it is the **fallback for a configuration
that has no memory yet**. Once a configuration has a remembered service,
`default_service` no longer has any effect on it — it still decides every
configuration you have not switched services in, and still decides for
everyone until they do.

The memory is written to `state.toml`, a file the app creates beside whichever
`config.toml` is in use (including one named by `--config`). It is deliberately
a separate file from the one you hand-edit: `config.toml` keeps your comments
and formatting exactly as you wrote them, and an automatic rewrite of a second
file can never disturb that. You are not expected to open or edit `state.toml`
yourself. A service name in it that this build of nuvem does not recognise
(written by a newer version) is ignored for that configuration — it falls
back to `default_service` — and is kept rather than dropped the next time this
build saves.

The remembered service is recorded the moment you switch, not on a clean exit,
so it survives a crash or a killed terminal, not just a quit. `--read-only`
does not suppress this: that flag promises no cloud-resource mutations, and
nuvem already makes other local-only changes regardless of it (activating a
gcloud configuration, opening a tunnel, writing the clipboard).

### Reload on command

Open the command palette (`:`) and run **Reload configuration** (aliases
`reload config`, `config reload`) to re-read the config file without
restarting. It applies the safe fields live: theme, aliases, sidebar
visibility, poll interval, hotkeys, each service's columns and sort (see
*Columns and sort* above), and the jump rules (see *Jump rules* above).
`default_service`, the starting
configuration and the plugin file are restart-required — reload reports them in
the status line instead of applying them. A reload that fails to parse leaves the running
config untouched and shows the error in the status line; cloud state is never
touched by a reload.

### Fallback rules

A bad config file never crashes nuvem — problems degrade to a warning and a
default value:

- unparseable file, or an unreadable path → all fields fall back to defaults,
  with the failure reported as a warning;
- unknown/unsupported `version` → all fields fall back to defaults, with a
  warning;
- an invalid enum value (`general.theme`, `general.default_service`) → that
  field falls back to its default, with a warning; other fields still load
  normally;
- an invalid `[hotkeys]` set (unknown action, bad key spec, a reserved key, a
  collision, or an override that makes `quit`/`help` unreachable) → the
  *entire* hotkey set falls back to the built-in defaults, with a warning;
- an invalid single `[aliases]` entry → only that alias is dropped, with a
  warning; the rest of the table is kept;
- an invalid single `[favorites]` entry → only that entry is dropped, with a
  warning; its valid siblings survive, the same as `[aliases]`. A `projects`,
  `regions`, or `zones` key whose value is not an array drops that whole key
  instead, with a warning, leaving the other keys untouched; a `[favorites]`
  section that is not a table at all is dropped in its entirety, with one
  warning;
- any problem inside one `[columns.<service>]` block → that service's *entire*
  block is dropped and the service falls back to its built-in columns and sort,
  with a warning; every other service's block is still applied. A block naming
  a service that does not exist is skipped without affecting any service;
- two or more `[columns.*]` keys naming the same service (`[columns.gce]` and
  `[columns.compute]` are both Compute Engine) → the alphabetically last key
  decides on its own, and the service reports one warning naming every key it
  displaced as well as the winner; a displaced key that is itself invalid has
  its own problem named in that same warning, so deleting the winning block is
  not how you first find out about the typo under it. If the deciding key's
  block is invalid, every block naming that service is discarded and the
  service falls back to its built-in columns and sort, with that one warning
  naming the problem and the blocks discarded with it. Blocks are read in key
  order, not file order, so the one you wrote second does not necessarily win —
  which is why the loser is reported instead of silently changing nothing;
- an invalid single `[theme.overrides]` entry → only that entry is dropped,
  with a warning; its valid siblings survive. A `[theme]` that is not a table,
  an unknown key inside it (the base theme is `[general].theme`), or an
  `overrides` value that is not a table drops that much and nothing more — see
  [Frame overrides](#frame-overrides);
- any problem inside one `[[jumps]]` entry → only that rule is dropped, with a
  warning naming its index; the other rules still apply. A `[jumps]` written
  with single brackets is not an array of rules, so the whole section is
  dropped with one warning and no jump is configured — the rest of the file
  still loads.

The separate plugin file degrades the same way, and independently — it is a
separate load, so nothing below costs you your `config.toml`:

- the plugin file is absent → no plugins, no warning; this is the normal case;
- the plugin file is unreadable or unparseable → no plugins, with one warning
  naming the path and line; your `config.toml` is unaffected, because the two
  are separate loads;
- an unknown plugin-file `version` → no plugins, with a warning; entries are
  rejected whole rather than read under rules that may not be theirs;
- a `[plugins]` written with single brackets is not an array of plugins, so the
  whole section is dropped with one warning;
- any problem inside one `[[plugins]]` entry → only that plugin is dropped,
  with a warning naming its index; the other plugins still load. Two plugins in
  the same scope sharing an `id` or a `key` is such a problem: the later one is
  dropped and the warning names the plugin it clashed with. The same key in two
  *different* scopes is fine — the two are never offered at once. A problem
  inside one of a plugin's `[[plugins.inputs]]` tables is such a problem too:
  an unknown `type`, a `dropdown` with no `choices`, a `default` that
  contradicts its own `type` or sits outside its own `choices`, a `name` that is
  already a field of the plugin's scope, or an unknown key inside the input
  table. The warning names the input by index and by name, that one plugin is
  dropped, and the rest still load.

Startup warnings are shown on stderr (and via `nuvem info`) — except the
deferred `[theme.overrides]` check, which can only run once the configuration
list has loaded and so joins the list in the running app rather than on
stderr. Every one of them is listed in the help overlay (`?`) under
`Config problems`, so a
misconfigured file is always visible in full and not just by its first
complaint. The status line summarizes: a lone problem is quoted whole
(`config warning: …`), and several are counted with the key that opens the list
(`config: 3 problems (press ? for details)` — it names your `help` key, rebound
or not). A reload says the same thing in its own wording and rewrites the list,
so fixing the file empties the section rather than leaving stale advice in it.

Every normal view uses the shared application frame: a two-row title/context
header, breadcrumb, workspace body, operation status with last-refresh time,
and mode-aware footer hints. The header prioritizes the active configuration
and project; account, service, region, and zone are shown when the terminal has
room. Status markers include `[OK]`, `[LOAD]`, `[SYNC]`, `[STALE]`, and `[ERR]`
so loading and degraded states remain understandable without relying on color.
The normal frame requires at least `60x12`; smaller terminals show a resize
message, and the help overlay requires `73x22`.

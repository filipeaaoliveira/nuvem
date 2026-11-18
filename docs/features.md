# Features

What each workspace shows and how it behaves. Keys are listed in
[keys.md](keys.md); configuration in [configuration.md](configuration.md).

## Home overview

The Home workspace (`h`) is a read-only dashboard over already-loaded state;
it never triggers a resource load on its own. It shows:

- **Context health**: whether `gcloud` is available and authenticated, the
  active account, and any classified problem from the last diagnostics probe.
  The probe covers the `gcloud` CLI only; a native read that fails on its own
  (an expired token, a disabled API, no route to `googleapis.com`) surfaces on
  the service row rather than here.
- **Inventory by status**: one line per service (GCE instances, GKE clusters,
  Cloud SQL, Cloud Run, Cloud Storage, Secret Manager, Cloud Functions,
  Persistent disks, Pub/Sub topics, Pub/Sub subscriptions, BigQuery datasets,
  Service accounts, IAM policy, VPC networks, VPC subnets, Firewall rules) — every service nuvem can browse — with status counts for the services that have a
  lifecycle status and a freshness marker distinguishing not-loaded (never
  fetched yet), healthy-empty (fetched, zero rows), loaded (fetched, has rows),
  and stale (fetched but out of date for the active context). A short
  terminal that cannot show every line ends the block with `+N more` rather than
  leaving the services it skipped unmentioned.
- **Recent operations**: the most recent in-app operations (currently GCE and
  Cloud SQL mutations, and plugin runs) with their outcome (pending, succeeded,
  failed, cancelled, unknown) and detail. `cancelled` means the command was
  never issued; `unknown` means it was issued but its result was never seen,
  so the resource's real state has to be re-checked with a refresh.
- **Degraded causes**: when context health or a service line reports a
  problem, it shows a sanitized, actionable headline — a classified reason
  plus a short remediation hint (for example `permission denied — Check IAM
  roles for this account and project`) — instead of raw gcloud output. Press
  `Y` in any normal view to copy the sanitized technical detail of the last
  diagnostic to the clipboard.

Press `r` on Home to refresh the overview diagnostics (the gcloud
availability/auth probe behind the context-health card). It does not reload
resource inventory; switch to the Resources workspace and press `r` there to
refresh a service's rows.

## Resources and services

The Resources workspace opens with `g`. It provides a searchable service
navigator sidebar: press `v` to show or hide the sidebar, `/` to search the
service registry when the sidebar is focused, `j`/`k` or the arrows to move
through matching services, and `Enter` to open the selected service directly.
Press `Tab` to move focus between the sidebar and the resource content,
including while a detail view is open. Selecting a live service keeps the
existing typed resource state and effects; unavailable services remain visible
with their availability status.

In the Resources list, use the left/right arrows to move the selected column,
`s` to sort by it (press `s` again to reverse direction), and `w` to toggle
wide columns. Column headers show the active sort direction with an arrow.
Which columns a service shows, in which order, and which one it opens sorted by
are configurable per service — see *Columns and sort* under
[Configuration](configuration.md#configuration).

Reads are native. Every list and detail load below is a Google Cloud
client-library call, authenticated by `gcloud config config-helper` for the
active configuration; the `gcloud` command shown with each service is the
equivalent invocation, and is what demo mode, the fallback path and a
label-mode (`-l`) read actually run. BigQuery is the exception: it always
shells out to `bq`. Mutations always run the `gcloud` command shown.

For GCE instances, pressing `r` runs:

```bash
gcloud --configuration <active-config> compute instances list --format=json --limit=500
```

For GKE clusters, pressing `r` runs:

```bash
gcloud --configuration <active-config> container clusters list --format=json --limit=500
```

In the GKE clusters service, select a cluster and press `Enter` to load a
read-only detail view. The detail command is:

```bash
gcloud --configuration <active-config> container clusters describe <name> --location <location> --format=json
```

Use `[` and `]` to move across Summary, Networking, Node Pools, Addons, and
JSON tabs. GKE mutation is not included: nuvem does not create, scale or
delete clusters.

For Cloud Storage, pressing `r` runs:

```bash
gcloud --configuration <active-config> storage buckets list --format=json
```

The bucket list shows name, location, storage class, last-updated time,
versioning, and public access prevention where available. Press `Enter` on a
bucket to open a hierarchical, prefix-aware
object browser with breadcrumb navigation instead of the flat list used by
other services. Object rows show name, size, storage class, updated time,
content type, and encryption/retention indicators (never key material). Press
`n` to load the next bounded page of objects (100 objects per `gcloud`
request) while preserving the current selection; there is no unbounded load,
so very large buckets are browsed page by page. Press `-` or `Backspace` to
move up one prefix level, eventually returning to the bucket list from the
top-level prefix.

Press `/` in the object browser to filter the already-loaded rows; when the
filter text is also a safe prefix (no glob characters), it is additionally
sent to `gcloud` as a server-side prefix filter, and the filter tools line
labels the applied filter `(local)` or `(remote)` so it is clear whether the
current rows reflect a client-side or backend-narrowed listing. Press `i` on a
bucket, or `Enter` on an object, to view read-only metadata across Summary,
Metadata, and JSON tabs. Press `y` in the bucket list or object browser to
copy the selected item's `gs://` URI to the clipboard; inside a bucket/object
detail view `y` follows the detail-pane rules in the key list below, so it
copies the selected field, or the raw document on the JSON tab, and falls back
to the `gs://` URI only when neither applies. Copied values are stripped of
terminal escape sequences and control characters (newlines and tabs survive),
so a hostile bucket or object name cannot smuggle them onto the clipboard.
Distinct states are shown for an empty bucket, a filter with no matches,
permission denied, a bucket requiring requester-pays
billing, a bucket-level retention policy, and a partial page failure (rows
already loaded are kept if a later page fails to load).

Cloud Storage browsing is read-only: uploading, downloading, deleting objects,
ACL/IAM management, and signed URLs are out of scope.

Note on pagination: the object browser pages with the token the service itself
issues, so continuing a listing resumes exactly where the previous page ended.
Demo mode and the fallback path use `gcloud storage objects list`'s own
`--page-token`/`--page-size` flags instead. The gcloud
CLI does not expose a raw, independently resumable cursor beyond what
`--format=json` returns for a given invocation, so continuing a listing
precisely is best-effort against real bucket state. Pagination itself (page
boundaries, selection preservation, and the local/remote filter interaction)
is modeled at the domain/reducer layer and is fixture-tested rather than
verified against live changing buckets.

For Secret Manager, pressing `r` runs:

```bash
gcloud --configuration <active-config> secrets list --format=json --limit=500
```

For Cloud Functions, pressing `r` runs:

```bash
gcloud --configuration <active-config> functions list --regions=- --v2 \
  --format=json --limit=500
```

`--regions=-` asks for the whole project rather than the region a
configuration's `functions/region` property happens to name, and `--v2` makes
both generations come back in one shape instead of 1st gen rows arriving in the
older format. The list is read-only and shows name, region, state,
generation, update time and invoke URL. Name and region lead because a
function's name is unique within its region, not within the project, so both
are needed to tell two rows apart. STATE is colour-coded like every other
lifecycle column: a function that is deploying or being deleted reads as busy
rather than as broken, and a state nuvem does not recognise is left uncoloured
rather than guessed at.

There is no detail pane: `Enter` does nothing on a function. `functions
describe` returns the same resource `functions list` already returned for every
row, so a pane would be a second gcloud call showing what is on screen. The
value with the least room in the table is the invoke URL, the first column
dropped on a narrow terminal — the preview beside the navigator gives it a line
of its own, along with the rest of the row, so it survives at widths where the
column is elided or gone.

The secret list shows name, create time, replication policy, and labels. There
is no version-count column: the listed secret resource does not carry one, so
filling a column would cost one extra call per row on every list load, and one
list load is one `gcloud` call on every service here. The count lives on the
detail pane instead, fetched for the one secret you open.

Select a secret and press `Enter` to load a read-only detail view across
Summary, Versions, Labels, and JSON tabs (`[` and `]` to move between them).
Opening it runs two commands:

```bash
gcloud --configuration <active-config> secrets describe <name> --format=json
gcloud --configuration <active-config> secrets versions list <name> --format=json
```

Versions list version name, state (enabled, disabled, destroyed), and create
time. Listing versions needs an IAM permission that listing secrets does not,
so a failure there is reported as such rather than shown as a secret with no
versions.

Secret Manager browsing is metadata only, and that is a structural property
rather than a display rule: nuvem never issues the call that returns a
version's payload, so there is no secret material anywhere in the app to
redact. Creating, editing, disabling, or destroying secrets and versions is out
of scope too — the service is read-only.

### BigQuery

For BigQuery, pressing `r` runs:

```bash
CLOUDSDK_ACTIVE_CONFIG_NAME=<active-config> \
  bq --format=prettyjson --project_id=<project> ls -d -n 500
```

`bq` has no `--configuration` flag, so the active configuration is carried in
the child environment instead of on the argv; `--project_id` is omitted
entirely when the configuration names no project, which lets `bq` fall back to
the configuration's own. The table columns are DATASET, LOCATION and LABELS.

Press `Enter` on a dataset to drill into the tables it holds:

```bash
CLOUDSDK_ACTIVE_CONFIG_NAME=<active-config> \
  bq --format=prettyjson --project_id=<project> ls -n 500 <dataset>
```

Once that listing lands, each row's stats are read by its own call:

```bash
CLOUDSDK_ACTIVE_CONFIG_NAME=<active-config> \
  bq --format=prettyjson --project_id=<project> show <project>:<dataset>.<table>
```

No `-n` on that one: `show` answers about exactly one table, so there is no
listing to cap. The table is named fully qualified, because an unqualified
`<dataset>.<table>` would be resolved against the configuration's own project
rather than the project the listing came from.

The drill goes one level further: `Enter` on a table row opens that table's
column schema, described below. `Esc` ascends one level per press — the schema
view back to the tables list, the tables list back to the datasets — and `-`
and `Backspace` do the same thing `Esc` does, for the operator carrying the
Cloud Storage object browser's habit. Leaving a level drops what it had
listed; re-entering asks for it again.

The tables view has six columns. TABLE, TYPE and CREATED are everything in
`bq ls <dataset>`'s answer that earns a column of its own; the listing also
reports each row's labels and its dataset and project ids, and what becomes of
those is below. ROWS, BYTES and LAST MODIFIED come from elsewhere: the listing
does not carry them at all, so each row's three numbers are read by that row's
own `bq show`, issued once the listing lands. They arrive after the rows do,
and a row shows dashes until its own call answers. TYPE tells a table from a
view, a materialized view or an external table, and a type BigQuery adds later
is shown verbatim rather than reported as a plain table. The rows are painted
in the order `bq ls` returned them: this list has no sort and no column
selection of its own, and the tools line offers neither. (`s`, `←` and `→`
still act on the dataset list behind the drill, where nothing on screen moves.)

Opening a dataset therefore issues one `bq show` per listed table, and a
dataset holding 200 tables is 200 calls — rows a filter is hiding included.
Nothing bounds that beyond the listing's own `-n 500` cap, which caps the rows
and so caps the calls; four of them run at a time and the rest queue. Each call
is held to a two minute ceiling: past it the `bq` process is killed, its whole
process group with it, and the call is reported as refused rather than waited
on. That is a ceiling on a hang and not a latency budget — a `bq show` answers
in seconds — and it exists because one call that never comes back holds its
permit for good and stalls every row queued behind it. Nothing is
cached and a refused call is never retried, so every `r`, and every re-entry
into the dataset, re-issues the whole fan-out — and once the retry lands it
drops the numbers it already had rather than leaving them on screen as the
answer to a refresh. Starting a fan-out cancels whatever the last one still had
queued, so those calls never go out; the four already in flight run to
completion, and their answers are discarded when they arrive.

Only starting another listing cancels a fan-out. Leaving the dataset with
`Esc`, entering one of its tables, switching service, or switching context does
not: the calls already
queued keep running, four at a time, and every answer is thrown away on
arrival. On a wide dataset that is a few hundred `bq` processes still to spawn
for a table nobody is looking at any more. It costs the machine and the API
quota rather than the screen, and the next dataset entered ends it.

A dash in ROWS, BYTES or LAST MODIFIED means nobody answered: that row's `bq
show` is still out, or it was refused. It never means zero. A table that really
holds nothing paints `0` and `0 B`. The listing and the per-row `show` are
separately authorised — `bigquery.tables.list` lists the tables,
`bigquery.tables.get` reads each one — so a listing whose rows all show three
dashes is the ordinary shape of the second permission being denied, and it
leaves TABLE, TYPE and CREATED intact.

A dash means one more thing than those two, and this is the case to know
about: pressing `Enter` on a table before the fan-out has finished orphans the
rest of it. Opening a table, and then coming back with `Esc`, leaves the tables
list exactly as it was — the rows are not re-listed — but the `bq show` answers
still in flight were minted for the drill you left, so every one of them is
dropped on arrival. The rows the fan-out had already reached keep their
numbers; the rows it had not show `-` in ROWS, BYTES and LAST MODIFIED
indefinitely, on a list that looks settled rather than loading. Pressing `r` is
what fills them: it re-issues the listing and the whole fan-out with it. The
orphaned calls are not cancelled either — entering a table cancels nothing, so
they keep running four at a time, spending API quota on answers that are
discarded.

Labels have no column here, unlike the dataset table — one real dataset's
`dataplex-*` set flattens past 300 characters and would bid against the table
name for the width of the pane. They are still searchable: `/` narrows the
loaded rows on the table id, its dataset, its type as the TYPE cell spells it
(`/materialized` matches `Materialized view`) and its `key=value` labels. The
three enrichment columns are the deliberate exception: `/` does not match what
ROWS, BYTES and LAST MODIFIED paint. Those cells are formatted numbers rather
than words, and matching them would also match the dash — so `/-` would select
exactly the rows nothing is known about. `bq ls` takes no filter expression, in
either view, so a filter is applied client-side and issues no call. The preview
beside the navigator describes the table the cursor is on — its type, its
creation time, its dataset and its labels — rather than the dataset the drill
is inside, which the pane title already names.

Pressing `r` inside the tables drill re-issues the listing for the dataset you
are in, which is also what the failure state's "Press `r` to retry." refers to;
the dataset list behind the drill is refreshed in the same step. An empty
dataset is reported as such rather than as a failure.

Press `Enter` on a table to open its column schema, one level further down:

```bash
CLOUDSDK_ACTIVE_CONFIG_NAME=<active-config> \
  bq --format=prettyjson --project_id=<project> show <project>:<dataset>.<table>
```

That is the same command the per-row stats use, issued a second time for the
one table you opened. Nothing caches it, in either direction: the fan-out
throws its schemas away rather than retaining every listed row's schema for a
view nobody may open, and this call throws away the three numbers it carries
because the row behind it already has them. So one `Enter` is one `bq show`,
and `r` inside the schema view re-issues it for the table on screen rather than
re-listing the tables behind it. Re-entering the same table asks again.

The schema view has four columns: FIELD, TYPE, MODE and DESCRIPTION. Nested
records are flattened to dotted paths — `user.address.city` is a row — and the
record itself is kept as a row of its own above its children rather than
expanded in place or collapsed away, so a `RECORD` row is the parent of the
rows that follow it. The rows carry no indentation of their own — the dotted
path is what says where a field sits. The rows are painted in
schema order, which is what keeps a record's children together; this list has
no sort and no column selection, and its columns cannot be reordered or
subsetted by configuration. DESCRIPTION paints `-` for a field the schema
documents with nothing, which is not the same as a field documented with an
empty string.

`/` narrows the loaded columns on the path, the type, the mode, and the
descriptions that exist. The path is matched whole, so `/event_params` selects
a record together with everything nested inside it. The dash is the one thing
on screen `/` does not match, for the reason the tables level does not match
its own dashes: it is not a word the row holds, and matching it would make
`/-` select exactly the fields nobody documented. `bq show` takes no filter
expression either, so this narrowing is client-side and issues no call. The
preview beside the navigator describes the column the cursor is on, and carries
its description unelided where the column is too narrow for it.

A refused `bq show` fails this level rather than dashing a cell: the call is
the whole content of the view, so the view keeps the table in its header, says
what went wrong, and offers `r`. A table whose schema is empty is reported as
empty rather than as a failure — the same split the tables listing makes.

`Enter` on a column row opens nothing: a column is the bottom of this
hierarchy. Neither surface offers it here — the frame footer hints `Enter` only
where it opens something, and the help overlay (`?`) drops it at the bottom
level. Both read the same question: whether the drill on screen has another
level below it.

BigQuery browsing is read-only throughout: no query is ever run, and nothing is
created, modified or deleted.

In `--demo` mode the same recorded tables answer every dataset, and the same
recorded table resource answers every `show` — so both demo rows paint one
table's row count and byte size, and both open that same table's columns. Demo
mode holds one fixture per command rather than one per argument, and the two
callers of `show` send an identical command line, so one recorded response has
to answer both. The one it serves is a nested schema, which is what makes the
demo schema view show the dotted paths, the `REPEATED` mode and the `-` a real
one does; the numbers it paints are that recorded response's rather than the
listed row's, and that response was transcribed rather than freshly captured —
its counters carry the shape the API sends and not verified values.

### Project IAM policy

For the project IAM policy, pressing `r` runs:

```bash
gcloud --configuration <active-config> --project <project> \
  projects get-iam-policy <project> --format=json
```

The project is named positionally there, not with `--project` alone, so a
configuration with no project set says it has no policy to read rather than
shelling out to a command that would fail with gcloud's own wording. There is
no `--limit`: the call returns one policy object rather than a list, so a limit
would cap the number of policies, not the number of bindings inside one.

That object is flattened into a table of one row per (role, member) pair. A
binding granting one role to three members is three rows; a member holding four
roles is four rows. The shape is what lets one table answer both audit
questions with `/`: filter by a member to see what that principal can do, and
filter by a role to see who holds it. The columns are MEMBER, ROLE, TYPE (the
member's kind — user, group, service account, domain, principal, principal set,
or one of the two public populations) and CONDITION, which shows a conditional
binding's title, not its CEL expression, and a dash on the unconditional
majority.

The role is shown verbatim, exactly as the policy spells it. A custom role
reads as `projects/<project>/roles/editor` where the predefined role it shadows
reads as `roles/editor`, and shortening either would make the two
indistinguishable in the column an audit reads down.

The member cell leads with the badges the row wears:

- `PUBLIC` — the grant is held by `allUsers` (anyone on the internet, with or
  without a Google account) or by `allAuthenticatedUsers` (any authenticated
  Google account, including accounts outside your organisation).
- `DELETED` — the principal was deleted after the binding was written, so the
  grant names an identity that no longer exists.

Both are stale policy rather than errors, so a badged row is coloured as a
warning, the way an orphaned disk is. Both are also text `/` can match:
`/PUBLIC` isolates the badged rows because the badge is a searched field of its
own, and `/deleted` does the same because the member itself arrives prefixed
`deleted:` and the filter is case-insensitive. The badges lead the cell rather
than trail it because the member column is the one that elides first on a
narrow terminal, and a trailing badge would be the first thing lost on exactly
the rows it exists to mark.

The policy table is list-only and read-only: `Enter` does nothing on a row,
there is no detail pane, and nuvem never issues `set-iam-policy`. Editing an
IAM policy is out of scope for this service.

### VPC networks

For VPC networks, pressing `r` runs:

```bash
gcloud --configuration <active-config> --project <project> \
  compute networks list --format=json --limit=500
```

One flat, project-wide call: a network is a global resource, so there is no
region to iterate and no second call to make.

The columns are NAME, MODE, ROUTING and SUBNETS. MODE is `AUTO`, `CUSTOM` or
`LEGACY`; ROUTING is `REGIONAL` or `GLOBAL`; SUBNETS is how many subnets the
network has.

Both MODE and ROUTING are **derived from the API's own fields** —
`autoCreateSubnetworks` and `routingConfig.routingMode` — and never from the
`x_gcloud_subnet_mode` and `x_gcloud_bgp_routing_mode` keys gcloud synthesises
into the same JSON. Those two carry exactly the strings this table paints,
which is what makes reading them tempting, and they are gcloud's presentation
layer rather than API fields: the `x_` prefix is gcloud saying so, and nothing
commits it to keeping them.

`LEGACY` is a network that omits `autoCreateSubnetworks` altogether. It is
kept distinct from `CUSTOM` rather than folded into it because the two differ
in what an operator does next: a custom network has subnets to go and look at,
and a legacy network cannot have any. A network whose JSON carries no
`routingConfig` leaves ROUTING blank rather than reading `REGIONAL` — regional
is the API's default for a network created without one, and a default is a
claim about creation, not about what this listing reported.

SUBNETS is a count and not a list. The `subnetworks` field is an array of full
self-link URLs, 43 of them on the default network of a fresh project, which is
not a table cell; the subnets themselves are a table of their own. A count of
`0` is painted as `0` and never as a dash: the API listed the network's subnets
and found none, and a custom-mode network nobody ever added a subnet to is a
finding rather than a missing value.

A network's description has no column — it is unbounded free text that would
push the four facts above off a narrow terminal — but it is searchable and the
preview pane shows it in full. `/` narrows wholly client-side here: nothing is
pushed to the server, because three of the four columns are derived values
that appear verbatim in no raw field, and a pushed clause would drop rows an
operator can read on screen. Label filtering (`-l k=v`) reports unsupported,
because the `compute/v1` networks resource has no labels field at all.

Because the narrowing is client-side, applying a filter runs **no gcloud
command at all** — it re-narrows the rows already in memory. That is not only
an efficiency point. A list refresh that fails clears the table and its
context, so a table refreshed on every filter keystroke would go blank on a
transient auth blip or quota error, in response to the operator typing. `r`
still fetches, because `r` is an explicit ask for fresh rows.

The networks table is list-only and read-only: `Enter` does nothing on a row,
there is no detail pane, and nuvem never creates, deletes or modifies a
network. `L` is refused rather than scoped to the selected network — VPC flow
logs and firewall hit logs are attributed by Cloud Logging to the *subnetwork*,
so a network-scoped query would return zero rows while looking to an operator
like it should return many.

There is no `gateway` column: `gatewayAddress` is a subnet field, and a
network carries no address of its own.

The fixture behind demo mode and the tests, `tests/fixtures/vpc_networks.json`,
is a live capture taken on 2026-08-25 with the project id replaced by
`nuvem-fixture-project`. Its field set and key spellings are the API's own.
The `CUSTOM`, `GLOBAL` and `LEGACY` cases are not in it — the account it was
captured from can reach no such network — so those arms are pinned against
rows constructed from Google's published `compute/v1/networks` reference, and
the tests that use them say so.

### VPC subnets

For VPC subnets, pressing `r` runs:

```bash
gcloud --configuration <active-config> --project <project> \
  compute networks subnets list --format=json --limit=500
```

One flat, project-wide call despite a subnet being a *regional* resource:
`networks subnets list` returns every region at once, so there is no per-region
fan-out and no `--regions` flag. Every region's subnets are in one table, and
REGION is a column you sort by rather than a scope you enter.

The columns are NAME, REGION, NETWORK, RANGE, GATEWAY and PGA.

**A subnet's name does not identify it.** An auto-mode network gets one subnet
per region and calls every one of them `default`, so a fresh project's table is
43 rows sharing a single name, told apart only by REGION. That is why REGION
sits immediately behind NAME in the column order and is the last column a
narrow terminal drops: without it the rows are indistinguishable. It is also
why a jump rule from this service can use `{location}` as well as `{name}`, and
why the preview pane heads itself `default (us-central1)` rather than
`default`.

REGION and NETWORK are painted as **ids**, not as the self-link URLs the API
sends. `region` and `network` both arrive as
`https://www.googleapis.com/compute/v1/projects/<p>/regions/us-central1` and
similar; the last path segment is taken once, in the parser, so no two cells
can disagree about what a network is called.

PGA is `PRIVATE GOOGLE ACCESS`, abbreviated because the full phrase is 21
characters carrying a 3-character cell — and it is the abbreviation the console
and the `gcloud` flag both use. It reads `On` or `Off`, never a dash: the API
documents an absent flag as off, so `Off` is an answer rather than a missing
value.

`L` works here, and it is the **only** VPC table it works on. Cloud Logging
attributes both VPC flow logs and firewall hit logs to the subnetwork
(`resource.type="gce_subnetwork"`), never to the network and never to the
firewall rule — which is exactly why the networks table refuses `L` rather than
offering a query that could only return zero rows. An operator asking what
traffic a network is carrying is asking about its subnets.

Narrowing stays wholly client-side, for the reason the networks table's does:
REGION and NETWORK are the tails of URLs and PGA renders a bool as a word, so
none of the three is a verbatim substring of a raw field. Label filtering
reports unsupported, because the `compute/v1` subnetworks resource has no
labels field. Applying a filter therefore runs no gcloud command here either,
and for the same second reason: a refresh that fails clears the table, so a
redundant one would turn a transient error into a blank screen the operator
caused by typing.

The subnets table is list-only and read-only: `Enter` does nothing on a row,
there is no detail pane, and nuvem never creates, deletes or modifies a subnet.

`tests/fixtures/vpc_subnets.json` is a live capture taken on 2026-08-25 with
the project id replaced by `nuvem-fixture-project`, trimmed from the project's
43 rows to three. The trim removes rows and changes no field of the rows it
keeps; three is what it takes to show the one-name-many-regions shape the table
is built around.

### Firewall rules

For firewall rules, pressing `r` runs:

```bash
gcloud --configuration <active-config> --project <project> \
  compute firewall-rules list --format=json --limit=500
```

Flat and project-wide like the other two VPC tables. A firewall rule is a
*global* resource that names the network it applies to, so there is no
per-network fan-out and no `--network` flag; NETWORK is a column you sort and
filter by rather than a scope you enter.

The columns are NAME, NETWORK, DIRECTION, PRIORITY, ACTION, TARGETS and
SOURCE/DEST.

**ACTION is derived, and it is the cell this table exists for.** The API
reports no action field. It reports an `allowed` array or a `denied` array, and
exactly one of the two is ever present; the parser collapses that into one
value, so no later reader can find the pair in a state the API never sends and
guess at it. A rule carrying neither key, or both, paints a dash rather than a
verdict: `ALLOW` over a rule that blocks, or `DENY` over one that does not, is
worse than a blank cell an operator goes and checks. ACTION therefore sits
immediately behind NAME in the drop order — a list of rules that cannot say
what any of them does says nothing at all.

**A disabled rule is badged on its name, not given a column.** A disabled DENY
rule reads exactly like a live DENY rule in every other cell — same name, same
direction, same priority, same ranges — while the traffic an operator believes
is blocked is flowing. So `DISABLED` rides the identity column, which is the
one column the layout can never drop, and it leads the cell rather than
trailing it, because the name elides from the right. The preview pane heads
itself `rule-name (DISABLED)` for the same reason.

PRIORITY sorts numerically, and ascending by it is evaluation order: lower
priority wins, so that is the order traffic actually meets the rules in. A rule
whose listing reported no priority sorts as absent rather than as zero — zero
would put it ahead of every real rule and claim it is evaluated first, which is
a statement about a value nothing reported. The cell dashes rather than
painting the API's `1000` default, for the reason the networks table dashes an
absent routing mode.

TARGETS is the rule's target tags or target service accounts, whichever scopes
it. An empty list is **not** a blank: a rule with neither applies to every
instance in the network, which is the widest scope there is, so the cell reads
`all instances`. A blank there would read as the narrowest scope and invert the
fact. SOURCE/DEST follows the direction — source ranges on an INGRESS rule,
destination ranges on an EGRESS one — because the two are never both the
question.

NETWORK is painted as an **id**, not as the self-link URL the API sends, using
the same helper the other two VPC tables use.

`/` narrows wholly client-side. `ALLOW`, `DENY`, `all instances` and the
`DISABLED` badge are all words this application puts on the screen and no raw
field contains, so a pushed clause would drop rows an operator can read. All
four are searchable, which is what lets `/DENY` answer "what does this project
block" at all. Label filtering (`-l k=v`) reports unsupported: the `compute/v1`
firewalls resource has no labels field.

Applying a filter runs no gcloud command, as on the other two VPC tables — and
this is the table where that matters most. A refresh that fails clears the
rows, so an operator who types `/DENY` to ask what a project blocks would, on
any transient failure of a call that was never needed, get a blank table back
instead of the answer. `r` still fetches, because `r` is an explicit ask.

The rules table is list-only and read-only: `Enter` does nothing on a row,
there is no detail pane, and nuvem never creates, deletes, enables or disables
a rule. The one action worth having here would be the most dangerous button in
the application.

`L` is refused, as it is on the networks table and for a sharper version of the
same reason. A rule's hits *are* logged — against the subnetwork the traffic
crossed, with the rule named in the payload — so nothing writes an entry whose
resource is the rule. A rule-scoped query would return zero rows, and on a DENY
rule zero rows reads as "nothing was blocked".

`tests/fixtures/vpc_firewall_rules.json` is the one **mixed** fixture in this
group. Its four `default-allow-*` rules are a live capture taken on 2026-08-25
with the project id replaced by `nuvem-fixture-project`. The other three —
`allow-web-to-tagged`, `deny-all-egress` and `disabled-sa-scoped-ssh` — are
constructed from Google's published `compute/v1/firewalls` reference, because
no project the capturing account can reach has an egress rule, a `denied` rule,
a target-tagged rule, a service-account-scoped rule or a disabled rule. Every
interesting parser arm in this table therefore rests on transcription rather
than on bytes gcloud produced, including the two whose misrendering is
dangerous, and the tests that use them say so.

## Cloud Logging

The Logging workspace is a top-level workspace, a peer of Home,
Configurations, and Resources. There are three ways into it, and they differ
only in what the query is scoped to:

- **`l`** — the Logging workspace on **all recent project logs**, from
  anywhere in the app. It never looks at your selection.
- **`L`** (shifted) — Cloud Logging scoped to the **one resource the current
  view has on screen**. Available anywhere in the Resources workspace: in a
  resource list and in a resource detail pane.
- **The command palette** — `Open project logs` (aliases `logs`, `logging`) is
  the palette form of `l`; the `Open logs for selected ...` entries are the
  palette form of `L`, one per service, offered only when that service has a
  resource the jump can resolve.

`l` and `L` differ only in case and land in the same workspace, so it is worth
stating the distinction once: `l` widens to the project, `L` narrows to the
selection. The help overlay (`?`) says the same thing on both entries.

`L` covers all eleven services — the selected GCE instance, GKE cluster, Cloud
SQL instance, Cloud Run service, Cloud Storage bucket, secret, Cloud Function, Pub/Sub topic, Pub/Sub subscription, BigQuery dataset, or VPC subnet — and resolves the
target from the view rather than from the raw list cursor. Inside a Cloud Run
revision it scopes to that revision rather than to its service; inside the
Cloud Storage object browser, to the bucket being browsed; and with a detail
pane open it follows the pane, not the list cursor still movable behind it.
When nothing can be resolved — an empty list, or no active gcloud context — the
status line says so and the workspace does not change. A jump that does land,
whether from `L` or from the palette, is recorded in navigation history, so
`Alt-←` returns to the resource you jumped from.

Resource-scoped queries are shown as the exact generated filter string in the
workspace header; the project-wide entry shows an `(all recent project logs)`
placeholder. Two of those queries interpolate the project into the identity
they match on — Cloud SQL log entries are keyed by a composite
`<project>:<instance>` database ID, a secret's by the full resource name
`projects/<project>/secrets/<name>` — so those two jumps refuse (with a status
message) when the active context has no project rather than issuing a filter
that would silently match nothing. Logging reads are read-only: the workspace never
mutates log sinks, metrics, or exclusions.

While the tail is running (the default when a query opens), the workspace
polls for new entries on the same interval as `--poll-interval`, in a single
request at a time; new entries are appended and deduplicated by insert ID,
and the buffer is capped so memory use stays bounded. On a quota/rate-limit
failure, polling backs off for a growing number of ticks before retrying;
other errors show a degraded status while keeping the last-known entries
visible.

In the Logging list, use `j`/`k` or the arrows to move the selection and
`Enter` to open the selected entry's detail (payload and labels), `Enter`,
`Esc`, or `q` to close it. Press `t` to pause or resume the tail, `w` to
toggle line wrapping of long message/payload lines in the entry detail view
(wrapped instead of truncated), and `/` to edit a client-side search over the
loaded entries (`Enter` applies it, `Esc` cancels the draft). Press `y` to
copy the selected entry and `i` to copy its insert ID, both from the list and
from an open entry's detail. While an entry's detail is open, use
`PageUp`/`PageDown` to scroll it.

`r` asks for the current window again and keeps the rows already on screen, so
a page that comes back unchanged leaves the view as it was. When no active
gcloud context resolves there is no project to read from and nothing is
fetched; because the rows stay put, the status line reports the missing context
rather than letting the refused refresh look like one that returned the same
page.

Two axes narrow the window server-side, by changing the `gcloud logging read`
call rather than hiding rows that were already fetched. `s` raises and `S`
lowers the severity floor one rank at a time (`DEBUG`, `INFO`, `NOTICE`,
`WARNING`, `ERROR`, `CRITICAL`, `ALERT`, `EMERGENCY`), and the steps stop at
either end rather than wrapping; the default floor is `all`, which adds no
severity clause at all. The since window is seven buckets addressed directly
by the digits `0`-`6` — `1m`, `5m`, `15m`, `30m`, `1h` (the default), `6h`,
`24h` — and `T` still cycles through them shortest to longest. The header
shows both, as `Severity: <floor>` and `Range: <window> [<digit>]`.

The `/` search runs over the loaded buffer and understands four forms: a bare
term is a case-insensitive substring (the default), `~term` is a fuzzy
subsequence, `/pattern/` is a case-insensitive regex, and a leading `!`
inverts any of them. The header spells the sigils out while nothing is
applied and names the mode a pattern selected once one is; a regex that does
not compile shows no rows and reports the reason. Substring rather than regex
is the default so that a bracketed level such as `[ERROR]` matches the
brackets that were typed instead of being read as a character class;
`ARCHITECTURE.md` records why, and why regex stays an explicit opt-in.

`Ctrl-S` saves the currently visible (that is, filtered) entries to
`log-dumps/logs-<timestamp>.log` beside nuvem's own log file — run `nuvem
info` for that path — one sanitized line per entry, and reports the absolute
path it wrote. With nothing visible it writes no file, and when no log
directory resolves it reports a save failure rather than falling back to the
working directory.

Select a GKE cluster and open the palette with `:` to run `Open selected GKE
cluster in k9s` (aliases `k9s`, `open`). The action appears only when the active
configuration provides a project and the cluster has a location. nuvem first
checks (best-effort, via `kubectl config get-contexts`) whether the deterministic
kubeconfig context `gke_<project>_<location>_<cluster>` already exists, then shows
a confirmation modal that spells out whether it will reuse that context or run
`get-credentials` first, the exact argv for both commands, and a note that
`get-credentials` modifies your local kubeconfig:

```bash
gcloud --configuration <active-config> container clusters get-credentials <name> --location <location> --project <project>
k9s --context gke_<project>_<location>_<cluster>
```

Press `Enter` to launch or `Esc` to cancel before either command runs. When a
credentials refresh is required it runs first; if it fails, k9s is not launched
and the error is surfaced. The launch itself reuses the shared interactive-process
runtime, so nuvem suspends the TUI while k9s owns the terminal and restores it
(preserving your list selection and detail state) when k9s exits. If `k9s` is not
on your `PATH` you get a diagnostic and cluster browsing stays usable. Commands
are invoked without a shell, so cluster names and locations are never interpolated
into a shell string. Override the binaries with `--k9s-binary` and
`--kubectl-binary` when they live outside `PATH`.

For Cloud Run services, pressing `r` runs:

```bash
gcloud --configuration <active-config> [--project <project>] run services list --platform=managed --format=json --limit=500
```

This lists services across all regions in one call. Select a service and
press `Enter` to load a read-only detail view, scoped to that service's
region:

```bash
gcloud --configuration <active-config> [--project <project>] run services describe <name> --platform=managed --region <region> --format=json
```

Use `[` and `]` to move across Summary, Traffic, Revisions, Configuration,
Conditions, and JSON tabs. On the Revisions tab, press `Enter` to drill into a
revision's own read-only detail (Summary, Conditions, and JSON tabs), fetched
with the same region scoping. `Esc`, `q`, or history-Back returns to the
service's Revisions tab without losing the prior selection. With a service
selected in the list, the command palette's `Open logs for selected Cloud Run
service` entry opens resource-scoped logs in the Logging workspace; `L` does
the same from the list and from the service's detail pane, and from inside a
revision it scopes to that revision instead (the palette offers `Open logs for
selected Cloud Run revision` there). `J` on a service offers its Revisions tab
unless you configure otherwise — see
[Related-resource jumps](#related-resource-jumps). Cloud Run mutation
(deploy, traffic management, delete, rollback) is not supported.

For Cloud SQL instances, pressing `r` runs:

```bash
gcloud --configuration <active-config> sql instances list --format=json --limit=500
```

Select an instance and press `Enter` to load a read-only detail view. The detail
command is:

```bash
gcloud --configuration <active-config> sql instances describe <name> --format=json
```

Use `[` and `]` to move across Summary, Connectivity, Maintenance, Backups,
Replicas, Users, and JSON tabs. The Users list is a second `gcloud sql users
list` call, lazy-loaded the first time the Users tab is opened. Passwords and
certificates are redacted at parse time and never appear on any tab, including
the raw JSON one.

In the Cloud SQL workspace, select an instance and press `:` to open the command
palette. Mutation availability follows the instance status: stopped instances
can be started; runnable instances can be stopped or restarted; and Failover is
offered only for runnable HA/REGIONAL instances. Every mutation opens the shared
confirmation modal showing the exact gcloud command before it runs — Start/Stop
via `sql instances patch --activation-policy=ALWAYS|NEVER`, Restart and Failover
via the `sql instances restart`/`failover` verbs. Press `Enter` to confirm or
`Esc` to cancel. On success the app refreshes the instance list as with GCE.
The same four mutations also run against a marked set, in which case the
palette offers a command when any marked instance is eligible for it — see
[Batch actions on marked rows](#batch-actions-on-marked-rows).

In the Configurations workspace, press `a` to create a native gcloud
configuration. The form accepts name, account, project, region, and zone. Name
is required; optional blank fields are skipped. Press `Enter` to create or
`Esc` to cancel.

In the Configurations workspace, press `e` to edit the active gcloud
configuration's project, region, and zone. Blank submitted fields unset the
corresponding native gcloud property. Press `Enter` to save or `Esc` to cancel.

While editing the active gcloud context, press Ctrl-Space on the focused
Project, Region, or Zone field to discover values. Use / to filter, j/k
or the arrows to select a row, and Enter to copy it into the draft. While
filtering, use Backspace to remove filter characters, Enter to apply the
filter, or Esc to cancel the filter draft and restore the applied picker
filter. Press Enter again to save, r to retry discovery, or Esc to go back
from the picker. Project rows can be filtered by ID, name, or lifecycle state;
region rows by name or status; and zone rows by name, region, or status. The
selection follows the row it is on while the filter narrows or widens the
visible list, so rows appearing or disappearing above the cursor never move it
onto a different item; when the selected row itself stops matching, the
selection falls back to the nearest still-visible row.

The Configurations workspace renders name, account, project, region, and zone
for each loaded native gcloud configuration, using `-` for missing values. Press
`/` to edit a case-insensitive filter across those fields, `Enter` to apply it,
and `Esc` to cancel the draft. Press `u` to clear the applied configuration
filter.

To activate an inactive selected configuration, press `Enter` or open `:` and
choose `Activate selected gcloud configuration`. Both paths show a confirmation
with the target context and exact `gcloud` command. Press `Enter` or `y` to
confirm, or `Esc` or `n` to cancel. If the terminal cannot show the full command
preview, enlarge it before confirming. Requesting activation for the active
configuration only shows a no-op status. If the reload does not show the target
as the sole active configuration after gcloud succeeds, the app keeps the target
context and shows a reload warning.

Every resource list refresh is capped at 500 rows (`--limit=500`), so a poll no
longer downloads and holds the full result set. Press `/` in the resource list
to edit a filter, `Enter` to apply it, and `Esc` to cancel the draft. For GCE,
GKE, and Cloud SQL, applying or clearing the filter refreshes the list and
pushes the term down to gcloud as a server-side `--filter` when it is safely
expressible against raw fields, then narrows the residual client-side; a term
that could only match a transformed or derived display value falls back to
client-side-only narrowing so a matching row is never dropped. Cloud Run is
capped but always narrowed client-side (its Knative list shape isn't reliably
server-filter-expressible). Secret Manager also refreshes on apply and clear,
but pushes only label mode: a secret's replication and its `key=value` label
rendering are display values with no raw field behind them, so every other
mode narrows client-side. Cloud Functions behaves the same way and for the same
reason: label mode is pushed as `labels.k = "v"`, every other mode narrows the
loaded rows on every cell the table paints — name, region, state, generation,
update time, invoke URL and `key=value` labels. State and generation are
narrowed on the words the column shows (`Active`, `1st gen`), which appear in
no raw gcloud field, which is exactly why nothing but label mode is pushed.
Pub/Sub topics behave the same way again: label mode is pushed as
`labels.k = "v"`, and every other mode narrows the loaded rows on every cell the
table paints — name, subscription count, retention, KMS key and `key=value`
labels. The count is the clearest case for keeping the rest client-side: it is
derived from a second list call and appears in no raw field of the topic
resource at all. The displayed narrowing is still client-side: GCE
filters match name, zone, status, machine type, internal IP, and external IP;
GKE filters match name, location, status, Kubernetes version, node count, and
endpoint visibility.

The filter text is a small prefix grammar with four modes. It applies to GCE,
GKE, Cloud SQL, Cloud Run, Secret Manager, Cloud Functions, and Pub/Sub topics:

| Input          | Mode      | Meaning                                             |
| -------------- | --------- | --------------------------------------------------- |
| `term`         | `regex`   | Case-insensitive regex, partial match, any field     |
| `-f term`      | `fuzzy`   | Case-insensitive subsequence match, any field        |
| `-l k=v[,k=v]` | `label`   | GCP resource labels; every pair must match           |
| `!term`        | `inverse` | Rows the `regex` predicate does **not** match        |

`regex` is the default, so a plain word keeps behaving the way the old
substring filter did. An empty input — and a prefix with nothing after it
(`-f`, `-l`, `!`) — means "no filter": every row stays visible and nothing is
reported, because you are mid-typing and an error there would be noise. The
mode renders as a `[mode]` chip next to the filter text. While the input is
open the chip tracks the *draft*, so it flips to `[fuzzy]` the moment you type
`-f`, before anything is applied; that is what makes the grammar discoverable.

Label mode is server-side only. `-l` narrows nothing locally on any service —
almost no list row carries labels to match against, and the client-side
predicate keeps every row even where one does (a secret's does) — so the term
is pushed to gcloud as a `--filter` and whatever comes back is shown as-is.
A label-mode read therefore always goes through `gcloud`, even on the services
that otherwise read through the native client libraries: it is the one filter
mode the native path cannot honour, and an unfiltered table under a `[label]`
chip would be worse than the slower call.
Three consequences follow. Label values match
**case-sensitively** on the server, unlike every other mode, so `-l env=Prod`
will not find a `prod` label. `-l` does nothing under `--demo`, which serves
embedded fixtures and ignores `--filter`. And Cloud Run exposes no user labels
in list output, so `-l` there reports
`label filtering is not supported for Cloud Run` and leaves the table unfiltered
rather than pretending everything matched. Keys and values are
limited to ASCII letters, digits, `.`, `_`, and `-`, and a key may not contain
`.` (GCP label keys cannot); anything else reports `label selector is not
expressible as a gcloud filter`.

A malformed filter never empties the table. `a(b` shows **every** loaded row
plus an inline `⚠ invalid regex: unclosed group` next to the filter text, and
`-l env` shows every row plus `⚠ malformed label selector: expected key=value`.
Nothing panics and nothing silently disappears: an empty table behind a bad
pattern is indistinguishable from a correct empty result, so nuvem shows too
much and says why. The `⚠` describes the *applied* filter, so it is dropped as
soon as your draft diverges from what is applied.

Cloud Storage keeps the plain case-insensitive substring filter and shows
no mode chip. Its object browser already pushes safe filter text down as a
server-side object prefix, and grammar prefixes would fight that pushdown for
no benefit. The grammar therefore does not apply there at all, and the
consequence is worth stating plainly: on Cloud Storage `-l env=prod`, `-f api`,
and `!x` are not modes, they are text. In the bucket list that text is
matched as literal substrings; in the object browser it goes through the same
pushdown as any other text, so `-l env=prod` (no leading `/`, no glob
characters) is sent as a server-side object prefix and the tools line reads
`-l env=prod (remote)`. Either way it normally matches no bucket or object
name and you get an empty table with no chip and no `⚠`:
Cloud Storage reports no filter error, because it never parses the grammar in
the first place.

Press `←`/`→` to move the selected column, `s` to sort by it (press `s` again
to reverse direction), and `w` to toggle wide columns.

## Command palette

Press `:` from any view to open the palette as a global command mode. It lists
navigation commands — Home, Configurations, Tunnels, Logging (`Open project
logs`), each service (GCE instances, GKE clusters, Cloud SQL instances, Cloud
Run services, Cloud Storage buckets, Secret Manager, Cloud Functions, Pub/Sub
topics), Refresh, Help, and Switch
project / context — alongside the actions valid for the current selection. Type
to search: entries are ranked by exact alias or label, then prefix, then
substring, and commands expose aliases such as `gce`, `compute`, `gke`,
`clusters`, `cloud sql`, `sql`, `run`, `serverless`, `cloud run`, `gcs`,
`storage`, `bucket`, `secret`, `secrets`, `sm`, and `tunnels`. When a search
matches nothing the palette shows the query and a recovery hint; opening it
never starts empty.

In the GCE workspace, select an instance and press `:` to open the command
palette. Mutation availability follows the instance status: stopped instances
can be started; running instances can be stopped, reset, or suspended; and
suspended instances can be resumed. Every mutation opens a confirmation modal;
the app shows the exact gcloud command before it runs. Press `Enter` to confirm
or `Esc` to cancel. If the terminal is too small to show the full command
preview, enlarge it before confirming. After a successful mutation, the old
rows are cleared, the shared status area shows a refresh-in-progress state,
and the app automatically runs the active-context instance-list command once,
carrying GCE's own applied filter — a reload never borrows another service's
filter term, and never drops the reloaded service's own, because in label mode
the narrowing lives entirely in the pushed `--filter`:

```bash
gcloud --configuration <active-config> compute instances list --format=json --limit=500
```

with `--filter=<expression>` appended when that filter is server-expressible,
exactly as a manual refresh would push it.

If that follow-up refresh fails, the resource list remains stale/empty and the
shared status area shows the gcloud error. Press `r` to retry the refresh
manually.

The palette also offers `Show SSH command for selected GCE instance`, which
opens a preview of the exact SSH command. Press `Enter` to copy it or `Esc` to
cancel. You can copy the selected instance name and zone, any available
internal or external IP, and its self link when a project is configured.
Clipboard success or failure appears in the shared status area.

For a running instance, the palette also offers `SSH to selected GCE
instance` (alias `ssh`), which opens a preview of the exact `gcloud compute
ssh` command. Press `Enter` to launch it or `Esc` to cancel. Launching
suspends the TUI and hands the terminal to the SSH session; when the session
exits, the TUI redraws and the shared status area reports how it ended
(finished, exited with a non-zero status, terminated by signal, or failed to
launch). This differs from `Show SSH command`, which only copies the command
without running it.

## Related-resource jumps

Press `J` (shifted) in a resource list or a resource detail pane to open the
related-resource jump picker: a small modal listing the collections you can
reach from the resource currently on screen. Each row names the destination tab
and the concrete resource — `Network of web-01`, `Replicas of orders-db` — so
one key means the same thing in every service without reading as a generic
"jump". Move with `j`/`k` or the arrows (the cursor clamps at both ends rather
than wrapping), press `Enter` to jump, or `Esc` to cancel. The offers are
captured when the picker opens, so a background list reload cannot change what
the row you are looking at means. Jump rules are uncapped and user-configured,
so when more are offered than the terminal can show at once, the row list
scrolls within the modal to keep the selected row visible — every configured
rule stays reachable with `j`/`k`.

The jumps offered out of the box. The first three land on a detail tab; the
last two land on the VPC networks table with the cursor on the network the
subnet or the rule is attached to:

| Selection             | Jumps offered      |
| --------------------- | ------------------ |
| GCE instance          | Network, Disks, VPC networks, VPC subnets, Firewall rules |
| Cloud SQL instance    | Replicas           |
| Cloud Run service     | Revisions          |
| GKE cluster           | none               |
| Cloud Storage bucket  | none               |
| secret                | none               |
| Cloud Function        | none               |
| persistent disk       | none               |
| Pub/Sub topic         | none               |
| Pub/Sub subscription  | none               |
| BigQuery dataset      | none               |
| service account       | none               |
| policy binding        | none               |
| VPC network           | none               |
| VPC subnet            | VPC networks       |
| firewall rule         | VPC networks       |
| Cloud Run revision    | none               |

That table is the default, not the limit: a `[[jumps]]` array in the config file
replaces it with rules of your own, including jumps from the services that
offer none by default and jumps that cross from one service to another's detail
tab. See *Jump rules* under [Configuration](configuration.md#configuration) for the keys and the
template vocabulary.

Nothing new is fetched to build the picker; the destinations are views the app
already renders, so a jump costs at most the detail load that opening that
resource normally costs, and a jump onto a list costs nothing at all. Following
one opens a pane, or moves a list cursor, only for a resource already in the
destination service's loaded list, matched on name and location both, and
reports on the status line when there is no such row — but
when that list has never been loaded at all, the jump starts its fetch on the
way past, so the report is a "try again once they arrive" rather than a dead
end. Only a never-loaded list gets that; one whose load is already in flight,
or one whose last load failed *in this configuration* and is waiting on the
retry backoff, is left alone. Every one of those questions is asked of the
configuration you are in now, so switching configuration and jumping again
fetches rather than inheriting the old one's verdict. That is worth knowing
before writing a rule that crosses from one service to another — see
*Jump rules*.

A selection with no applicable jumps never opens an empty picker. `J` reports
on the status line instead, and says which reason applies: no rule offers
anything from this service
(`No related-resource jumps configured for GKE clusters`,
`No related-resource jumps configured for Cloud Storage buckets`,
`No related-resource jumps from a Cloud Run revision`), nothing is selected to
jump from (`No GCE instance selected to jump from`), or a rule that does exist
cannot be resolved for this row
(`Jump "compute:network" needs external_ip, which this GCE instance has no value for`).
The distinction matters because each one is fixed somewhere else: by the config
file, by selecting a row, or by the row itself.

Like `L`, a jump follows the view rather than the raw list cursor: with a
detail pane open the pane's resource is the origin, and the nested Cloud Run
revision view is neither an origin nor a destination. Landing is a tab change
when the pane already shows that resource — exactly what `[`/`]` would do —
and a detail open otherwise. A jump that moves is recorded in navigation
history, so `Alt-←` returns to where you jumped from and `Alt-→` returns to
the jump. Picking the tab you are already on does nothing at all beyond closing
the picker: it records no history, and it leaves the pane's scroll position,
field cursor, and revision cursor exactly where you left them.

## Tunnels

nuvem can hold open long-running local port forwards to your cloud resources
and keep them in a list you can watch and kill. Unlike an SSH session, a tunnel
never takes over your terminal and keeps running while you carry on using the
app.

Press `t` on a row in the GCE instances or Cloud SQL list to start one, or use
the palette's `Start tunnel for selected GCE instance` / `Start tunnel for
selected Cloud SQL instance` (aliases `tunnel`, `pf`, `forward`). nuvem picks
the tunnel kind for you:

- a GCE instance **with** an external IP gets a plain SSH local forward
  (`gcloud --configuration <active-config> compute ssh <name> --zone <zone> -- -N -o ExitOnForwardFailure=yes -L <local>:localhost:22`),
  where `ExitOnForwardFailure=yes` makes a forward that cannot bind its local
  port end instead of sitting there carrying nothing;
- a GCE instance **without** an external IP is only reachable through
  Identity-Aware Proxy, so it gets
  `gcloud --configuration <active-config> compute start-iap-tunnel <name> 22 --local-host-port=localhost:<local> --zone <zone>`;
- a Cloud SQL instance gets the Cloud SQL Auth Proxy
  (`cloud-sql-proxy --address 127.0.0.1 --port <local> <project>:<region>:<name>`),
  with the local port defaulting to the engine's usual one — 5432 for
  PostgreSQL, 3306 for MySQL, 1433 for SQL Server — and falling back to a free
  port when that one is taken. Override the executable with
  `--cloud-sql-proxy-binary` when it is not on `PATH`.

A preview shows the exact command before anything runs; press `Enter` to
launch or `Esc` to cancel. Commands are invoked without a shell, so instance
names, zones, and connection names are never interpolated into a shell string.
Every tunnel listens on loopback only (`localhost` / `127.0.0.1`), so a forward
is never reachable from outside the machine.

Open the list with `Ctrl-t` from anywhere, or with the palette's `Tunnels`
entry (a bare `t` is taken by the Logging tail toggle). It shows each tunnel's
status, kind, local port, target, age, and last message. Inside it:

- `j` / `k` or the arrows move the selection;
- `Shift-K` kills the selected tunnel;
- `d` dismisses a stopped or failed row;
- `Esc` goes back to the previous view.

A tunnel that cannot start says why rather than hanging: a missing binary, no
free local port, the port being taken between the check and the child's own
bind, or a child that exits or never reports itself ready all land the row in
`Failed` with the underlying error text. Once a tunnel is up, a complaint from
the child that does not stop it carrying traffic — an `ssh` forward reporting
one refused connection, say — is shown in the message column and leaves the
tunnel running. A kill that does not work says so the same way: the row lands in
`Failed` with the reason instead of reporting a stop nobody observed. A killed
tunnel always settles on `Stopped` or `Failed`, so it never sits in `Stopping`
holding its local port with no way to dismiss it. Tunnels stay in the list when
you switch gcloud configuration — the child is still running, and each row
records the context it was started under — and every remaining tunnel is shut
down when you quit nuvem, when the terminal that started it closes, or when
nuvem is terminated.

## Batch actions on marked rows

The GCE instances and Cloud SQL instances lists support marking multiple rows
so a mutation targets all of them at once, alongside the single-instance flows
above. Marking is offered only on services that have lifecycle actions to
apply; GKE, Cloud Run, Cloud Storage, Secret Manager, Cloud Functions, and
Pub/Sub topics are read-only here, so
the mark keys do nothing there.

- Press `Space` to mark or unmark the selected instance; the toggled instance
  becomes the mark anchor.
- Press `Shift-V` to range-mark every instance between the anchor and the
  current cursor, inclusive, in the visible (filtered/sorted) order.
- Press `\` to clear every mark.
- Press `Shift-U` to unmark only the currently visible (filter-matching)
  marks, leaving marks on rows hidden by the active filter untouched.

Marks are per service: marking GCE instances and then switching to Cloud SQL
shows Cloud SQL's own (initially empty) marks, and a batch never mixes the two.

Whenever at least one instance is marked, the resource list title shows a
`Marked: <n>` indicator; if the active filter hides some marked instances it
shows `Marked: <n> (<hidden> hidden)` so the total never silently drops out of
view.

With one or more instances marked, opening the command palette (`:`) turns that
service's lifecycle commands into batch commands: instead of the single
selected row, the action targets the whole marked set. That is
`Start`/`Stop`/`Reset`/`Suspend`/`Resume selected GCE instance` in the GCE
workspace, and `Start`/`Stop`/`Restart`/`Fail over selected Cloud SQL instance`
in the Cloud SQL workspace. Marked instances that are not eligible for the
chosen action (for example a stopped instance targeted by Stop, or a zonal
non-HA instance targeted by Fail over) are skipped rather than silently
dropped.

The batch confirmation names the service it targets, and lists every eligible
target by name together with its exact per-item `gcloud` command, followed by
any skipped targets and their reason (`ineligible (<status>)`, or
`ineligible (requires an HA (regional) instance)` for a Cloud SQL failover on a
zonal instance), so the operator can verify the complete target set before
committing — it is never summarized or truncated (the modal scrolls when it
overflows). Destructive batch actions (GCE Stop, Reset, and Suspend; Cloud SQL
Stop, Restart, and Fail over) prompt `Type <n> to confirm` and require typing
that exact number of eligible targets, a stronger acknowledgement than the
single-resource confirmation. Non-destructive batch actions (GCE Start and
Resume; Cloud SQL Start) prompt `Enter to confirm` like the single-resource
flow.
Press `Esc` to cancel a batch confirmation.

Once confirmed, targets run with bounded concurrency — at most
`--batch-concurrency` (default `4`) actions in flight at a time — so a large
marked set never spawns unbounded concurrent `gcloud` processes. While the
batch runs, a progress panel shows an aggregate line
(`<ok>/<total> done · <fail> failed · <pending> pending`) and one line per
target with its current status (`PENDING`, `OK`, or `FAIL: <reason>`); a very
long target list is capped with a `+N more` line. This panel is read-only —
the keyboard keeps working normally while a batch is in flight. Once every
target resolves, the app issues a single follow-up instance-list refresh, and
the progress panel is dismissed once that refresh applies fresh rows.

The live batch progress panel tracks every target regardless of batch size, so
batch **monitoring** is never limited by any cap. The Home overview's "Recent
operations" list is a separate, intentionally bounded **history** log (it keeps
only the most recent entries); for a batch larger than that capacity only the
most recent targets remain in that historical list. This cap is history-only —
it never affects live batch monitoring, which the per-target progress panel
above renders in full.

Switching gcloud configuration or context while a batch is running safely
detaches it: the progress panel disappears immediately, any targets already
running are left to finish but their results are discarded, and any
not-yet-started targets are cancelled and never run. No target is left showing
as still in progress afterwards: each one's "Recent operations" entry is
settled at what is actually known — `cancelled` for a target that never issued
its command, `unknown` for one whose command was already in flight when the
switch happened and whose real outcome is therefore unobserved.

Confirming a new batch while one is still running detaches the running batch
the same way — its remaining targets are settled as `cancelled` or `unknown` as
above — and the progress panel then tracks the newly confirmed batch. Those
entries say the batch was superseded by a newer one rather than cancelled: the
outcome is the same because it is what is provable about each target, but you
cancelled nothing here, you confirmed a replacement.

In a resource list, select an item and press `Enter` to load a read-only detail
view. Use `[` and `]` to move across the service's detail tabs. `Esc` goes back
to the prior view — from a resource detail view that returns to the resource
list; from a resource list it steps back through navigation history (for
example, to the previous service or workspace) when there is history to go
back to. While a resource detail view is open, `q` also returns to the
resource list. `Alt-Left` and `Alt-Right` move back and forward through
navigation history from anywhere in the Resources workspace; the footer shows
these hints only when there is somewhere to go. Navigation history is reset
whenever the active gcloud context changes.
From the home screen and list views, `q` quits the app. `Ctrl-C` quits from any
mode.
Empty projects, stale rows, and refresh failures are shown in the resource
workspace instead of leaving the resource panel blank.

On wide terminals (body at least 100 columns), the Resources workspace splits
into a left list pane and a right pane: a summary preview of the selected row
in list view, or the full tabbed detail view when a detail is open. The list
stays visible in both cases. This uses only already-loaded resource data and
does not trigger an extra `gcloud` call. Narrower terminals, including the
`60x12` minimum, keep the existing full-screen list or detail view.

---
title: Columns and sort
description: Choose which columns each service shows, in which order, and its default sort.
sidebar:
  order: 6
---

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

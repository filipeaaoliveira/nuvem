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
matched case-insensitively. Each service answers to several spellings, among
them `compute` (or `gce`, `instances`, `compute_instances`),
`gke` (`clusters`, `kubernetes`, `gke_clusters`), `cloudsql` (`cloud_sql`,
`sql`, `database`, `postgres`, `mysql`), `run` (`cloudrun`, `cloud_run`,
`serverless`), `storage` (`gcs`, `bucket`, `buckets`), `secrets`
(`secret`, `secret manager`, `sm`), and `functions` (`gcf`,
`cloud functions`). The hyphenated forms
(`compute-instances`, `gke-clusters`, `cloud-sql`, `cloud-run`) and the service
labels shown in the sidebar (`GCE instances`, `Cloud SQL`, …) resolve too. A
label needs quoting as a TOML key. nuvem reports and skips a name that matches
no service, and leaves every real service alone.

The column names are the ids in the following table, and that table is what
`[columns]` accepts. A name is usually the header you see in the running
table, lowercased. Some headers abbreviate or rename the column, so read the
id from this list, not from the running table. You cannot sort by a column
marked `*`, from `sort` or with `s`:

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

Three tables are absent from that list: the Cloud Storage object browser, the
BigQuery tables drill-down, and the BigQuery schema view below it. They are
views of a service, not services of their own, and `[columns]` is keyed by
service. Their columns are fixed. Configuration cannot reorder, subset or sort
them.

Every setting is optional. A block can set only `columns` and keep the
default sort, only `sort` and keep the default columns, or both. An empty block
changes nothing. Without a `sort`, a list opens on `name`, ascending.

`columns` must list at least `name`. A table of `STATUS | ZONE` gives no way
to tell which instance a row is, while `Enter`, `t`, `y` and the palette's
lifecycle commands still act on it. A `sort` column must be sortable. If
`columns` is set, the `sort` column must be one of the columns listed.
`sort_direction` is `asc` (the default) or `desc`, and only means something
beside a `sort`. On its own, nuvem rejects it instead of quietly ignoring it.
A list can start with a column that cannot be sorted. The `←`/`→` cursor then
opens on the leftmost sortable column, so `s` always has something to act on.

nuvem validates a block as a unit. Any of these drops the whole block: an
unknown or repeated column name, an empty list, a list without `name`, an
unusable `sort`, a misspelled setting key (`column = [...]`), or a value of the
wrong type. That service goes back to its built-in columns and sort, with a
warning that names the service and the value at fault:

```
columns.compute: unknown column "zzz"; ignoring this service's column settings
```

Every other service's block still applies, and no block is applied in part. An
unusable `sort` takes that block's `columns` down with it, because a list that
sorts by a column it does not show is a worse table than the default one.
Column warnings appear like any other config warning: in the startup and
reload status lines, on stderr and under `nuvem info`, and in the help
overlay's `Config problems` list.

Reload (`:` → **Reload configuration**) reads the blocks again while nuvem
runs, service by service. If a service's new block fails validation, the
service keeps the settings it is already running: the last good table, not the
built-in defaults. Its warning ends
`ignoring this block, keeping the column settings already in use` instead of
the fallback wording shown earlier. A
service with nothing running yet does fall back, and keeps the fresh-load
wording. Deleting a block restores that service's defaults.

A reload never fetches again. The loaded rows, the row selection, the applied
filters, and the load state all survive it. Only the layout changes, and with
it the `←`/`→` cursor and the sort, under the rules that follow.

`sort` is a *default*. It decides what a list opens on, and pressing `s`
overrides it for the session. A reload installs a service's configured sort
again only if that service's own block changed. A sort you set by hand thus
survives a reload caused by something else, such as another service's block or
a `[general]` field. Editing that service's block puts its configured sort
back. If a reload installs a list that no longer shows the sorted column, the
sort falls back to ascending `name`, so the arrow never points off the table.

`[columns.storage]` configures the bucket list. The object browser behind
`Enter` is a detail mode with no column navigation and no sorting. It has
nothing to configure, and it ignores the block.

`w` still toggles a *width preset*. The config decides which columns exist for
you, and `w` decides how many of them fit right now. Narrow (the default) drops
columns until the row fits, and reports the hidden count in the tools line as
`+N cols (w)`. In narrow mode:

- A column whose values are short, such as a status or a region, is shown
  whole or dropped, never cut short.
- A column with long values is cut to no fewer than 16 characters before
  another column is dropped to make room.
- `name` is shown whole for as long as any other column can be dropped
  instead.
- No column header is cut.

Wide keeps every column in the list, and marks the row `▸ more` when it
overruns the terminal.

The two interact in one place. If a service's `columns` list is configured,
narrow mode drops your rightmost column first and works leftwards. The one
exception is `name`, which is never dropped, wherever it sits in the list. A
list that ends in `name` loses its *second*-to-last column first, and `name` is
the column still standing on the narrowest terminal. No column list can leave
you looking at rows you cannot tell apart while `Enter`, `t`, `y` and the
palette's lifecycle commands still act on the selected one. This is the same
reason `columns` must list `name`. Without a configured list, including a
block that sets only `sort`, the built-in priorities of each column decide
which columns narrow mode drops.

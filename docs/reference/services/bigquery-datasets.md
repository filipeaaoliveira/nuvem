---
title: BigQuery datasets
description: Browse datasets, the tables inside them, and each table's column schema.
sidebar:
  order: 12
---

The BigQuery datasets list shows each dataset in the project, with its
location and labels. `Enter` opens a dataset's tables, and a table's column
schema one level below that.

## Datasets

For BigQuery, pressing `r` runs:

```bash
CLOUDSDK_ACTIVE_CONFIG_NAME=<active-config> \
  bq --format=prettyjson --project_id=<project> ls -d -n 500
```

`bq` has no `--configuration` flag, so nuvem passes the active configuration
in the environment of the `bq` process instead of on its command line. If the configuration names no
project, nuvem leaves out `--project_id`, and `bq` uses the configuration's own
project. The table columns are DATASET, LOCATION and LABELS.

## Tables

Press `Enter` on a dataset to drill into the tables it holds:

```bash
CLOUDSDK_ACTIVE_CONFIG_NAME=<active-config> \
  bq --format=prettyjson --project_id=<project> ls -n 500 <dataset>
```

After that listing arrives, nuvem reads each row's stats with a call of its
own:

```bash
CLOUDSDK_ACTIVE_CONFIG_NAME=<active-config> \
  bq --format=prettyjson --project_id=<project> show <project>:<dataset>.<table>
```

That command has no `-n`, because `show` answers about exactly one table and
there is no listing to cap. The table name is fully qualified. An unqualified
`<dataset>.<table>` would resolve against the configuration's own project, not
the project the listing came from.

The drill goes one level further: `Enter` on a table row opens that table's
column schema, described in [Column schema](#column-schema). `Esc` goes up one
level per press, from the schema view to the tables list and from the tables
list to the datasets. `-` and `Backspace` do the same as `Esc`, as they do in
the Cloud Storage object browser. Leaving a level drops what it had listed,
and entering it again asks for it again.

The tables view has six columns. TABLE, TYPE and CREATED come from
`bq ls <dataset>`. The listing also reports each row's labels and its dataset
and project IDs, which have no column (see the labels paragraph further down).

The listing does not carry ROWS, BYTES and LAST MODIFIED at all. nuvem reads
each row's three numbers with that row's own `bq show`, issued after the
listing arrives. The numbers arrive after the rows do, and a row shows dashes
until its own call answers.

TYPE tells a table from a view, a materialized view or an external table. A
type that BigQuery adds later is shown verbatim, not reported as a plain table.
The rows are shown in the order `bq ls` returned them. This list has no sort
and no column selection of its own, and the tools line offers neither. (`s`,
`←` and `→` still act on the dataset list behind the drill, where nothing on
screen moves.)

Opening a dataset therefore issues one `bq show` per listed table, and a
dataset holding 200 tables is 200 calls, including rows that a filter hides.
Only the listing's own `-n 500` cap bounds that: it caps the rows and so caps
the calls. These per-table calls run four at a time, and the rest queue.

Each call is held to a two-minute ceiling. Past it, nuvem kills the `bq`
process and its whole process group, and reports the call as refused. The
ceiling is for a hang, not a latency budget, because a `bq show` answers in
seconds. Without it, one call that never comes back would hold one of the four
slots for good and stall every row queued behind it.

Nothing is cached and a refused call is never retried. Every `r`, and every
re-entry into the dataset, issues all the per-table calls again. When the
repeat arrives, nuvem drops the numbers it already had and does not leave them
on screen as the answer to a refresh.

Starting a new set of per-table calls cancels whatever the last set still had
queued, so those calls never go out. The four already in flight run to
completion, and nuvem discards their answers when they arrive.

Only starting another listing cancels the per-table calls. Leaving the dataset
with `Esc`, entering one of its tables, switching service, or switching
context does not. The calls already queued keep running, four at a time, and
nuvem discards every answer on arrival.

On a wide dataset, that is a few hundred `bq` processes still to start for a
table nobody is looking at. It costs machine time and API quota but does not
change the screen, and the next dataset you enter ends it.

A dash in ROWS, BYTES or LAST MODIFIED means nobody answered: that row's `bq
show` is still out, or it was refused. It never means zero. A table that really
holds nothing shows `0` and `0 B`.

The listing and the per-row `show` need separate permissions.
`bigquery.tables.list` lists the tables, and `bigquery.tables.get` reads each
one. Three dashes on every row is what a denied `bigquery.tables.get` looks
like. TABLE, TYPE and CREATED stay intact.

A dash has one more cause. If you press `Enter` on a table before the
per-table calls have finished, nuvem abandons the rest of them. When you come
back with `Esc`, the tables list is exactly as it was, and the rows are not
listed again. The `bq show` answers still in flight belong to the level you
left, so nuvem drops every one of them on arrival.

The rows already answered keep their numbers. The other rows show `-` in ROWS,
BYTES and LAST MODIFIED indefinitely, on a list that looks settled. Press `r`
to fill them: it issues the listing and all the per-table calls again.
Entering a table cancels nothing, so the abandoned calls keep running four at
a time, and spend API quota on answers that nuvem discards.

Labels have no column here, unlike in the dataset table. One real dataset's
`dataplex-*` set flattens past 300 characters and would compete with the table
name for the width of the pane. Labels are still searchable. `/` narrows the
loaded rows on the table ID, its dataset, its type as the TYPE cell spells it
(`/materialized` matches `Materialized view`) and its `key=value` labels.

`/` does not match what ROWS, BYTES and LAST MODIFIED show. Those cells are
formatted numbers, not words, and matching them would also match the dash, so
`/-` would select exactly the rows nothing is known about. `bq ls` takes no
filter expression, in either view, so a filter is applied client-side and
issues no call.

The preview beside the list describes the table the cursor is on: its type,
its creation time, its dataset and its labels. The pane title already names
the dataset.

Inside the tables drill, `r` issues the listing again for the dataset you are
in. The failure state's "Press `r` to retry." refers to this. The dataset list
behind the drill is refreshed in the same step. An empty dataset is reported as
empty, not as a failure.

## Column schema

Press `Enter` on a table to open its column schema, one level further down:

```bash
CLOUDSDK_ACTIVE_CONFIG_NAME=<active-config> \
  bq --format=prettyjson --project_id=<project> show <project>:<dataset>.<table>
```

That is the same command the per-row stats use, issued a second time for the
one table you opened. Nothing caches it, in either direction. The per-table
calls throw their schemas away instead of keeping every listed row's schema,
most of which you never open. This call throws away the three numbers it
carries, because the row behind it already has them. So one `Enter` is one
`bq show`.

Inside the schema view, `r` issues it again for the table on screen and does
not list the tables behind it again. Entering the same table again asks again.

The schema view has four columns: FIELD, TYPE, MODE and DESCRIPTION. Nested
records are flattened to dotted paths, so `user.address.city` is a row. The
record itself stays as a row of its own above its children, so a `RECORD` row
is the parent of the rows that follow it. The rows have no indentation. The
dotted path says where a field sits.

The rows are shown in schema order, which keeps a record's children
together. This list has no sort and no column selection, and the config file
cannot reorder or subset its columns. DESCRIPTION shows `-` for a field the
schema documents with nothing, and a blank cell for a field documented with an
empty string.

`/` narrows the loaded columns on the path, the type, the mode, and the
descriptions that exist. The path is matched whole, so `/event_params` selects
a record together with everything nested inside it. The dash is the one thing
on screen that `/` does not match, for the same reason as on the tables level.
It is not a word the row holds, and matching it would make `/-` select exactly
the fields nobody documented.

`bq show` takes no filter expression either, so this narrowing is client-side
and issues no call. The preview beside the list describes the column the
cursor is on, and shows its full description where the column is too narrow
for it.

A refused `bq show` fails this level and does not put a dash in a cell. The call
is the whole content of the view, so the view keeps the table in its header,
says what went wrong, and offers `r`. A table whose schema is empty is reported
as empty, not as a failure, the same as in the tables listing.

`Enter` on a column row opens nothing, because a column is the bottom of this
hierarchy. The frame footer hints `Enter` only where it opens something, and
the help overlay (`?`) leaves it out at the bottom level. Both decide this by
whether the level on screen has another level below it.

BigQuery browsing is read-only throughout: no query is ever run, and nothing is
created, modified or deleted.

## Demo mode

In `--demo` mode, the same recorded tables answer every dataset, and the same
recorded table resource answers every `show`. So both demo rows paint one
table's row count and byte size, and both open that same table's columns. Demo
mode holds one recorded response per command, not one per argument. The two callers of
`show` send an identical command line, so one recorded response has to answer
both. That response is a nested schema, so the demo schema view shows the
dotted paths, the `REPEATED` mode and the `-` that a real one does. The numbers
it paints come from that recorded response, not from the listed row. Also,
that response was transcribed rather than freshly captured, so
its counters carry the shape the API sends and not verified values.

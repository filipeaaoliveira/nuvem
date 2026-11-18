---
title: BigQuery datasets
description: Browse datasets, the tables inside them, and each table's column schema.
sidebar:
  order: 12
---

## Datasets

For BigQuery, pressing `r` runs:

```bash
CLOUDSDK_ACTIVE_CONFIG_NAME=<active-config> \
  bq --format=prettyjson --project_id=<project> ls -d -n 500
```

`bq` has no `--configuration` flag, so the active configuration is carried in
the child environment instead of on the argv; `--project_id` is omitted
entirely when the configuration names no project, which lets `bq` fall back to
the configuration's own. The table columns are DATASET, LOCATION and LABELS.

## Tables

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

## Column schema

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

## Demo mode

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

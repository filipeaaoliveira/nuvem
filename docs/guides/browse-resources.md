---
title: Browse resources
description: Open a service, move between the list and the detail pane, sort, and go back through history.
sidebar:
  order: 2
---

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
are configurable per service — see
[Columns and sort](../configuration/columns-and-sort.md).

In a resource list, select an item and press `Enter` to load a read-only detail
view. Use `[` and `]` to move across the service's detail tabs. `Esc` goes back
to the prior view — from a resource detail view that returns to the resource
list; from a resource list it steps back through navigation history (for
example, to the previous service or workspace) when there is history to go
back to. In Cloud Storage, `Esc` from the object browser or a bucket's
metadata returns to the bucket list, and from an object's detail to the object
browser; in BigQuery it climbs from a table's schema to the tables, and from
the tables to the datasets. While a resource detail view is open, `q` also
returns to the resource list. `Alt-Left` and `Alt-Right` move back and forward through
navigation history from anywhere in the Resources workspace; the footer shows
these hints only when there is somewhere to go. Where `Esc` and `Alt-Left` do
the same thing, the footer and help list them as one entry, `Esc/Alt-←`. Navigation history is reset
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

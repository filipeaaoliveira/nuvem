---
title: Browse resources
description: Open a service, move between the list and the detail pane, sort, and go back through history.
sidebar:
  order: 2
---

Press `g` to open the Resources workspace. A sidebar on the left lists every
service, and the rest of the screen shows the open service.

## Pick a service

- Press `v` to show or hide the sidebar.
- When the sidebar has focus, press `/` to search the list of services.
- Press `j`/`k` or the arrows to move through the matching services.
- Press `Enter` to open the selected service.
- Press `Tab` to move focus between the sidebar and the resource content. This
  works while a detail view is open, too.

A service that is unavailable stays in the sidebar and shows its availability
status.

## Sort the list and change its columns

In the Resources list, the left and right arrows move the selected column.
Press `s` to sort by that column, and `s` again to reverse the direction. The
column header shows the sort direction with an arrow. Press `w` to switch
between narrow columns, which drop columns to fit the terminal, and wide
columns, which keep every column.

You can set, per service, which columns show, in which order, and which column
the list opens sorted by. See [Columns and sort](../configuration/columns-and-sort.md).

## Open a detail and go back

In a resource list, select an item and press `Enter` to load a read-only
detail view. Use `[` and `]` to move across the service's detail tabs.

`Esc` goes back to the prior view:

- From a resource detail view, it returns to the resource list.
- From a resource list, it steps back through navigation history, for example
  to the previous service or workspace, when there is history to go back to.
- In Cloud Storage, it returns from the object browser or a bucket's metadata
  to the bucket list, and from an object's detail to the object browser.
- In BigQuery, it climbs from a table's schema to the tables, and from the
  tables to the datasets.

While a resource detail view is open, `q` also returns to the resource list.
`Alt-Left` and `Alt-Right` move back and forward through navigation history
from anywhere in the Resources workspace. The footer shows these hints only
when there is somewhere to go. Where `Esc` and `Alt-Left` do the same thing,
the footer and help list them as one entry, `Esc/Alt-←`. nuvem resets
navigation history whenever the context changes.

From Home and from a list, `q` quits nuvem. `Ctrl-C` quits from any view.

If a project is empty, the rows are stale, or a refresh fails, the Resources
workspace says so instead of leaving the list blank.

## Wide terminals

On a wide terminal, with a body at least 100 columns wide, the Resources
workspace splits in two. The list stays on the left. The right pane shows a
summary preview of the selected row in list view, or the full tabbed detail
view when a detail is open. The split uses only resource data that nuvem has
already loaded, and it sends no extra request to Google Cloud. Narrower terminals,
including the `60x12` minimum, keep the full-screen list or detail view.

---
title: Related-resource jumps
description: Jump from a resource to the resources it is attached to.
sidebar:
  order: 10
---

Press `J` (shifted) in a resource list or a resource detail pane to open the
related-resource jump picker. It is a small dialog that lists the collections
you can reach from the resource on screen. Each row names the destination tab
and the resource, such as `Network of web-01` or `Replicas of orders-db`, so a
row never reads as a generic "jump".

Move with `j`/`k` or the arrows. The cursor stops at both ends and does not
wrap. Press `Enter` to jump, or `Esc` to cancel. The picker captures its rows
when it opens, so a background reload of the list cannot change what a row
means.

Jump rules come from your configuration, and their number has no limit. If the
picker has more rows than the terminal can show, the rows scroll inside the
picker to keep the selected row visible. Every configured rule stays reachable
with `j`/`k`.

## Default jumps

The table lists the jumps that nuvem offers by default. Of the rows that offer
jumps, the first three land on a detail tab. The last two land on the VPC
networks table, with the cursor on the network that the subnet or the rule is
attached to:

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

A `[[jumps]]` array in the config file replaces this table with rules of
your own. Your rules can add jumps from the services that offer none by
default, and jumps that cross from one service to another's detail tab. See
[Jump rules](../configuration/jump-rules.md) for the keys and the template
vocabulary.

## What a jump loads

nuvem fetches nothing new to build the picker. The destinations are views that
nuvem already renders. A jump costs at most the detail load that opening that
resource normally costs, and a jump onto a list costs nothing.

A jump opens a pane, or moves a list cursor, only for a resource already in
the destination service's loaded list. It matches on both name and location.
If there is no such row, the jump reports it on the status line. If that list
has never been loaded, the jump starts its fetch on the way past, so the
report tells you to try again once the rows arrive.

Only a never-loaded list gets that fetch. A list whose load is already in
flight, or whose last load failed *in this configuration* and is waiting on
the retry backoff, is left alone. nuvem asks each of these questions of the
configuration you are in now. So switching configuration and jumping again
fetches, instead of reusing the old configuration's result.

Keep this in mind before you write a rule that crosses from one service to
another. See [Jump rules](../configuration/jump-rules.md).

## When there is nothing to jump to

A selection with no applicable jumps never opens an empty picker. `J` reports
on the status line instead, and says which reason applies:

- No rule offers anything from this service:
  `No related-resource jumps configured for GKE clusters`,
  `No related-resource jumps configured for Cloud Storage buckets`, or
  `No related-resource jumps from a Cloud Run revision`.
- Nothing is selected to jump from:
  `No GCE instance selected to jump from`.
- A rule exists but cannot be resolved for this row:
  `Jump "compute:network" needs external_ip, which this GCE instance has no value for`.

Each reason is fixed in a different place: in the config file, by selecting a
row, or in the row itself.

## Where a jump lands

Like `L`, a jump follows the view, not the raw list cursor. With a detail pane
open, the pane's resource is the origin. The nested Cloud Run revision view is
neither an origin nor a destination.

If the pane already shows the destination resource, the jump changes the tab,
as `[`/`]` does. Otherwise it opens the detail. A jump that moves is recorded
in navigation history, so `Alt-←` returns to where you jumped from and
`Alt-→` returns to the jump.

If you pick the tab you are already on, the picker closes and nothing else
happens. No history is recorded, and the pane's scroll position, field
cursor, and revision cursor stay where you left them.

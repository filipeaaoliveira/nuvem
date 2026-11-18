---
title: Related-resource jumps
description: Jump from a resource to the resources it is attached to.
sidebar:
  order: 10
---

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
tab. See [Jump rules](../configuration/jump-rules.md) for the keys and the
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
[Jump rules](../configuration/jump-rules.md).

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

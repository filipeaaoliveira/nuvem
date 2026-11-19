---
title: VPC networks
description: List VPC networks with their subnet mode, routing mode and subnet count.
sidebar:
  order: 15
---

For VPC networks, `r` reloads the list. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> --project <project> \
  compute networks list --format=json --limit=500
```

This is one flat, project-wide call. A network is a global resource, so there
is no region to iterate and no second call to make.

The columns are NAME, MODE, ROUTING and SUBNETS. MODE is `AUTO`, `CUSTOM` or
`LEGACY`. ROUTING is `REGIONAL` or `GLOBAL`. SUBNETS is how many subnets the
network has.

nuvem derives MODE and ROUTING from the API's own fields,
`autoCreateSubnetworks` and `routingConfig.routingMode`. It never reads the
`x_gcloud_subnet_mode` and `x_gcloud_bgp_routing_mode` keys that gcloud adds to
the same JSON. Those two keys carry exactly the strings this table paints, but
they are gcloud's presentation layer and not API fields. The `x_` prefix marks
them as gcloud's own, and gcloud makes no promise to keep them.

`LEGACY` is a network that omits `autoCreateSubnetworks` altogether. It stays
distinct from `CUSTOM` because the two differ in what you do next: a custom
network has subnets to go and look at, and a legacy network cannot have any. A
network whose JSON carries no `routingConfig` leaves ROUTING blank. It does not
read `REGIONAL`. Regional is the API's default for a network created without
one, and a default is a claim about creation, not about what this listing
reported.

SUBNETS is a count, not a list. The `subnetworks` field is an array of full
self-link URLs, 43 of them on the default network of a fresh project, which
does not fit in a table cell. The subnets have a table of their own. A count of
`0` is painted as `0` and never as a dash. The API listed the network's subnets
and found none, and a custom-mode network with no subnets is a finding, not a
missing value.

A network's description has no column, because it is unbounded free text that
would push the four columns off a narrow terminal. It is searchable, and the
preview pane shows it in full. `/` narrows wholly client-side here and pushes
nothing to the server. Three of the four columns are derived values that appear
verbatim in no raw field, so a pushed clause would drop rows you can read on
screen. Label filtering (`-l k=v`) reports unsupported, because the
`compute/v1` networks resource has no labels field at all.

Because the narrowing is client-side, applying a filter makes no call at all.
It narrows the rows already in memory again. This also protects the table. A
list refresh that fails clears the table and its context, so a table refreshed
on every filter keystroke would go blank on a short auth or quota error while
you type. `r` still fetches, because `r` is an explicit request for fresh rows.

The networks table is list-only and read-only. `Enter` does nothing on a row,
there is no detail pane, and nuvem never creates, deletes or modifies a
network. `L` is refused, not scoped to the selected network. Cloud Logging
attributes VPC flow logs and firewall hit logs to the *subnetwork*, so a
network-scoped query would return zero rows when you expect many.

There is no `gateway` column: `gatewayAddress` is a subnet field, and a
network carries no address of its own.

The fixture behind demo mode and the tests, `tests/fixtures/vpc_networks.json`,
is a live capture taken on 2026-08-25 with the project id replaced by
`nuvem-fixture-project`. Its field set and key spellings are the API's own.
The `CUSTOM`, `GLOBAL` and `LEGACY` cases are not in it, because the account it
was captured from can reach no such network. Those arms are pinned against
rows constructed from Google's published `compute/v1/networks` reference, and
the tests that use them say so.

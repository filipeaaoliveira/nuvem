---
title: VPC networks
description: List VPC networks with their subnet mode, routing mode and subnet count.
sidebar:
  order: 15
---

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

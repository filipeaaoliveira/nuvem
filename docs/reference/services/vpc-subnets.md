---
title: VPC subnets
description: List every subnet in every region in one table.
sidebar:
  order: 16
---

For VPC subnets, `r` reloads the list. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> --project <project> \
  compute networks subnets list --format=json --limit=500
```

A subnet is a *regional* resource, but this is still one flat, project-wide
call. `networks subnets list` returns every region at once, so there is no
per-region fan-out and no `--regions` flag. Every region's subnets are in one
table, and REGION is a column you sort by, not a scope you enter.

The columns are NAME, REGION, NETWORK, RANGE, GATEWAY and PGA.

A subnet's name does not identify it. An auto-mode network gets one subnet per
region and calls every one of them `default`, so a fresh project's table is 43
rows that share a single name and differ only in REGION. For this reason REGION
sits immediately behind NAME in the column order and is the last column a
narrow terminal drops. Without it, the rows are indistinguishable. It is also
why a jump rule from this service can use `{location}` as well as `{name}`, and
why the preview pane heads itself `default (us-central1)` and not `default`.

REGION and NETWORK are painted as ids, not as the self-link URLs the API sends.
`region` and `network` both arrive as
`https://www.googleapis.com/compute/v1/projects/<p>/regions/us-central1` and
similar. The parser takes the last path segment once, so no two cells can
disagree about what a network is called.

PGA is `PRIVATE GOOGLE ACCESS`. The full phrase is 21 characters for a
3-character cell, and the console and the `gcloud` flag both use the
abbreviation. It reads `On` or `Off`, never a dash. The API documents an absent
flag as off, so `Off` is an answer and not a missing value.

`L` works here, and this is the only VPC table where it works. Cloud Logging
attributes both VPC flow logs and firewall hit logs to the subnetwork
(`resource.type="gce_subnetwork"`), never to the network and never to the
firewall rule. That is why the networks table refuses `L`: a query there could
only return zero rows. To see what traffic a network carries, look at its
subnets.

Narrowing stays wholly client-side, for the same reason as on the networks
table. REGION and NETWORK are the tails of URLs and PGA renders a bool as a
word, so none of the three is a verbatim substring of a raw field. Label
filtering reports unsupported, because the `compute/v1` subnetworks resource has
no labels field. As a result, applying a filter makes no call here either. A
refresh that fails clears the table, so a needless refresh would turn a short
error into a blank screen while you type.

The subnets table is list-only and read-only. `Enter` does nothing on a row,
there is no detail pane, and nuvem never creates, deletes or modifies a subnet.

`tests/fixtures/vpc_subnets.json` is a live capture taken on 2026-08-25 with
the project id replaced by `nuvem-fixture-project`, trimmed from the project's
43 rows to three. The trim removes rows and changes no field of the rows it
keeps. Three rows are enough to show the one-name-many-regions shape that the
table is built around.

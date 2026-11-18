---
title: VPC subnets
description: List every subnet in every region in one table.
sidebar:
  order: 16
---

For VPC subnets, pressing `r` runs:

```bash
gcloud --configuration <active-config> --project <project> \
  compute networks subnets list --format=json --limit=500
```

One flat, project-wide call despite a subnet being a *regional* resource:
`networks subnets list` returns every region at once, so there is no per-region
fan-out and no `--regions` flag. Every region's subnets are in one table, and
REGION is a column you sort by rather than a scope you enter.

The columns are NAME, REGION, NETWORK, RANGE, GATEWAY and PGA.

**A subnet's name does not identify it.** An auto-mode network gets one subnet
per region and calls every one of them `default`, so a fresh project's table is
43 rows sharing a single name, told apart only by REGION. That is why REGION
sits immediately behind NAME in the column order and is the last column a
narrow terminal drops: without it the rows are indistinguishable. It is also
why a jump rule from this service can use `{location}` as well as `{name}`, and
why the preview pane heads itself `default (us-central1)` rather than
`default`.

REGION and NETWORK are painted as **ids**, not as the self-link URLs the API
sends. `region` and `network` both arrive as
`https://www.googleapis.com/compute/v1/projects/<p>/regions/us-central1` and
similar; the last path segment is taken once, in the parser, so no two cells
can disagree about what a network is called.

PGA is `PRIVATE GOOGLE ACCESS`, abbreviated because the full phrase is 21
characters carrying a 3-character cell — and it is the abbreviation the console
and the `gcloud` flag both use. It reads `On` or `Off`, never a dash: the API
documents an absent flag as off, so `Off` is an answer rather than a missing
value.

`L` works here, and it is the **only** VPC table it works on. Cloud Logging
attributes both VPC flow logs and firewall hit logs to the subnetwork
(`resource.type="gce_subnetwork"`), never to the network and never to the
firewall rule — which is exactly why the networks table refuses `L` rather than
offering a query that could only return zero rows. An operator asking what
traffic a network is carrying is asking about its subnets.

Narrowing stays wholly client-side, for the reason the networks table's does:
REGION and NETWORK are the tails of URLs and PGA renders a bool as a word, so
none of the three is a verbatim substring of a raw field. Label filtering
reports unsupported, because the `compute/v1` subnetworks resource has no
labels field. Applying a filter therefore runs no gcloud command here either,
and for the same second reason: a refresh that fails clears the table, so a
redundant one would turn a transient error into a blank screen the operator
caused by typing.

The subnets table is list-only and read-only: `Enter` does nothing on a row,
there is no detail pane, and nuvem never creates, deletes or modifies a subnet.

`tests/fixtures/vpc_subnets.json` is a live capture taken on 2026-08-25 with
the project id replaced by `nuvem-fixture-project`, trimmed from the project's
43 rows to three. The trim removes rows and changes no field of the rows it
keeps; three is what it takes to show the one-name-many-regions shape the table
is built around.

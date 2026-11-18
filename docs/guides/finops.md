---
title: FinOps
description: See cost recommendations and budgets for the active project.
sidebar:
  order: 11
---

The FinOps workspace collects what Google Cloud already recommends you stop
paying for, and the budgets set on the project's billing account. It is
read-only: nuvem never applies a recommendation and never changes a budget.

## Open it

1. Open Resources (`g`) and list **GCE instances**, then **Persistent disks**.
   FinOps asks about the zones and regions those lists contain, so it needs
   them first.
2. Press `Ctrl-f`, or choose **FinOps** in the command palette (aliases
   `finops`, `cost`, `costs`, `savings`, `budget`, `spend`).
3. Press `r` to ask again.

Until a location is known, the total reads `Potential savings: not probed` and
each card says `Open Resources and list instances or disks first — FinOps
queries the locations they are in.` That is not a zero: nuvem never shows
savings it did not ask about.

## What it shows

The first line is the total: `Potential savings: <amount>/mo`, one amount per
currency.

Below it, one card per recommender:

| Card | Recommender | Asked per |
| --- | --- | --- |
| Idle VMs | `google.compute.instance.IdleResourceRecommender` | zone |
| Machine-type right-sizing | `google.compute.instance.MachineTypeRecommender` | zone |
| Idle disks | `google.compute.disk.IdleResourceRecommender` | zone |
| Unused addresses | `google.compute.address.IdleResourceRecommender` | region |

Each card lists up to five recommendations. It also says which locations
answered and which refused, for example `covers europe-west1-b; no answer from
us-central1-a`, because one zone can answer while another is denied.

A card that could not ask names the reason in the words Home uses (for example
`permission denied` or `API disabled`) and what to do about it. See
[Troubleshooting](../troubleshooting/common-problems.md#error-categories).

The budgets block lists each budget's name, amount and alert thresholds. A
budget that tracks last period's spend reads `matches last period`. A budget
that covers the whole billing account says so. Budgets show no spend, because
the Budgets API returns none. When the project has no billing account, the
block says so instead of showing an empty list.

The last block is the same resource inventory Home shows.

## The commands it runs

Recommendations are read through the Recommender client library, one call per
recommender and location. Budgets go through gcloud, because there is no Rust
client for the Budgets API:

```bash
gcloud --configuration <active-config> billing projects describe <project> --format=json --quiet
gcloud --configuration <active-config> billing budgets list --billing-account=<account> --format=json --quiet
```

In demo mode the same calls are answered from fixtures.

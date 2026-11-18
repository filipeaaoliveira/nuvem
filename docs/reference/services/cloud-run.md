---
title: Cloud Run
description: Browse Cloud Run services, their traffic split and their revisions.
sidebar:
  order: 5
---

For Cloud Run services, pressing `r` runs:

```bash
gcloud --configuration <active-config> [--project <project>] run services list --platform=managed --format=json --limit=500
```

This lists services across all regions in one call. Select a service and
press `Enter` to load a read-only detail view, scoped to that service's
region:

```bash
gcloud --configuration <active-config> [--project <project>] run services describe <name> --platform=managed --region <region> --format=json
```

Use `[` and `]` to move across Summary, Traffic, Revisions, Configuration,
Conditions, and JSON tabs. On the Revisions tab, press `Enter` to drill into a
revision's own read-only detail (Summary, Conditions, and JSON tabs), fetched
with the same region scoping. `Esc`, `q`, or history-Back returns to the
service's Revisions tab without losing the prior selection. With a service
selected in the list, the command palette's `Open logs for selected Cloud Run
service` entry opens resource-scoped logs in the Logging workspace; `L` does
the same from the list and from the service's detail pane, and from inside a
revision it scopes to that revision instead (the palette offers `Open logs for
selected Cloud Run revision` there). `J` on a service offers its Revisions tab
unless you configure otherwise — see
[Related-resource jumps](../../guides/related-resource-jumps.md). Cloud Run mutation
(deploy, traffic management, delete, rollback) is not supported.

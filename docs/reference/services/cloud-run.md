---
title: Cloud Run
description: Browse Cloud Run services, their traffic split and their revisions.
sidebar:
  order: 5
---

The Cloud Run list shows the project's Cloud Run services across all regions.
`Enter` opens a service's detail, including its traffic split and revisions.

For Cloud Run services, `r` reloads the list. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> [--project <project>] run services list --platform=managed --format=json --limit=500
```

This lists services across all regions in one call. To open a read-only
detail view, select a service and press `Enter`. The detail read is scoped to
that service's region:

```bash
gcloud --configuration <active-config> [--project <project>] run services describe <name> --platform=managed --region <region> --format=json
```

Use `[` and `]` to move across Summary, Traffic, Revisions, Configuration,
Conditions, and JSON tabs. On the Revisions tab, press `Enter` to open a
revision's own read-only detail (Summary, Conditions, and JSON tabs), read with
the same region scoping. `Esc`, `q`, or `Alt-←` returns to the service's
Revisions tab and keeps the previous selection.

With a service selected in the list, the command palette's `Open logs for
selected Cloud Run service` entry opens resource-scoped logs in the Logging
workspace. `L` does the same from the list and from the service's detail pane.
Inside a revision, `L` scopes to that revision instead, and the palette offers
`Open logs for selected Cloud Run revision` there.

`J` on a service offers its Revisions tab unless you configure otherwise. See
[Related-resource jumps](../../guides/related-resource-jumps.md). nuvem does
not change Cloud Run resources: deploy, traffic management, delete and rollback
are not supported.

---
title: Secret Manager
description: Browse secret metadata and versions. Payloads are never read.
sidebar:
  order: 7
---

For Secret Manager, pressing `r` runs:

```bash
gcloud --configuration <active-config> secrets list --format=json --limit=500
```

The secret list shows name, create time, replication policy, and labels. There
is no version-count column: the listed secret resource does not carry one, so
filling a column would cost one extra call per row on every list load, and one
list load is one `gcloud` call on every service here. The count lives on the
detail pane instead, fetched for the one secret you open.

Select a secret and press `Enter` to load a read-only detail view across
Summary, Versions, Labels, and JSON tabs (`[` and `]` to move between them).
Opening it runs two commands:

```bash
gcloud --configuration <active-config> secrets describe <name> --format=json
gcloud --configuration <active-config> secrets versions list <name> --format=json
```

Versions list version name, state (enabled, disabled, destroyed), and create
time. Listing versions needs an IAM permission that listing secrets does not,
so a failure there is reported as such rather than shown as a secret with no
versions.

Secret Manager browsing is metadata only, and that is a structural property
rather than a display rule: nuvem never issues the call that returns a
version's payload, so there is no secret material anywhere in the app to
redact. Creating, editing, disabling, or destroying secrets and versions is out
of scope too — the service is read-only.

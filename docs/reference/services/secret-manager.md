---
title: Secret Manager
description: Browse secret metadata and versions. Payloads are never read.
sidebar:
  order: 7
---

The Secret Manager list shows the project's secrets and their metadata, never
their values.

For Secret Manager, `r` reloads the list. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> secrets list --format=json --limit=500
```

The secret list shows name, create time, replication policy, and labels. There
is no version-count column. The listed secret resource does not carry a count,
so a column would cost one extra call per row on every list load. The count is
on the detail pane instead, read for the one secret you open.

To open a read-only detail view, select a secret and press `Enter`. The view
has Summary, Versions, Labels, and JSON tabs, and `[` and `]` move between
them. Opening it makes two calls. The equivalent `gcloud` commands are:

```bash
gcloud --configuration <active-config> secrets describe <name> --format=json
gcloud --configuration <active-config> secrets versions list <name> --format=json
```

Versions list version name, state (enabled, disabled, destroyed), and create
time. Listing versions needs an IAM permission that listing secrets does not.
If that call fails, nuvem reports the failure. It does not show a secret with
no versions.

Secret Manager browsing is metadata only. nuvem never makes the call that
returns a version's payload, so no secret material exists anywhere in nuvem,
and there is nothing to redact. The service is read-only: creating, editing,
disabling, or destroying secrets and versions is out of scope.

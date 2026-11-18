---
title: Home overview
description: "What the Home workspace shows: context health, inventory, recent operations and degraded causes."
sidebar:
  order: 1
---

The Home workspace (`h`) is a read-only dashboard over already-loaded state;
it never triggers a resource load on its own. It shows:

- **Context health**: whether `gcloud` is available and authenticated, the
  active account, and any classified problem from the last diagnostics probe.
  The probe covers the `gcloud` CLI only; a native read that fails on its own
  (an expired token, a disabled API, no route to `googleapis.com`) surfaces on
  the service row rather than here.
- **Inventory by status**: one line per service (GCE instances, GKE clusters,
  Cloud SQL, Cloud Run, Cloud Storage, Secret Manager, Cloud Functions,
  Persistent disks, Pub/Sub topics, Pub/Sub subscriptions, BigQuery datasets,
  Service accounts, IAM policy, VPC networks, VPC subnets, Firewall rules) — every service nuvem can browse — with status counts for the services that have a
  lifecycle status and a freshness marker distinguishing not-loaded (never
  fetched yet), healthy-empty (fetched, zero rows), loaded (fetched, has rows),
  and stale (fetched but out of date for the active context). A short
  terminal that cannot show every line ends the block with `+N more` rather than
  leaving the services it skipped unmentioned.
- **Recent operations**: the most recent in-app operations (currently GCE and
  Cloud SQL mutations, and plugin runs) with their outcome (pending, succeeded,
  failed, cancelled, unknown) and detail. `cancelled` means the command was
  never issued; `unknown` means it was issued but its result was never seen,
  so the resource's real state has to be re-checked with a refresh.
- **Degraded causes**: when context health or a service line reports a
  problem, it shows a sanitized, actionable headline — a classified reason
  plus a short remediation hint (for example `permission denied — Check IAM
  roles for this account and project`) — instead of raw gcloud output. Press
  `Y` in any normal view to copy the sanitized technical detail of the last
  diagnostic to the clipboard.

Press `r` on Home to refresh the overview diagnostics (the gcloud
availability/auth probe behind the context-health card). It does not reload
resource inventory; switch to the Resources workspace and press `r` there to
refresh a service's rows.

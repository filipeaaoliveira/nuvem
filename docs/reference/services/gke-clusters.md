---
title: GKE clusters
description: Browse GKE clusters, read their detail, and open one in k9s.
sidebar:
  order: 3
---

For GKE clusters, pressing `r` runs:

```bash
gcloud --configuration <active-config> container clusters list --format=json --limit=500
```

In the GKE clusters service, select a cluster and press `Enter` to load a
read-only detail view. The detail command is:

```bash
gcloud --configuration <active-config> container clusters describe <name> --location <location> --format=json
```

Use `[` and `]` to move across Summary, Networking, Node Pools, Addons, and
JSON tabs. GKE mutation is not included: nuvem does not create, scale or
delete clusters.

## Open a cluster in k9s

Select a GKE cluster and open the palette with `:` to run `Open selected GKE
cluster in k9s` (aliases `k9s`, `open`). The action appears only when the active
configuration provides a project and the cluster has a location. nuvem first
checks (best-effort, via `kubectl config get-contexts`) whether the deterministic
kubeconfig context `gke_<project>_<location>_<cluster>` already exists, then shows
a confirmation modal that spells out whether it will reuse that context or run
`get-credentials` first, the exact argv for both commands, and a note that
`get-credentials` modifies your local kubeconfig:

```bash
gcloud --configuration <active-config> container clusters get-credentials <name> --location <location> --project <project>
k9s --context gke_<project>_<location>_<cluster>
```

Press `Enter` to launch or `Esc` to cancel before either command runs. When a
credentials refresh is required it runs first; if it fails, k9s is not launched
and the error is surfaced. The launch itself reuses the shared interactive-process
runtime, so nuvem suspends the TUI while k9s owns the terminal and restores it
(preserving your list selection and detail state) when k9s exits. If `k9s` is not
on your `PATH` you get a diagnostic and cluster browsing stays usable. Commands
are invoked without a shell, so cluster names and locations are never interpolated
into a shell string. Override the binaries with `--k9s-binary` and
`--kubectl-binary` when they live outside `PATH`.

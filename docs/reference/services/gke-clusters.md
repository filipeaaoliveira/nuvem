---
title: GKE clusters
description: Browse GKE clusters, read their detail, and open one in k9s.
sidebar:
  order: 3
---

The GKE clusters list shows the project's GKE clusters. From it you can open
a cluster's detail, or open the cluster in k9s, the terminal UI for
Kubernetes.

For GKE clusters, `r` reloads the list. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> container clusters list --format=json --limit=500
```

To open a read-only detail view, select a cluster and press `Enter`. The
equivalent detail command is:

```bash
gcloud --configuration <active-config> container clusters describe <name> --location <location> --format=json
```

Use `[` and `]` to move across Summary, Networking, Node Pools, Addons, and
JSON tabs. GKE mutation is not included: nuvem does not create, scale or
delete clusters.

## Open a cluster in k9s

Select a GKE cluster, open the palette with `:`, and run `Open selected GKE
cluster in k9s` (aliases `k9s`, `open`). The action appears only when the active
configuration provides a project and the cluster has a location.

nuvem first runs `kubectl config get-contexts` to find out whether your
kubeconfig already has the kube context `gke_<project>_<location>_<cluster>`.
This check is best-effort. nuvem then shows a confirmation with three things:
whether it will reuse that context or run `get-credentials` first, the exact
command line for both commands, and a note that `get-credentials` modifies
your local kubeconfig:

```bash
gcloud --configuration <active-config> container clusters get-credentials <name> --location <location> --project <project>
k9s --context gke_<project>_<location>_<cluster>
```

Press `Enter` to launch or `Esc` to cancel before either command runs. If a
credentials refresh is required, it runs first. If it fails, nuvem shows the
error and does not launch k9s.

nuvem suspends the interface while k9s owns the terminal. When k9s exits,
nuvem restores the interface with your list selection and detail state. If `k9s` is not on
your `PATH`, you get a diagnostic and cluster browsing stays usable. nuvem runs
both commands without a shell, so cluster names and locations never become part
of a shell string. If the binaries live outside `PATH`, set them with
`--k9s-binary` and `--kubectl-binary`.

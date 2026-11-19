---
title: Read-only mode
description: Refuse every command that changes a cloud resource.
sidebar:
  order: 13
---

Read-only mode refuses every command that changes a cloud resource. Start
nuvem with it:

```bash
nuvem --read-only
```

## What it refuses

The mode refuses any command that changes cloud state, or that hands you an
interactive session that can. It refuses:

- Starting, stopping, resetting, suspending, or resuming a GCE instance
- Starting, stopping, restarting, or failing over a Cloud SQL instance
- SSH to an instance
- Opening a GKE cluster in k9s. k9s is a full read-write Kubernetes UI, and
  the launcher runs `get-credentials` when your kubeconfig has no kube context
  for the cluster yet.
- Every plugin that nuvem cannot prove only reads

A plugin's command line comes from your `plugins.toml`, not from nuvem. So the
mode runs a plugin only when its declared `args` open with a gcloud command
that nuvem recognizes as read-only. It refuses the rest, including a command
that reads but that nuvem does not happen to name. If you choose a refused
plugin, nuvem reports the refusal instead of opening its prompt or its
confirmation. See [Plugins](../plugins/overview.md).

nuvem checks the mode again at each step where a change can still start:

- When you submit a command from the palette
- When you confirm a dialog that was already open
- For k9s, at the interactive launch itself. Its confirmation only starts
  `get-credentials`, so the mode is checked again when those credentials come
  back.

If a reload turns the mode on while a k9s launch is in flight, the launch
never reaches the cluster.

## What it still allows

Read-only mode protects cloud resources. nuvem still changes some local
state. It allows three actions on purpose, because none of them touches a
cloud resource:

- Activating a gcloud configuration, which changes local gcloud state
- Opening a tunnel, which is a process on this machine
- Copying a value to the clipboard

Read commands, such as listing, detail views and logs, work as usual. Only
these three changes are exempt. Running on your machine does not exempt a
command: any command that changes cloud state, or opens a session that can,
is blocked.

Blocked commands stay in the command palette, and you can still select them.
They show dimmed, with a trailing `— read-only`. If you submit one, nuvem
tells you why instead of doing nothing.

## Turn it on

Turn the mode on with `--read-only`, or with `[general] read_only` in the
[config file](../configuration/config-file.md). Either one is enough. Neither
can force the mode off, so leaving out the flag never overrides a file that
turns the mode on. The mode also lasts for the life of the process. Reloading
a configuration can turn read-only on, but never off.

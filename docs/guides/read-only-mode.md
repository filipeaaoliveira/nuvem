---
title: Read-only mode
description: Refuse every command that changes a cloud resource.
sidebar:
  order: 13
---

```bash
nuvem --read-only
```

Refuses any command that changes cloud state, or that hands you an
interactive session which can. That is the rule; the current list is:

- starting, stopping, resetting, suspending, or resuming a Compute Engine
  instance;
- starting, stopping, restarting, or failing over a Cloud SQL instance;
- SSHing to an instance;
- opening a GKE cluster in k9s — k9s is a full read-write Kubernetes UI, and
  the launcher runs `get-credentials` when no kube context is present;
- every plugin (see [Plugins](../plugins/overview.md)) nuvem cannot prove only reads: a plugin's
  command line comes from your `plugins.toml` rather than from nuvem, so the
  mode runs one only when its declared `args` open with a gcloud command nuvem
  recognises as read-only, and refuses the rest — a reading command nuvem does
  not happen to name included. In read-only, choosing a refused plugin reports
  the refusal instead of opening its prompt or its confirmation.

The refusal applies at every point a mutation can still be ahead of the mode:
submitting from the palette, confirming a modal that was already open, and —
for k9s, whose confirmation only starts `get-credentials` — the interactive
launch itself, once those credentials come back. Turning the mode on stops an
in-flight k9s launch from reaching the cluster.

Read-only is a promise about **cloud resources**, not a claim that nuvem
changes nothing at all. Three things it deliberately still does, because none
of them touch a cloud resource: activating a gcloud configuration (local
gcloud state), opening a tunnel (a process on this machine), and copying a
value to the clipboard. Read commands — listing, detail views, logs — are
unaffected. Treat the exceptions as *these three*, not as "anything local":
if a new command changes cloud state or opens a session that can, it is
blocked.

Blocked commands stay in the command palette, shown and selectable, marked
with a trailing `— read-only` and rendered dimmed; submitting one reports why
instead of doing nothing. Turn it on with either `--read-only` or
`[general] read_only` in the [config file](../configuration/config-file.md) — either one is enough, and
neither can force the mode off, so the flag's absence never overrides a file
that turns it on. It is also sticky for the life of the process: reloading a
configuration can turn read-only on, never off.

---
title: GCE instances
description: Browse Compute Engine instances, run lifecycle actions, SSH, and copy their fields.
sidebar:
  order: 2
---

For GCE instances, pressing `r` runs:

```bash
gcloud --configuration <active-config> compute instances list --format=json --limit=500
```

In the GCE workspace, select an instance and press `:` to open the command
palette. Mutation availability follows the instance status: stopped instances
can be started; running instances can be stopped, reset, or suspended; and
suspended instances can be resumed. Every mutation opens a confirmation that
shows the instance, its zone, the configuration and project, the impact, and
the API call it will make, such as `compute.instances.stop`; see
[Before a change runs](overview.md#before-a-change-runs). Press `Enter` to
confirm or `Esc` to cancel. If the terminal is too small to show the whole
confirmation, enlarge it before confirming. After a successful mutation, the old
rows are cleared, the shared status area shows a refresh-in-progress state,
and the app automatically runs the active-context instance-list command once,
carrying GCE's own applied filter — a reload never borrows another service's
filter term, and never drops the reloaded service's own, because in label mode
the narrowing lives entirely in the pushed `--filter`:

```bash
gcloud --configuration <active-config> compute instances list --format=json --limit=500
```

with `--filter=<expression>` appended when that filter is server-expressible,
exactly as a manual refresh would push it.

If that follow-up refresh fails, the resource list remains stale/empty and the
shared status area shows the gcloud error. Press `r` to retry the refresh
manually.

The palette also offers `Show SSH command for selected GCE instance`, which
opens a preview of the exact SSH command. Press `Enter` to copy it or `Esc` to
cancel. You can copy the selected instance name and zone, any available
internal or external IP, and its self link when a project is configured.
Clipboard success or failure appears in the shared status area.

The palette also offers `SSH to selected GCE
instance` (alias `ssh`), which opens a preview of the exact `gcloud compute
ssh` command. Press `Enter` to launch it or `Esc` to cancel. Launching
suspends the TUI and hands the terminal to the SSH session; when the session
exits, the TUI redraws and the shared status area reports how it ended
(finished, exited with a non-zero status, terminated by signal, or failed to
launch). This differs from `Show SSH command`, which only copies the command
without running it.

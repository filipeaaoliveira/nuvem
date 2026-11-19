---
title: GCE instances
description: Browse Compute Engine instances, run lifecycle actions, SSH, and copy their fields.
sidebar:
  order: 2
---

For GCE instances, `r` reloads the list. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> compute instances list --format=json --limit=500
```

In the GCE workspace, select an instance and press `:` to open the command
palette. The actions on offer follow the instance status:

- A stopped instance can be started.
- A running instance can be stopped, reset, or suspended.
- A suspended instance can be resumed.

Every action opens a confirmation. It shows the instance, its zone, the
configuration and project, the impact, and the API call it will make, such as
`compute.instances.stop`. See
[Before a change runs](overview.md#before-a-change-runs). Press `Enter` to
confirm or `Esc` to cancel. If the terminal is too small to show the whole
confirmation, enlarge it before confirming.

After a successful action, the old rows are cleared and the shared status area
shows that a refresh is in progress. The app then reloads the instance list
once for the active context, with GCE's own applied filter. A reload never
borrows another service's filter term, and never drops GCE's own, because in
label mode the narrowing lives entirely in the pushed `--filter`. The
equivalent command is:

```bash
gcloud --configuration <active-config> compute instances list --format=json --limit=500
```

with `--filter=<expression>` appended when that filter is server-expressible,
exactly as a manual refresh would push it.

If that follow-up refresh fails, the resource list stays stale or empty and the
shared status area shows the error. Press `r` to retry the refresh.

The palette also offers `Show SSH command for selected GCE instance`, which
opens a preview of the exact SSH command. Press `Enter` to copy it or `Esc` to
cancel. You can also copy the selected instance name and zone, any available
internal or external IP, and, when a project is configured, its self link.
The shared status area reports whether the copy worked.

To run SSH, use `SSH to selected GCE instance` (alias `ssh`) from the palette.
It opens a preview of the exact `gcloud compute ssh` command. Press `Enter` to
launch it or `Esc` to cancel. Launching suspends the TUI and hands the terminal
to the SSH session. When the session exits, the TUI redraws and the shared
status area reports how it ended: finished, exited with a non-zero status,
terminated by signal, or failed to launch. `Show SSH command` only copies the
command and does not run it.

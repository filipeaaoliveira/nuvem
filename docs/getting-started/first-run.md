---
title: First run
description: Start nuvem against your gcloud configuration, or try it offline with demo mode.
sidebar:
  order: 2
---

You can start nuvem against your own gcloud configuration, or try it first
with no account at all.

## Try it without a project

To see the whole interface with no Google Cloud account, run:

```bash
nuvem --demo
```

Demo mode shows built-in sample data. A command that changes a resource,
such as stopping an instance, reports success but does nothing. See
[Demo mode](../guides/demo-mode.md).

## Run it against your project

1. Make sure that gcloud has a configuration and an account:

   ```bash
   gcloud config configurations list
   ```

2. Start nuvem:

   ```bash
   nuvem
   ```

   nuvem loads your gcloud configurations and opens on the active one.

3. To start on a different configuration, pass its name. nuvem asks gcloud to
   activate it, then reloads the configuration list:

   ```bash
   nuvem --configuration dev
   ```

4. To make sure that no key you press can change a cloud resource, add
   `--read-only`. See [Read-only mode](../guides/read-only-mode.md).

## Find your way around

nuvem is keyboard-only: it has no mouse support.

| Press | To |
| --- | --- |
| `h` | Open Home: health, inventory and recent operations |
| `g` | Open Resources: every service, with a list and a detail pane |
| `c` | Open Configurations: your gcloud configurations |
| `l` | Open Logging on the project's recent logs |
| `Ctrl-t` | Open Tunnels |
| `Ctrl-f` | Open FinOps |
| `:` | Open the command palette and run any command by name |
| `?` | Show the keys for the view you are in |
| `q` | Quit from Home or a list. In a detail view, `q` goes back to the list. |
| `Ctrl-C` | Quit from anywhere |

The footer shows the most useful keys for the current view. `?` lists all of
them. [Keys](../reference/keys.md) lists every key in every view.

## Background refresh

While Resources is open, nuvem refreshes the active service every 5 seconds.
The same tick drives the Cloud Logging tail. To change the interval, pass
`--poll-interval <seconds>` or set `poll_interval_secs` in the
[config file](../configuration/config-file.md). `0` turns background refresh
off. You can still press `r` to refresh, and a service still loads when you
open it.

## Next

- [Reading the screen](the-screen.md) explains the header, status markers and
  footer.
- [Browse resources](../guides/browse-resources.md) shows how to move through a
  service.
- [Config file](../configuration/config-file.md) lists every setting.

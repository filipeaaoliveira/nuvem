---
title: Tunnels
description: Open IAP, SSH and Cloud SQL Auth Proxy port forwards and manage them from one list.
sidebar:
  order: 9
---

nuvem can hold open long-running local port forwards to your cloud resources,
and keeps them in a list you can watch and kill. A tunnel never takes over
your terminal the way an SSH session does. It keeps running while you use the
rest of nuvem.

## Start a tunnel

Press `t` on a row in the GCE instances or Cloud SQL list. You can also choose
`Start tunnel for selected GCE instance` or
`Start tunnel for selected Cloud SQL instance` in the palette (aliases
`tunnel`, `pf`, `forward`). nuvem picks the tunnel kind for you:

- A GCE instance with an external IP gets a plain SSH local forward:
  `gcloud --configuration <active-config> compute ssh <name> --zone <zone> -- -N -o ExitOnForwardFailure=yes -L <local>:localhost:22`.
  `ExitOnForwardFailure=yes` makes a forward that cannot bind its local port
  exit, instead of staying up and carrying nothing.
- A GCE instance without an external IP is reachable only through
  Identity-Aware Proxy, so it gets
  `gcloud --configuration <active-config> compute start-iap-tunnel <name> 22 --local-host-port=localhost:<local> --zone <zone>`.
- A Cloud SQL instance gets the Cloud SQL Auth Proxy:
  `cloud-sql-proxy --address 127.0.0.1 --port <local> <project>:<region>:<name>`.
  The local port defaults to the engine's usual one: 5432 for PostgreSQL,
  3306 for MySQL, 1433 for SQL Server. If that port is taken, nuvem falls back
  to a free port. If `cloud-sql-proxy` is not on your `PATH`, point nuvem at it
  with `--cloud-sql-proxy-binary`.

The Tunnels list shows each tunnel's kind as a short badge:

| Kind | Started for | Program |
| --- | --- | --- |
| `SSH -L` | a GCE instance with an external IP | `gcloud compute ssh` |
| `IAP` | a GCE instance without an external IP | `gcloud compute start-iap-tunnel` |
| `SQL proxy` | a Cloud SQL instance | `cloud-sql-proxy` |

Before anything runs, a preview shows the exact command. Press `Enter` to
launch or `Esc` to cancel. nuvem runs the command without a shell, so instance
names, zones, and connection names are never interpolated into a shell
string. Every tunnel listens on loopback only (`localhost` / `127.0.0.1`), so
no one outside your machine can reach a forward.

## Manage your tunnels

Open the list with `Ctrl-t` from anywhere, or with the palette's `Tunnels`
entry. A bare `t` already starts a tunnel from a list and pauses the tail in
Logging, so the Tunnels list needs `Ctrl`. The list shows each
tunnel's status, kind, local port, target, age, and last message. Inside it:

- `j` / `k` or the arrows move the selection.
- `Shift-K` kills the selected tunnel.
- `d` dismisses a stopped or failed row.
- `Esc` goes back to the previous view.

Tunnels stay in the list when you switch gcloud configuration. The child
process keeps running, and each row records the context it was started in.
nuvem shuts down every remaining tunnel when you quit, when the terminal
nuvem runs in closes, or when nuvem is terminated.

## When a tunnel fails

A tunnel that cannot start says why, and does not hang. Each of these puts the
row in `Failed` with the underlying error text:

- A missing binary
- No free local port
- The port taken by something else between nuvem's check and the child
  process's own bind
- A child process that exits, or never reports that it is ready

Once a tunnel is up, the child process can report an error and still carry
traffic. The error then shows in the message column, and the tunnel keeps
running. An `ssh` forward that reports one refused connection is an example.

If a kill does not work, the row goes to `Failed` with the reason, so nuvem
never reports a stop that nobody saw. A killed tunnel always ends in `Stopped`
or `Failed`. It never stays in `Stopping`, holding its local port with no way
to dismiss it.

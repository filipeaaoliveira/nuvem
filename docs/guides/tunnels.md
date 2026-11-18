---
title: Tunnels
description: Open IAP, SSH and Cloud SQL Auth Proxy port forwards and manage them from one list.
sidebar:
  order: 9
---

nuvem can hold open long-running local port forwards to your cloud resources
and keep them in a list you can watch and kill. Unlike an SSH session, a tunnel
never takes over your terminal and keeps running while you carry on using the
app.

Press `t` on a row in the GCE instances or Cloud SQL list to start one, or use
the palette's `Start tunnel for selected GCE instance` / `Start tunnel for
selected Cloud SQL instance` (aliases `tunnel`, `pf`, `forward`). nuvem picks
the tunnel kind for you:

- a GCE instance **with** an external IP gets a plain SSH local forward
  (`gcloud --configuration <active-config> compute ssh <name> --zone <zone> -- -N -o ExitOnForwardFailure=yes -L <local>:localhost:22`),
  where `ExitOnForwardFailure=yes` makes a forward that cannot bind its local
  port end instead of sitting there carrying nothing;
- a GCE instance **without** an external IP is only reachable through
  Identity-Aware Proxy, so it gets
  `gcloud --configuration <active-config> compute start-iap-tunnel <name> 22 --local-host-port=localhost:<local> --zone <zone>`;
- a Cloud SQL instance gets the Cloud SQL Auth Proxy
  (`cloud-sql-proxy --address 127.0.0.1 --port <local> <project>:<region>:<name>`),
  with the local port defaulting to the engine's usual one — 5432 for
  PostgreSQL, 3306 for MySQL, 1433 for SQL Server — and falling back to a free
  port when that one is taken. Override the executable with
  `--cloud-sql-proxy-binary` when it is not on `PATH`.

The Tunnels list shows each tunnel's kind as a short badge:

| Kind | Started for | Program |
| --- | --- | --- |
| `SSH -L` | a GCE instance with an external IP | `gcloud compute ssh` |
| `IAP` | a GCE instance without an external IP | `gcloud compute start-iap-tunnel` |
| `SQL proxy` | a Cloud SQL instance | `cloud-sql-proxy` |

A preview shows the exact command before anything runs; press `Enter` to
launch or `Esc` to cancel. Commands are invoked without a shell, so instance
names, zones, and connection names are never interpolated into a shell string.
Every tunnel listens on loopback only (`localhost` / `127.0.0.1`), so a forward
is never reachable from outside the machine.

Open the list with `Ctrl-t` from anywhere, or with the palette's `Tunnels`
entry (a bare `t` is taken by the Logging tail toggle). It shows each tunnel's
status, kind, local port, target, age, and last message. Inside it:

- `j` / `k` or the arrows move the selection;
- `Shift-K` kills the selected tunnel;
- `d` dismisses a stopped or failed row;
- `Esc` goes back to the previous view.

A tunnel that cannot start says why rather than hanging: a missing binary, no
free local port, the port being taken between the check and the child's own
bind, or a child that exits or never reports itself ready all land the row in
`Failed` with the underlying error text. Once a tunnel is up, a complaint from
the child that does not stop it carrying traffic — an `ssh` forward reporting
one refused connection, say — is shown in the message column and leaves the
tunnel running. A kill that does not work says so the same way: the row lands in
`Failed` with the reason instead of reporting a stop nobody observed. A killed
tunnel always settles on `Stopped` or `Failed`, so it never sits in `Stopping`
holding its local port with no way to dismiss it. Tunnels stay in the list when
you switch gcloud configuration — the child is still running, and each row
records the context it was started under — and every remaining tunnel is shut
down when you quit nuvem, when the terminal that started it closes, or when
nuvem is terminated.

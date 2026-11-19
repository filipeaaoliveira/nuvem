---
title: Common problems
description: What nuvem's error messages mean and how to fix the problems people meet most.
sidebar:
  order: 1
---

Start here when nuvem shows an error, or does not do what you expect.

## Find out what went wrong

- The status line shows the last error in one sentence.
- Home (`h`) shows each failing service with a category and a hint.
- `Y` copies the full, sanitized detail of the last diagnostic to the
  clipboard. Tokens and keys are removed before it is shown or copied.
- `nuvem info` prints the path of nuvem's own log on its `log:` line. For more
  detail in that log, set `RUST_LOG=debug`.
- `nuvem info` lists every config-file warning without starting the interface.

## Error categories

When a call fails, nuvem names one of these categories with a hint. nuvem
retries a transient failure on the next background refresh. A persistent
failure waits until you press `r`, switch configuration or open another
service.

| Category | Hint nuvem shows | Retried |
| --- | --- | --- |
| permission denied | Check IAM roles for this account and project | persistent |
| API disabled | Enable the required API in the Cloud Console | persistent |
| invalid query | Fix the query filter and retry | persistent |
| quota | Wait and retry, or request a quota increase | transient |
| network | Check your network connection and retry | transient |
| parse error | Update nuvem — gcloud output did not match the expected schema | persistent |
| missing project | Set an active project on this configuration | persistent |
| unknown error | Retry, or copy the details to investigate | transient |
| missing CLI | Install the missing CLI and ensure it is on PATH | persistent |
| not authenticated | Run `gcloud auth login` to reauthenticate | persistent |
| invalid input | Check the command arguments and try again | persistent |
| cancelled | The operation was cancelled; retry when ready | persistent |

## Symptoms and fixes

### nuvem exits with code 3

Standard input or standard output is not a terminal, for example in a pipe or
a CI step. nuvem
needs a terminal to draw on. `nuvem info` works without one. See
[Command line](../reference/cli.md#exit-codes).

### "missing CLI"

A program nuvem runs is not on your `PATH`: `gcloud`, `bq` (BigQuery), `k9s`
or `kubectl` (k9s launch), or `cloud-sql-proxy` (Cloud SQL tunnels). Home
names which one. Install it, or point nuvem at it with
`--gcloud-binary`, `--bq-binary`, `--k9s-binary`, `--kubectl-binary` or
`--cloud-sql-proxy-binary`.

### "not authenticated"

Your gcloud credentials expired. Run `gcloud auth login`, then press `r`.

### A list is empty

An empty list with `[OK]` means the project has none of that resource. An
empty list after a filter means the filter matched nothing. A malformed filter
never empties the table. It shows a `⚠` beside the filter instead. Label
values are matched case-sensitively, so `-l env=Prod` does not find `prod`.
See [Filter a list](../guides/filter-a-list.md).

### My plugins do not appear

1. Run `nuvem info`. The `plugins:` line shows which file nuvem read. On macOS
   that is `~/Library/Application Support/nuvem/plugins.toml`, not
   `~/.config/nuvem/plugins.toml`.
2. If it says `(0 loaded)`, the `warning:` lines say why.
3. A file that any user can write is refused: run `chmod o-w` on it.
4. A plugin appears only in the service named by its `scope`. It appears only
   in a list, not a detail pane, and only with a row selected. The row must
   also have every field the plugin's templates use. See
   [When a plugin is not offered](../plugins/overview.md#when-a-plugin-is-not-offered).
5. nuvem reads the plugin file only at startup. After you edit it, restart
   nuvem.

### My config change did nothing

Run **Reload configuration** from the palette, or restart. `default_service`
and the plugin file need a restart. Look for config problems on the status
line and in help (`?`). nuvem drops a bad entry with a warning and keeps
running. See
[Config problems](../configuration/config-problems.md).

### Copying does nothing on Linux

nuvem copies through `wl-copy`, `xclip` or `xsel`, and tries them in that
order. Install one. If none is found, the status line says so.

### The screen says the terminal is too small

nuvem needs at least 60 columns by 12 rows. A confirmation that cannot show its
whole command refuses `Enter` until the terminal is large enough.

### A tunnel shows Failed

The message column says why: a missing binary, no free local port, a port
taken between the check and the bind, or a child that exited or never became
ready. See [Tunnels](../guides/tunnels.md).

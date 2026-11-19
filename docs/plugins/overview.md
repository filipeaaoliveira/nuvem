---
title: How plugins work
description: Offer your own gcloud commands against the selected row, behind a confirmation.
sidebar:
  order: 1
---

A plugin is a `gcloud` command of your own that nuvem offers against the
selected row. Each plugin says: when this service's list is in view, offer
this labelled command against the selected row. Plugins run `gcloud` and
nothing else. See [Why plugins run only gcloud](#why-plugins-run-only-gcloud).
[Write a plugin](write-a-plugin.md) walks through a first one.

## Where plugins live

Plugins are the part of nuvem's configuration that runs a process, so they
live in their own file, `plugins.toml`, not in `config.toml`:

- `~/.config/nuvem/plugins.toml` on Linux.
- `~/Library/Application Support/nuvem/plugins.toml` on macOS.
- The path given with `--plugins <path>`.

nuvem never derives that location from `--config`. You can point `--config` at
a config file from a repository, and that file must not be able to bring
commands with it.

nuvem loads no plugins from a `plugins.toml` that is world-writable, because
any local account can add a command to it that looks exactly like one you
wrote. nuvem says so at startup. To fix it, run `chmod o-w plugins.toml`.
Group-writable files are accepted. nuvem also refuses a path that is not an
ordinary file, or a file over 1 MiB.

## Run a plugin from the palette

1. Select a row in a Resources list.
2. Press `:`. A plugin whose templates the selected row can fill shows up in
   the command palette under its own label.
3. Choose it. If the plugin declares inputs, nuvem asks for them first. See
   [Typed inputs](typed-inputs.md).
4. Choosing it opens a confirmation showing the exact command. `Enter` runs it
   and `Esc` cancels. A plugin declared `dangerous = true` will not offer
   `Enter` until you have typed `1`.

If the file is well formed, it loads, and `nuvem info` says how many entries
it accepted. If it is malformed, nuvem says exactly what is wrong with it.

## Follow a run

Every run is recorded in Home's *Recent operations* log, pending from the
moment you confirm and settled when the command exits. To read what the run
printed, use [Plugin output](plugin-output.md).

## Stop a run

A plugin runs your argv, which can be a command that never exits. `Ctrl-x`
cancels every plugin currently running, from anywhere. Like `Ctrl-C`, it works
while a modal, the command palette, help, a filter or an armed plugin prefix
has the keyboard, because a plugin outlives the view it was launched from.

If the command had not been issued yet, the entry settles as `Cancelled`, and
nothing changed outside nuvem. If it had, the entry settles as `Unknown`,
because a killed command can already have done its work.

## After a run

However the run ends, the resource table reloads, provided you are still
looking at it under the configuration you launched the run from. The reload
happens after every plugin, a reading one included, so the table agrees with
the log. It is in place, so your marks, your cursor and an open detail pane
survive the reload.

A run that finishes while you are in another workspace reloads nothing, so that
table is only as fresh as the last poll until you press `r`. A run that
finishes after you have switched configuration reloads nothing either, but the
switch itself already reloaded the table.

## Plugins and read-only mode

`--read-only` runs a plugin only when nuvem can prove from its declaration that
it reads. The first elements of `args` must spell out a gcloud command that
nuvem recognises as read-only, such as `["compute", "instances", "describe", ...]`.
nuvem refuses everything else, including a command it does not recognise. A
plugin that names a reading command that is not on the list is refused too.
To run it, start nuvem without `--read-only`. A plugin whose verb comes from an
input is always refused, because what it runs is not known until you type it.

## No shell

Nothing in the schema can run a shell command: `args` is an argv rather than a
command line, the program is always `gcloud`, and there is no shell anywhere on
this path.

## Run a plugin by key

A plugin also has a key of its own, reached through the **plugin prefix key**
`,`. Press `,` with a resource list in view, and the status line names the keys
this service declares, in the order your file declares them. `,` also works
with a detail pane open, where the palette lists no plugins: a palette entry
is a row of a list, and the pane is not that list. With the file in the
[plugin file reference](plugin-file-reference.md#example) and the Compute Engine
list in view, the status line shows one key:

```text
Plugin key: d (Esc to cancel)
```

The Cloud SQL plugin's `D` is not on that line, and you cannot press it there.
A key is offered under the scope it was declared for, the same way its palette
entry is. nuvem builds the line at the moment you press `,`, because the keys
that apply depend on your file and on the service in view.

The next key you press runs that plugin through the same confirmation, the
same `dangerous` acknowledgement, the same *Recent operations* entry, the same
`Ctrl-x` and the same reload as the palette. The key is a shortcut into that
path, not a second way to run a command. `Esc` disarms the prefix without a
message, because that is not an error.

Every way a keyed press can come to nothing says so, because two swallowed
keystrokes look exactly like a plugin that ran and printed nothing:

- `,` in a service with no plugin declared refuses at once and does not arm,
  so the key you press next still does its usual job.
- A key nobody declared names itself and the service you pressed it in.
- A plugin whose templates this row cannot fill names the field it needed, in
  the same words the palette uses when it withholds the entry.
- If nothing is under the cursor (an empty list, or a first load still in
  flight), `,` still arms, because it reads what your file declares, not what
  is selected. The key you press next tells you there is nothing to run a
  plugin on.
- A keyed press with no resolved gcloud configuration says so, because the
  `--configuration` that nuvem prepends has nothing to name.

The prefix stays tied to the service you armed it over. Something other than
your keyboard can change that service in between, such as a configuration
activation landing, or the first configuration load arriving as you press `,`.
If that happens, the prompt goes away, and the key you press next says the
view changed and names both services. It does not run the plugin that the new
service binds to that key. A key is unique only within a scope, so the same
letter can name *describe* in one service and *delete* in another.

## What you see, end to end

Take the `describe` plugin from the
[plugin file reference](plugin-file-reference.md#example), with the Compute Engine list in view, the row
`web-01` (zone `europe-west1-b`) selected, and the `dev` gcloud configuration
active. The command palette (`:`) offers an entry labelled **Describe web-01**,
which is your `label` rendered against that row. Choosing it opens a
confirmation:

```text
gcloud --configuration dev compute instances describe web-01 --zone europe-west1-b
```

`Enter` runs exactly that line. `Esc` closes the confirmation and runs nothing.
If the plugin is declared `dangerous = true`, the confirmation asks you to type
`1` first, and does not offer `Enter` until you have.

The run happens in the background, so nuvem stays usable while gcloud works.
Its outcome lands in Home's *Recent operations*. An entry appears as `pending`
the moment you confirm. It settles as succeeded or failed once the command
exits, with a short excerpt of what gcloud printed. The status line reports
the outcome too, unless you have since switched gcloud configuration or
started something else nuvem reports on, such as a refresh. In those cases the
run settles in the log only, and the status line keeps to what you are looking
at now.

nuvem prepends `--configuration dev`. You do not write it. Every command nuvem
builds carries it, so a plugin always names the configuration you are looking
at, never whatever configuration your shell has active. Each `args` element is
rendered on its own and stays one argument. The confirmation shows a value
that contains a space in quotes, so you can see it is one argument. That
quoting is for display only. gcloud receives the unquoted argv, and there is
no shell on this path.

## When a plugin is not offered

A plugin is offered against the selected row or not at all. If something
withholds it, the palette does not list it.

The case you are most likely to meet is an open detail pane. Select a GCE
instance, open its detail pane, press `:`, and every plugin is gone from the
palette. A plugin is scoped to a row in a list, and the pane is not that list.
Close the detail pane and they are all back. This is the one case where the two
ways in differ. `,` and the plugin's key still reach the plugin from the pane,
because a key pressed against the resource on screen can only mean that
resource.

A plugin also needs a resolved gcloud configuration. Every plugin command
carries `--configuration <active>`. If no configuration resolves, there is
nothing to name, and no plugin is offered, even with a row selected and a list
in view. Plugins are also offered only from the resources workspace (not from
tunnels or logs), only when `plugins.toml` declares at least one, and only
when a row is selected.

Three more things withhold a single plugin once its command is being rendered
against the row:

- A field the row has no value for. `{external_ip}` against an instance with
  no external IP renders nothing, so the plugin is withheld instead of offered
  as a command with a hole in it.
- A substituted value that begins with `-`. gcloud reads it as a flag, which
  silently changes what the command means. This applies only to values that
  come out of a template. A literal you wrote yourself, like `"--quiet"` in
  `args`, is your own text and passes through untouched.
- A rendered value carrying a control character. The confirmation line cannot
  show it faithfully, and you cannot check a command you cannot read.

No declared input *type* withholds a plugin: the prompt collects every one of
them.

There is deliberately no limit on how long a rendered value may be. An
over-long command line hits the confirmation's fit check, which asks for a
larger terminal. nuvem never truncates it into a line that reads as a
different command from the one you approve. That check also gates
`Enter`: a confirmation the terminal is too small to render cannot be
confirmed, only cancelled with `Esc`. The input prompt has the same check on
its own modal, one step earlier, so you cannot submit values you cannot read.

`--read-only` is the one case that does not withhold. A plugin it refuses stays
in the palette, dimmed and marked `— read-only`. Choosing it reports the
refusal instead of opening its prompt or its confirmation. See
[Read-only mode](../guides/read-only-mode.md) for which plugins fall under it.

## Plugins are read once, at startup

Editing `plugins.toml` while nuvem is running changes nothing until you restart
it. **Reload configuration** notices that the file changed and says
`restart required for: plugins`. It does not swap the plugin set under a
palette or a confirmation that already points into it.

## Why plugins run only gcloud

`args` is the argv `gcloud` receives. There is no field for the program, by
design. A plugin cannot run `bq`, `gsutil`, `kubectl`, or anything else, and
no program field is planned.

There are two reasons. The first is the main one:

- `--read-only` cannot vouch for another program. A plugin is allowed under
  `--read-only` only when its declared argv matches a fixed positional prefix
  in nuvem's allowlist of reading `gcloud` command paths. That allowlist keeps
  the refusal honest: an unmatched path means "nuvem never vouched for this",
  never "nuvem checked and it is safe". Nothing a `bq` or `gsutil` plugin
  declares can match it, so `--read-only` refuses every such plugin, however
  plainly it only reads. To make those work, nuvem needs a second allowlist
  per program, and each one goes stale silently as that program's commands
  change.
- A program field widens what a config file can run. This feature rests on
  one guarantee: a `plugins.toml` names arguments to one binary that nuvem
  already runs. A program field changes that to "any binary on your `PATH`",
  which is a different promise.

If you need another tool, point a plugin at a `gcloud` command that does the
job, or run the other tool outside nuvem.

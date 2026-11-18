---
title: How plugins work
description: Offer your own gcloud commands against the selected row, behind a confirmation.
sidebar:
  order: 1
---

Plugins are your own `gcloud` commands, offered against the selected row: "when
this service's list is in view, offer this labelled command against the
selected row". They run `gcloud` and nothing else; see
[Why plugins run only gcloud](#why-plugins-run-only-gcloud).
[Write a plugin](write-a-plugin.md) walks through a first one.

## Where plugins live

Plugins are the part of nuvem's configuration that runs a process, so they do
**not** live in `config.toml`. They live in `plugins.toml`:

- `~/.config/nuvem/plugins.toml` on Linux;
- `~/Library/Application Support/nuvem/plugins.toml` on macOS;
- or the path given with `--plugins <path>`.

That location is never derived from `--config`. Pointing `--config` at a
config file from a repository is a supported workflow, and it must not be able
to bring commands along with it.

nuvem loads no plugins from a `plugins.toml` that is **world-writable**: any
local account could add a command to it that looks exactly like one you wrote.
It says so at startup; `chmod o-w plugins.toml` fixes it. Group-writable files
are accepted. A path that is not an ordinary file, or a file over 1 MiB, is
refused too.

## Run a plugin from the palette

1. Select a row in a Resources list.
2. Press `:`. A plugin whose templates the selected row can fill shows up in
   the command palette under its own label.
3. Choose it. If the plugin declares inputs, nuvem asks for them first; see
   [Typed inputs](typed-inputs.md).
4. Choosing it opens a confirmation showing the exact command — `Enter` runs it
   and `Esc` cancels. A plugin declared `dangerous = true` will not offer
   `Enter` until you have typed `1`.

A well-formed file loads and `nuvem info` says how many entries it accepted; a
malformed one says exactly what is wrong with it.

## Follow a run

Every run is recorded in Home's *Recent operations* log, pending from the
moment you confirm and settled when the command exits. To read what the run
printed, use [Plugin output](plugin-output.md).

## Stop a run

A plugin's argv is yours, so it can also never exit: `Ctrl-x` cancels every
plugin currently running, from anywhere. Like `Ctrl-C` it works while a modal,
the command palette, help, a filter or an armed plugin prefix has the keyboard,
because a plugin outlives the view it was launched from.

The entry it settles reads `Cancelled` if the command had not been issued yet —
nothing changed outside nuvem — and `Unknown` if it had, because a killed
command may already have done its work.

## After a run

However the run ends, the resource table reloads — provided you are still
looking at it, under the configuration you launched the run from. The reload
happens after every plugin, a reading one included, so the table agrees with
the log. It is in place, so your marks, your cursor and an open detail pane
survive the reload.

A run that finishes while you are in another workspace reloads nothing, so that
table is only as fresh as the last poll until you press `r`. A run that
finishes after you have switched configuration reloads nothing either, but the
switch itself already reloaded the table.

## Plugins and read-only mode

`--read-only` runs a plugin only when nuvem can prove from its declaration that
it reads: the first elements of `args` have to spell out a gcloud command nuvem
recognises as read-only, such as `["compute", "instances", "describe", ...]`.
Everything else is refused, including a command nuvem does not recognise — so a
plugin can be refused for naming a reading command that is not on the list, and
the fix is to run it without `--read-only`. A plugin whose verb comes from an
input is always refused, because what it would run is not known until you have
typed it.

## No shell

Nothing in the schema can run a shell command: `args` is an argv rather than a
command line, the program is always `gcloud`, and there is no shell anywhere on
this path.

## Run a plugin by key

A plugin also has a key of its own, reached through the **plugin prefix key**
`,`. Press `,` with a resource list in view — or with a detail pane open, where
the palette lists no plugins because a palette entry is a row of a list and the
pane is not that list — and the status line names the keys this service declares,
in the order your file declares them. With the file in the
[plugin file reference](plugin-file-reference.md#example) and the Compute Engine
list in view, that is one key:

```text
Plugin key: d (Esc to cancel)
```

The Cloud SQL plugin's `D` is not on that line and not pressable there: a key is
offered under the scope it was declared for, the same way its palette entry is.
The line has to be built at the moment you ask, because which keys exist is your
file's business and nuvem cannot know which ones apply until it knows which
service you are looking at. The next key you press runs that plugin through the
same confirmation, the same `dangerous` acknowledgement, the same *Recent
operations* entry, the same `Ctrl-x` and the same reload described above: the key
is a shortcut into that path, not a second way to run a command. `Esc` disarms
the prefix, which is not an error and says nothing.

Every way a keyed press can come to nothing says so, because two swallowed
keystrokes look exactly like a plugin that ran and printed nothing. `,` in a
service with no plugin declared refuses at once and does not arm, so the key you
press next still does its usual job. A key nobody declared names itself and the
service you pressed it in. A plugin whose templates this row cannot fill names
the field it needed, in the same words the palette path uses when it withholds
the entry. With nothing under the cursor — an empty list, or a first load still
in flight — `,` still arms, because it asks what your file declares rather than
what happens to be selected; the key you press next is the one that tells you
there is nothing to run a plugin on. And a keyed
press with no gcloud configuration resolved says so, because the
`--configuration` nuvem prepends would have nothing to name.

The prefix is armed over the service you armed it over. If something other than
your keyboard changes that service in between — a configuration activation
landing, or the first configuration load arriving as you press `,` — the prompt
goes away and the key you press next says the view changed, naming both
services. It does not run the plugin the new service happens to bind that key
to: a key is unique within a scope, so the same letter can name *describe* in
one service and *delete* in another.

## What you see, end to end

Take the `describe` plugin from the
[plugin file reference](plugin-file-reference.md#example), with the Compute Engine list in view, the row
`web-01` (zone `europe-west1-b`) selected, and the `dev` gcloud configuration
active. The command palette (`:`) offers an entry labelled **Describe web-01**
— your `label`, rendered against that row. Choosing it opens a confirmation:

```text
gcloud --configuration dev compute instances describe web-01 --zone europe-west1-b
```

`Enter` runs exactly that line; `Esc` closes the confirmation and runs nothing.
Had the plugin been declared `dangerous = true`, the confirmation would ask you
to type `1` first and would not offer `Enter` until you had.

The run happens in the background, so nuvem stays usable while gcloud works.
Its outcome lands in Home's *Recent operations*: an entry appears as `pending`
the moment you confirm, and settles as succeeded or failed once the command
exits, carrying a short excerpt of what gcloud printed. The status line says so
too, unless you have switched gcloud configuration in the meantime, or started
something else nuvem is reporting on — a run confirmed under a configuration you
have since left, or overtaken by a refresh you asked for after it, settles in the
log without narrating itself over what you are looking at now.

`--configuration dev` is prepended by nuvem, not written by you. Every command
nuvem builds carries it, so a plugin can only ever name the configuration you
are looking at rather than whatever configuration your shell happens to have
active. Each `args` element is rendered on its own and stays one argument: a
value containing a space is shown quoted in the confirmation so you can see it
is one argument, and that quoting is display only — the argv gcloud receives is
the unquoted one, and there is no shell on this path either way.

## When a plugin is not offered

A plugin is offered against the selected row or not at all. When something
withholds it, the palette simply does not list it.

The case you are most likely to meet is an **open detail pane**. Select a GCE
instance, open its detail pane, press `:`, and every plugin is gone from the
palette — a plugin is scoped to a row in a list, and the pane is not that list.
Close the detail pane and they are all back. This is the one case where the two
ways in differ: `,` and the plugin's key still reach it from the pane, because a
key pressed against the resource on screen is unambiguous about which row it
means in a way a palette full of entries is not.

A plugin also needs a **resolved gcloud configuration**. Every plugin command
carries `--configuration <active>`, so if no configuration resolves there is
nothing to name and no plugin is offered, even with a row selected and a list
in view. Beyond that, plugins are offered only from the resources workspace —
not from tunnels or logs — only when `plugins.toml` declares at least one, and
only when a row is actually selected.

Four more things withhold a single plugin once its command is being rendered
against the row:

- **A field the row has no value for.** `{external_ip}` against an instance
  with no external IP renders nothing, so the plugin is withheld rather than
  offered as a command with a hole in it.
- **A substituted value that begins with `-`.** gcloud would read it as a flag,
  which would silently change what the command means. This applies only to
  values that come out of a template — a literal you wrote yourself, like
  `"--quiet"` in `args`, is your own text and is passed through untouched.
- **A rendered value carrying a control character.** The confirmation line
  could not show it faithfully, and a command you cannot read is a command you
  cannot check.

No declared input *type* withholds a plugin: the prompt collects every one of
them.

There is deliberately no limit on how long a rendered value may be. An
over-long command line hits the confirmation's fit check, which asks for a
larger terminal, rather than being silently truncated into a line that reads as
a different command than the one you would be approving. That check also gates
`Enter`: a confirmation the terminal is too small to render cannot be
confirmed, only cancelled with `Esc`. The input prompt carries the same check on
its own modal, one step earlier — values you cannot read are values you cannot
submit.

`--read-only` is the one case that does not withhold: a plugin it refuses stays
in the palette, dimmed and marked `— read-only`, and choosing it reports the
refusal instead of opening its prompt or its confirmation. See
[Read-only mode](../guides/read-only-mode.md) for which plugins fall under it.

## Plugins are read once, at startup

Editing `plugins.toml` while nuvem is running changes nothing until you restart
it. **Reload configuration** notices the file has changed and says so —
`restart required for: plugins` — rather than swapping the plugin set under a
palette or a confirmation that is already pointing into it.

## Why plugins run only gcloud

`args` is the argv `gcloud` receives; there is no field for the program, and
that is a decision rather than an unfinished edge. A plugin cannot run `bq`,
`gsutil`, `kubectl`, or anything else, and adding a program field is not a
planned extension of this feature.

Two reasons, and the first is the load-bearing one:

- **`--read-only` could not vouch for another program.** A plugin is allowed
  under `--read-only` only when its declared argv matches a fixed positional
  prefix in nuvem's allowlist of reading `gcloud` command paths. That allowlist
  is what makes the refusal honest: an unmatched path means "nuvem never
  vouched for this", never "nuvem checked and it is safe". Nothing a `bq` or
  `gsutil` plugin could declare would match it, so every such plugin would be
  refused under `--read-only` no matter how plainly it only reads — and making
  it work would mean a second allowlist per program, each one silently rotting
  as that program's surface changes.
- **It would widen what a config file can execute.** The guarantee this feature
  rests on is that a `plugins.toml` names arguments to one binary nuvem already
  shells to. A program field changes that to "any binary on the operator's
  `PATH`", which is a different promise, not a larger version of the same one.

If you need another tool, wrap it: point a plugin at a `gcloud` command that
does the job, or run the other tool outside nuvem. The gap is deliberate, and
closing it needs a design pass of its own — starting with what `--read-only`
means when nuvem cannot classify what it is about to run.

# Plugins

User-declared, row-scoped `gcloud` commands. This is the one part of nuvem's
configuration that causes a process to run, which is why it lives in a file of
its own — see [configuration.md](configuration.md) for everything else.

### Plugins

Plugins are user-declared, row-scoped `gcloud` commands: "when this service's
list is in view, offer this labelled command against the selected row". They run
`gcloud` and nothing else — see [Why plugins run only
gcloud](#why-plugins-run-only-gcloud). They are
the part of nuvem's configuration that causes a process to run, and so they do
**not** live in `config.toml`. They live in `plugins.toml`, read from
`dirs::config_dir()/nuvem/plugins.toml` —
`~/.config/nuvem/plugins.toml` on Linux, `~/Library/Application Support/nuvem/plugins.toml` on macOS — or from
an explicit `--plugins <path>`. That location is deliberately **not** derived
from `--config`: pointing `--config` at a repository's config file is a
workflow this README advertises, and it must not be able to bring executable
declarations along with it.

For the same reason, `plugins.toml` is refused if it is **world-writable**:
a file any local account can append to is a command any local account can get
offered, in a confirmation that looks exactly like one you wrote. nuvem loads
no plugins in that case and says so at startup; `chmod o-w plugins.toml`
restores it. Group-writable files are left alone, since `umask 002` setups are
ordinary. All three configuration files are also refused if they are not
ordinary files or exceed 1 MiB — a named pipe left at one of these paths would
otherwise hang startup with nothing on screen.

**This release runs a plugin — from the command palette or from the plugin's
own key — after a confirmation showing the exact argv.** A well-formed
file loads and `nuvem info` says how many entries it accepted; a malformed one
says exactly what is wrong with it. A plugin whose templates the selected row
can fill shows up in the command palette (`:`) under its own label, and
choosing it opens a confirmation showing the exact command — `Enter` runs it
and `Esc` cancels, and a plugin declared `dangerous = true` will not offer
`Enter` until you have typed `1`. A plugin declaring values this release can
ask for is prompted for them first; see [Typed inputs](#typed-inputs). Every
run is recorded in Home's *Recent
operations* log, pending from the moment you confirm and settled when the
command exits. A plugin's argv is yours, so it can also never exit: `Ctrl-x`
cancels every plugin currently running, from anywhere. Like `Ctrl-C` it resolves
ahead of whatever owns the keyboard, so a modal, the command palette, help, a
filter you are typing into or an armed plugin prefix are all places you can kill
a runaway run from — a plugin outlives the view it was launched from, so the key
that stops it is not scoped to one either. The entry it settles reads
`Cancelled` if the command had not been issued yet — nothing changed
outside nuvem — and `Unknown` if it had, because a killed command may already
have done its work. However the run ends, the resource table reloads — provided
you are still looking at it, under the configuration you launched the run from.
The reload happens after every plugin, a reading one included, rather than leave
the table disagreeing with the log: reconciling the screen after any gcloud run
is worth one list call. It is in place, so your marks, your cursor and an open
detail pane survive the reload. A run that finishes while you are in another
workspace reloads nothing, so that table is only as fresh as the last poll until
you press `r`. A run that finishes after you have switched configuration reloads
nothing either, but the switch itself already reloaded the table. `--read-only`
runs a plugin only when nuvem can prove from its declaration that it reads: the
first elements of `args` have to spell out a gcloud command nuvem recognises as
read-only, such as `["compute", "instances", "describe", ...]`. Everything else
is refused, including a command nuvem does not recognise — so a plugin can be
refused for naming a reading command that is simply not on the list, and the fix
is to run it without `--read-only`. A plugin whose verb comes from an input is
always refused, because what it would run is not known until you have typed it.
Nothing in the schema can run a shell command: `args` is an argv rather than a
command line, the program is always `gcloud`, and there is no shell anywhere on
this path.

#### Why plugins run only gcloud

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

#### Reading what a plugin printed

The operations-log entry says a run finished; it does not say what the run
said. **Plugin output** in the command palette (`:`) opens what the command
printed, as lines:

```text
┌plugin output──────────────────────────────────────────┐
│Describe web-01 · Succeeded                            │
│name: web-01                                           │
│machineType: e2-standard-2                             │
│status: RUNNING                                        │
└run 1 of 3 · line 1 of 84 · ↑↓ scroll · n/p run · y ───┘
```

`↑`/`↓` (or `j`/`k`) scroll a line, `PgUp`/`PgDn` twenty — a run shorter than
one page does not scroll at all, and a longer one stops with its tail on
screen rather than above a screen of blank rows — and `n`/`p` move
between the runs the log still holds — most recent first, stopping at either
end rather than wrapping. `y` copies the run on screen to the clipboard and
`Esc` closes. The command is offered only once some run has actually printed something, so
choosing it never opens an empty box: a run that failed before producing any
output — no gcloud on `PATH`, say — settles in the log with no output to
attach, and the entry stays absent.

Both streams are kept, and stderr has a reserved share of the bound, so a
`--format=json` listing far past it cannot silence the warning printed beside
it. A line the command wrote to stderr is shown marked `stderr │`, including
for a run that exited 0 — a quota warning printed alongside a successful
answer is not lost. `y` copies the marks too, so a failed run's trailer stays
distinguishable from its answer in the clipboard. A run that *failed* keeps its
stderr too, which is the case with the most to read: the log's own detail line
is bounded at 200 bytes, and a gcloud error trailer is routinely longer than
that.

What is kept is bounded: 256 KiB per run, and the log holds ten records. Past
that bound the **head** is kept — `gcloud` puts the identifying fields first,
so a truncated `describe` still names what it described — and the last line
says how much was dropped rather than simply ending. A single line longer than
the bound (`--format=json` prints one) is cut mid-line rather than dropped, on
a character boundary. Nothing after the cut is shown even if it would have
fitted, so the head never has a hole in it. `y` copies what was kept.

Output is redacted on the way in, by the same rule the status line uses: a
token that carries a credential becomes `[redacted]`, while one that merely
names something (`name: db-password`) is left alone. Indentation survives,
because YAML output is read by its shape; control characters do not.

Plugin output lives in memory only, and only for as long as its record is in
the log — ten operations, or until you quit. Nothing is written to disk. A
run you are *reading* survives its record being evicted: the viewer holds the
copy it opened, and says `run no longer in the log` instead of claiming a
place among runs it has left.

A plugin also has a key of its own, reached through the **plugin prefix key**
`,`. Press `,` with a resource list in view — or with a detail pane open, where
the palette lists no plugins because a palette entry is a row of a list and the
pane is not that list — and the status line names the keys this service declares,
in the order your file declares them. With the file below and the Compute Engine
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

```toml
version = 1                      # schema version; missing = 1; unknown = whole file rejected

[[plugins]]
id       = "describe"            # unique within a scope; names the plugin in warnings
scope    = "compute"             # one service, from the same vocabulary [aliases]/[columns]/[[jumps]] use
key      = "d"                   # press `,` then this to run it on the selected row
label    = "Describe {name}"     # palette entry and confirmation heading; templated
args     = ["compute", "instances", "describe", "{name}", "--zone", "{zone}"]
dangerous = false                # true adds a typed acknowledgement to the confirmation

[[plugins]]
id       = "delete-instance"
scope    = "cloudsql"
key      = "D"
label    = "Delete instance {name}"
args     = ["sql", "instances", "delete", "{name}"]
dangerous = true
```

A template renders whatever the row holds, so the flag you pair it with has to
be right for every row in that service. Persistent disks are the case to watch:
`compute disks list` returns zonal and regional disks in one table, so a disks
plugin's `{location}` is sometimes a zone and sometimes a region, and pairing
it with a fixed `--zone` fails on half the rows.

| field | required | meaning |
| --- | --- | --- |
| `id` | yes | unique within a scope; names the plugin in warnings |
| `scope` | yes | one service, from the same vocabulary `[aliases]` uses |
| `key` | yes | a single character, pressed after `,` to run this plugin; unique within a scope |
| `label` | yes | palette entry and confirmation heading; templated |
| `args` | yes | one string per argv element; each templated independently. The elements gcloud receives — the program is not one of them |
| `dangerous` | no (`false`) | adds a typed acknowledgement to the confirmation |
| `inputs` | no (none) | values you supply when the plugin runs; see [Typed inputs](#typed-inputs) |

`args` is an array of strings, never one string: splitting a string would be a
shell parser under another name, and there is no shell anywhere on this path.
Each element is templated independently and becomes exactly one argv element.
The program is not part of `args` — it is `gcloud`.

`{field}` templates use the same per-service vocabulary as
[Jump rules](configuration.md#jump-rules) above, validated against the `scope` you named. A
Cloud SQL plugin can write `{region}` and a Compute plugin `{zone}`; writing
`{zone}` under `scope = "cloudsql"` is rejected at load with a warning naming
the field, rather than accepted as a template that could never render.

#### Typed inputs

Some commands need a value the row cannot supply: the machine type to resize
*to*, a new disk size, a reason to record. A plugin declares those as `inputs`,
and each one's `name` becomes a `{field}` its own `label` and `args` may use,
alongside the row's fields:

```toml
[[plugins]]
id    = "grow-disk"
scope = "disks"
key   = "G"
label = "Grow {name} to {new_size}GB"
args  = ["compute", "disks", "resize", "{name}", "--size", "{new_size}"]

[[plugins.inputs]]
name    = "new_size"          # becomes {new_size}; unique within this plugin
type    = "integer"           # string | number | integer | bool | dropdown
default = 200                 # optional
```

| field | required | meaning |
| --- | --- | --- |
| `name` | yes | the `{field}` this input fills; a letter or `_`, then letters, digits or `_` |
| `type` | yes | `string`, `number`, `integer`, `bool` or `dropdown` |
| `choices` | for `dropdown` | non-empty list of strings; only a dropdown takes it |
| `default` | no | must match `type`, and be one of `choices` for a dropdown |

`integer` is its own type rather than a flag on `number` because the value
reaches gcloud verbatim: `number` accepts `0.5`, `integer` accepts `200` and
rejects `1.5` at load, and the type you declared is the type that can be sent.

An input may not be named after one of its scope's own fields, and which names
those are depends on the scope. On Compute Engine `{zone}` and `{machine_type}`
are the row's own, so an input called `zone` is rejected at load rather than one
of the two silently winning, and an input for the machine type to resize *to*
has to be named something else — `target_type`. On persistent disks neither
name is taken, so a disks plugin could call its input `size`; `new_size` above
is a readability choice, not a requirement.

**Every type is prompted for.** Choosing a plugin whose `args` or `label` name
inputs opens a prompt first, one field per input the plugin's own templates
name, in declaration order:

```text
┌plugin input───────────────────────────────────┐
│Resize web-01                                  │
│                                               │
│> reason: quarterly capacity review            │
│  tier: < db-g1-small >                        │
│  async: < false >                             │
│                                               │
│Enter continue   Tab next field   Esc cancel   │
└───────────────────────────────────────────────┘
```

A `string`, `number` or `integer` field is typed into; a `dropdown` and a `bool`
are moved through with `Left` and `Right`. `Tab` and `Shift-Tab` move between
fields, and the `>` marks the one your keystrokes reach. `Enter` is inert until every field
holds something it can send, and the modal names only the keys that do something
right now, so `Enter` is not offered until it works. `Esc` closes the prompt and
**runs nothing**: no log entry, because nothing was ever issued.

A field that is not ready says what it wants, next to what you typed: an empty
`string` reads `(needs a value)`, and a `number` or an `integer` reads
`(needs a number)` or `(needs a whole number)` until it parses. The rule is the
one your `default` is validated against, not Rust's — `inf` and `NaN` parse as
floats and are refused here for the reason `default = inf` is refused at load:
`--share inf` is not a number, and the text reaches gcloud verbatim. A refusal
takes nothing away, so you correct the one field with everything else you typed
still in place.

What you type is what is sent. A declared `default = 0.50` is stored as `0.5`,
but a `0.50` you type stays `0.50` all the way to the confirmation — that line
shows the exact characters that will run, and re-rendering your value would put
text on it you never entered. An `integer` accepts a negative, and a negative in
a leading position is refused for the reason any substituted value beginning
with `-` is: gcloud would read it as a flag.

`Enter` does not run the command either. It closes the prompt and opens the
ordinary confirmation, showing the exact argv with your values substituted, and
your `label` — rendered with them — as its heading. The line you approve is the
line that runs, and that confirmation stays the only thing that starts a
process. A `dangerous` plugin still asks you to type `1` there. `--read-only`,
when it refuses a plugin, refuses as you choose it, before the prompt opens, so
you are never asked to type values for a command that was never going to run.

Until the values exist there is nothing to render a label naming one against, so
a plugin whose `label` names an input is captioned by its `id` in the palette
and on the prompt, and by the label you wrote from the confirmation onwards.

**A `bool` sends the text `true` or `false`.** It is a two-state field showing
the text it will send, so `"--async={async}"` becomes `--async=true` or
`--async=false` and the argv has the same number of elements either way. With no
`default` it starts at `false`, the answer that adds nothing, so a field you tab
past cannot opt you into anything.

It is deliberately not a flag's *presence*: making `{async}` disappear from the
argv when false would make the command's length depend on an answer, and the
confirmation exists to show you the exact line that will run. If you want a flag
that is there or not there, write two plugins, or use gcloud's own
`--flag=false` form where it takes one.

An input no template names withholds nothing and is not a field on the prompt
either — that plugin's command line and caption are complete without it.

#### What you see, end to end

Take the `describe` plugin above, with the Compute Engine list in view, the row
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

#### When a plugin is not offered

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
[Read-only mode](demo-and-read-only.md#read-only-mode) for which plugins fall under it.

#### Plugins are read once, at startup

Editing `plugins.toml` while nuvem is running changes nothing until you restart
it. **Reload configuration** notices the file has changed and says so —
`restart required for: plugins` — rather than swapping the plugin set under a
palette or a confirmation that is already pointing into it.

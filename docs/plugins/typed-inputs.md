---
title: Typed inputs
description: Ask for values the row cannot supply before a plugin runs.
sidebar:
  order: 3
---

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

---
title: Typed inputs
description: Ask for values the row cannot supply before a plugin runs.
sidebar:
  order: 3
---

Some commands need a value the row cannot supply: the machine type to resize
*to*, a new disk size, a reason to record. A plugin declares those as `inputs`.
Each input's `name` becomes a `{field}` that the plugin's own `label` and
`args` can use, beside the row's fields:

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

`integer` is a type of its own, not a flag on `number`, because the value
reaches gcloud verbatim. `number` accepts `0.5`. `integer` accepts `200` and
rejects `1.5` at load. Only a value of the type you declared can be sent.

An input cannot be named after one of its scope's own fields, and those names
depend on the scope. On Compute Engine, `{zone}` and `{machine_type}` are the
row's own. An input called `zone` is thus rejected at load, so neither of the
two silently wins. An input for the machine type to resize *to* needs another
name, such as `target_type`. On persistent disks neither name is taken, so a
disks plugin can call its input `size`. The example uses `new_size` because it
reads more clearly.

nuvem prompts for every input type. If a plugin's `args` or `label` name
inputs, choosing it opens a prompt first. The prompt has one field for each
input the plugin's own templates name, in declaration order:

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

You type into a `string`, `number` or `integer` field. You step through a
`dropdown` or a `bool` with `Left` and `Right`. `Tab` and `Shift-Tab` move
between fields, and the `>` marks the one your keystrokes reach. `Enter` does
nothing until every field holds something it can send. The modal names only
the keys that do something right now, so it does not offer `Enter` until
`Enter` works. `Esc` closes the prompt and runs nothing. There is no log
entry, because nothing was issued.

A field that is not ready says what it wants, next to what you typed. An empty
`string` reads `(needs a value)`. A `number` or an `integer` reads
`(needs a number)` or `(needs a whole number)` until it parses. The rule is the
one your `default` is validated against, not Rust's number parser. Rust parses
`inf` and `NaN` as floats. The prompt refuses them, for the same reason
`default = inf` is refused at load: `--share inf` is not a number, and the
text reaches gcloud verbatim. A refusal takes nothing away. You correct the
one field, and everything else you typed stays in place.

What you type is what is sent. A declared `default = 0.50` is stored as `0.5`,
but a `0.50` you type stays `0.50` all the way to the confirmation. That line
shows the exact characters that will run, so nuvem never rewrites a value you
entered. An `integer` accepts a negative. A negative in a leading position is
refused, for the same reason as any substituted value that begins with `-`:
gcloud reads it as a flag.

`Enter` on the prompt does not run the command either. It closes the prompt
and opens the ordinary confirmation. The confirmation shows the exact argv
with your values substituted, under your `label` rendered with them. The line
you approve is the line that runs, and the confirmation is still the only
thing that starts a process. A `dangerous` plugin still asks you to type `1`
there. If `--read-only` refuses a plugin, it refuses as you choose it, before
the prompt opens. You are never asked to type values for a command that
cannot run.

A label that names an input cannot render until the values exist. The palette
and the prompt thus caption such a plugin by its `id`. From the confirmation
onwards, the caption is the label you wrote.

A `bool` sends the text `true` or `false`. It is a two-state field that shows
the text it sends, so `"--async={async}"` becomes `--async=true` or
`--async=false`, and the argv has the same number of elements either way.
Without a `default`, it starts at `false`, the answer that adds nothing, so a
field you tab past cannot opt you into anything.

A `bool` never adds or removes a flag. If `{async}` disappeared from the argv
when false, the command's length would depend on an answer, and the
confirmation must show you the exact line that runs. For a flag that is there
or not there, write two plugins, or use gcloud's own `--flag=false` form where
the command accepts it.

An input that no template names withholds nothing, and it is not a field on
the prompt either. That plugin's command line and caption are complete without
it.

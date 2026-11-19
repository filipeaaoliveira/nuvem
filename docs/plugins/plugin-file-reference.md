---
title: Plugin file reference
description: Every key plugins.toml accepts, every input type, and the template fields each service offers.
sidebar:
  order: 5
---

## Where it lives

| Platform | Path |
| --- | --- |
| Linux | `~/.config/nuvem/plugins.toml` (or `$XDG_CONFIG_HOME/nuvem/plugins.toml`) |
| macOS | `~/Library/Application Support/nuvem/plugins.toml` |

To read a different file, pass `--plugins <path>`. nuvem never looks for the
plugin file beside a file named by `--config`. `nuvem info` prints the path it
read and how many plugins loaded.

nuvem loads no plugins from a file that any user on the machine can write
(`chmod o-w plugins.toml` fixes it), from something that is not an ordinary
file, or from a file over 1 MiB.

## Example

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

## File layout

| key | required | meaning |
| --- | --- | --- |
| `version` | no (`1`) | schema version. Any value other than `1` rejects the whole file. |
| `plugins` | no | the array of plugins, written `[[plugins]]` once per plugin. `[plugins]` with single brackets is not an array and is rejected. |

Any other top-level key, such as `[[plugin]]`, is reported as a typo.

## Plugin fields

| field | required | meaning |
| --- | --- | --- |
| `id` | yes | unique within a scope; names the plugin in warnings |
| `scope` | yes | one service, from the same vocabulary `[aliases]` uses |
| `key` | yes | a single printable, non-space character, pressed after `,` to run this plugin; unique within a scope |
| `label` | yes | palette entry and confirmation heading; templated |
| `args` | yes | one string per argv element, each templated independently. These are the elements gcloud receives. The program is not one of them. |
| `dangerous` | no (`false`) | adds a typed acknowledgement to the confirmation |
| `inputs` | no (none) | values you supply when the plugin runs; see [Typed inputs](typed-inputs.md) |

`args` is an array of strings, never one string. Splitting a string needs a
shell parser, and there is no shell anywhere on this path. Each element is
templated independently and becomes exactly one argv element. The program is
not part of `args`: it is always `gcloud`. nuvem adds
`--configuration <active-config>` in front of your `args`.

Two plugins in one scope cannot share an `id` or a `key`. The same key in two
different scopes is fine, because only one scope is in view at a time.

## Input fields

Each `[[plugins.inputs]]` table declares one value the plugin asks for. See
[Typed inputs](typed-inputs.md) for how the prompt behaves.

| field | required | meaning |
| --- | --- | --- |
| `name` | yes | the `{field}` this input fills: a letter or `_`, then letters, digits or `_`. It cannot repeat a field of the plugin's scope. |
| `type` | yes | one of the input types below |
| `choices` | for `dropdown` | non-empty list of strings; only a dropdown takes it |
| `default` | no | must match `type`, and be one of `choices` for a dropdown |

## Input types

| type | the prompt shows | accepts |
| --- | --- | --- |
| `string` | a text field | any non-empty text |
| `number` | a text field | a finite number, such as `0.5` or `200` |
| `integer` | a text field | a whole number, such as `200` or `-3` |
| `bool` | a two-state choice, changed with `←`/`→` | sends the text `true` or `false`; starts at `false` without a `default` |
| `dropdown` | a choice from `choices`, changed with `←`/`→` | one of `choices` |

## Template fields

`label` and every `args` element can contain `{field}` placeholders. The
fields that exist depend on `scope`. The list is the same one
[Jump rules](../configuration/jump-rules.md) use, plus the names of the
plugin's own inputs. nuvem rejects a field the scope does not have when the
file loads, with a warning that names it. For example, `{zone}` under
`scope = "cloudsql"` is rejected, because Cloud SQL rows have a region, not a
zone.

A template renders whatever the row holds, so the flag you pair it with must
be right for every row in that service. Persistent disks are the case to watch.
`compute disks list` returns zonal and regional disks in one table, so a disks
plugin's `{location}` is sometimes a zone and sometimes a region. If you pair
it with a fixed `--zone`, the plugin fails on every regional disk.

## Problems

nuvem checks each plugin on its own. It drops a plugin with a problem, with a
warning that names its index, and the others still load. A problem in the file
itself (unreadable, not TOML, an unknown `version`) loads no plugins, with one
warning. Your `config.toml` is unaffected either way. See
[Config problems](../configuration/config-problems.md).

Plugins are read once, at startup. To pick up a change, restart nuvem.

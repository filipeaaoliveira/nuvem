---
title: Config problems
description: What happens when config.toml or plugins.toml has a mistake, and where the warnings appear.
sidebar:
  order: 10
---

A bad config file never crashes nuvem. Each problem becomes a warning, and
the setting it touches falls back to a default:

- If the file cannot be parsed, or the path cannot be read, every field falls
  back to its default. nuvem reports the failure as a warning.
- If the `version` is unknown or unsupported, every field falls back to its
  default, with a warning.
- If an enum value is invalid (`general.theme`, `general.default_service`),
  that field falls back to its default, with a warning. Other fields still
  load normally.
- If the `[hotkeys]` set is invalid (unknown action, bad key spec, a reserved
  key, a collision, or an override that makes `quit`/`help` unreachable), the
  *entire* hotkey set falls back to the built-in defaults, with a warning.
- If a single `[aliases]` entry is invalid, only that alias is dropped, with a
  warning. The rest of the table is kept.
- If a single `[favorites]` entry is invalid, only that entry is dropped, with
  a warning. Its valid siblings survive, the same as `[aliases]`. A
  `projects`, `regions`, or `zones` key whose value is not an array drops that
  whole key instead, with a warning, and leaves the other keys untouched. A
  `[favorites]` section that is not a table at all is dropped in its entirety,
  with one warning.
- If there is any problem inside one `[columns.<service>]` block, that
  service's *entire* block is dropped, with a warning. The service falls back
  to its built-in columns and sort. Every other service's block still applies.
  A block that names a service that does not exist is skipped without
  affecting any service.
- If two or more `[columns.*]` keys name the same service (`[columns.gce]` and
  `[columns.compute]` are both GCE instances), the alphabetically last key
  decides on its own. Blocks are read in key order, not file order, so the
  one you wrote second does not necessarily win.
- The service reports one warning that names the deciding key and every key
  it displaced. If a displaced key is itself invalid, that same warning names
  its problem, so you learn about the typo before you delete the deciding
  block. The warning tells you when the block you edited is not the one in
  use.
- If the deciding key's block is invalid, every block that names that service
  is discarded. The service falls back to its built-in columns and sort, and
  the one warning names the problem and the blocks discarded with it.
- If a single `[theme.overrides]` entry is invalid, only that entry is
  dropped, with a warning. Its valid siblings survive. A `[theme]` that is not
  a table, an unknown key inside it (the base theme is `[general].theme`), or
  an `overrides` value that is not a table drops that much and nothing more.
  See [Frame overrides](themes.md#frame-overrides).
- If there is any problem inside one `[[jumps]]` entry, only that rule is
  dropped, with a warning that names its index. The other rules still apply.
  A `[jumps]` written with single brackets is not an array of rules. The whole
  section is dropped with one warning and no jump is configured. The rest of
  the file still loads.

The separate plugin file fails the same way, on its own. nuvem loads it
separately, so no problem in the following list costs you your
`config.toml`:

- If the plugin file is absent, there are no plugins and no warning. This is
  the normal case.
- If the plugin file is unreadable or cannot be parsed, there are no plugins,
  and one warning names the path and line.
- If the plugin-file `version` is unknown, there are no plugins, with a
  warning. nuvem rejects every entry whole instead of reading it under rules
  written for another version.
- A `[plugins]` written with single brackets is not an array of plugins, so
  the whole section is dropped with one warning.
- If there is any problem inside one `[[plugins]]` entry, only that plugin is
  dropped, with a warning that names its index. The other plugins still load.
- Two plugins in the same scope that share an `id` or a `key` count as such a
  problem. The later one is dropped, and the warning names the plugin it
  clashed with. The same key in two *different* scopes is fine, because the
  two are never offered at once.
- A problem inside one of a plugin's `[[plugins.inputs]]` tables also drops
  that plugin. Such a problem is an unknown `type`, a `dropdown` with no
  `choices`, or a `default` that contradicts its own `type` or sits outside
  its own `choices`. A `name` that is already a field of the plugin's scope,
  or an unknown key inside the input table, is one too. The warning names the
  input by index and by name, and the other plugins still load.

Startup warnings appear on stderr and in `nuvem info`. The one exception is
the deferred `[theme.overrides]` check. It can only run once the configuration
list has loaded, so its warnings join the help list while nuvem runs, and
never reach stderr.

The help overlay (`?`) lists every warning under `Config problems`, so you see
all of a misconfigured file's problems, not only the first. The status line
summarizes them. It quotes a lone problem whole (`config warning: …`). It
counts several and names the key that opens the list
(`config: 3 problems (press ? for details)`). That key is your `help` key,
rebound or not. A reload says the same thing in its own wording and rewrites
the list, so fixing the file empties the section and leaves no stale advice
in it.

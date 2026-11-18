---
title: Config problems
description: What happens when config.toml or plugins.toml has a mistake, and where the warnings appear.
sidebar:
  order: 10
---

A bad config file never crashes nuvem — problems degrade to a warning and a
default value:

- unparseable file, or an unreadable path → all fields fall back to defaults,
  with the failure reported as a warning;
- unknown/unsupported `version` → all fields fall back to defaults, with a
  warning;
- an invalid enum value (`general.theme`, `general.default_service`) → that
  field falls back to its default, with a warning; other fields still load
  normally;
- an invalid `[hotkeys]` set (unknown action, bad key spec, a reserved key, a
  collision, or an override that makes `quit`/`help` unreachable) → the
  *entire* hotkey set falls back to the built-in defaults, with a warning;
- an invalid single `[aliases]` entry → only that alias is dropped, with a
  warning; the rest of the table is kept;
- an invalid single `[favorites]` entry → only that entry is dropped, with a
  warning; its valid siblings survive, the same as `[aliases]`. A `projects`,
  `regions`, or `zones` key whose value is not an array drops that whole key
  instead, with a warning, leaving the other keys untouched; a `[favorites]`
  section that is not a table at all is dropped in its entirety, with one
  warning;
- any problem inside one `[columns.<service>]` block → that service's *entire*
  block is dropped and the service falls back to its built-in columns and sort,
  with a warning; every other service's block is still applied. A block naming
  a service that does not exist is skipped without affecting any service;
- two or more `[columns.*]` keys naming the same service (`[columns.gce]` and
  `[columns.compute]` are both Compute Engine) → the alphabetically last key
  decides on its own, and the service reports one warning naming every key it
  displaced as well as the winner; a displaced key that is itself invalid has
  its own problem named in that same warning, so deleting the winning block is
  not how you first find out about the typo under it. If the deciding key's
  block is invalid, every block naming that service is discarded and the
  service falls back to its built-in columns and sort, with that one warning
  naming the problem and the blocks discarded with it. Blocks are read in key
  order, not file order, so the one you wrote second does not necessarily win —
  which is why the loser is reported instead of silently changing nothing;
- an invalid single `[theme.overrides]` entry → only that entry is dropped,
  with a warning; its valid siblings survive. A `[theme]` that is not a table,
  an unknown key inside it (the base theme is `[general].theme`), or an
  `overrides` value that is not a table drops that much and nothing more — see
  [Frame overrides](themes.md#frame-overrides);
- any problem inside one `[[jumps]]` entry → only that rule is dropped, with a
  warning naming its index; the other rules still apply. A `[jumps]` written
  with single brackets is not an array of rules, so the whole section is
  dropped with one warning and no jump is configured — the rest of the file
  still loads.

The separate plugin file degrades the same way, and independently — it is a
separate load, so nothing below costs you your `config.toml`:

- the plugin file is absent → no plugins, no warning; this is the normal case;
- the plugin file is unreadable or unparseable → no plugins, with one warning
  naming the path and line; your `config.toml` is unaffected, because the two
  are separate loads;
- an unknown plugin-file `version` → no plugins, with a warning; entries are
  rejected whole rather than read under rules that may not be theirs;
- a `[plugins]` written with single brackets is not an array of plugins, so the
  whole section is dropped with one warning;
- any problem inside one `[[plugins]]` entry → only that plugin is dropped,
  with a warning naming its index; the other plugins still load. Two plugins in
  the same scope sharing an `id` or a `key` is such a problem: the later one is
  dropped and the warning names the plugin it clashed with. The same key in two
  *different* scopes is fine — the two are never offered at once. A problem
  inside one of a plugin's `[[plugins.inputs]]` tables is such a problem too:
  an unknown `type`, a `dropdown` with no `choices`, a `default` that
  contradicts its own `type` or sits outside its own `choices`, a `name` that is
  already a field of the plugin's scope, or an unknown key inside the input
  table. The warning names the input by index and by name, that one plugin is
  dropped, and the rest still load.

Startup warnings are shown on stderr (and via `nuvem info`) — except the
deferred `[theme.overrides]` check, which can only run once the configuration
list has loaded and so joins the list in the running app rather than on
stderr. Every one of them is listed in the help overlay (`?`) under
`Config problems`, so a
misconfigured file is always visible in full and not just by its first
complaint. The status line summarizes: a lone problem is quoted whole
(`config warning: …`), and several are counted with the key that opens the list
(`config: 3 problems (press ? for details)` — it names your `help` key, rebound
or not). A reload says the same thing in its own wording and rewrites the list,
so fixing the file empties the section rather than leaving stale advice in it.

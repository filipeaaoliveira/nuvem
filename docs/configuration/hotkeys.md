---
title: Hotkeys
description: Rebind the seven global navigation keys with the [hotkeys] table.
sidebar:
  order: 3
---

You can rebind seven global keys in the `[hotkeys]` table. Every other key is
fixed.

```toml
[hotkeys]
home = "H"
refresh = "ctrl-r"
```

Each value is one key:

- A single character, case preserved: `"H"` is `Shift-H`, `"h"` is `h`.
- `ctrl-` and one character: `"ctrl-r"`.

nuvem recognizes names such as `"enter"` or `"esc"` only so that validation
can refuse them.

| action             | default key |
|--------------------|-------------|
| `home`             | `h`         |
| `configurations`   | `c`         |
| `resources`        | `g`         |
| `refresh`          | `r`         |
| `help`             | `?`         |
| `quit`             | `q`         |
| `command_palette`  | `:`         |

## Keys you cannot use

Validation refuses these as a hotkey and says why:

| Key | Why |
| --- | --- |
| `Esc`, `Enter`, `Tab`, `Shift-Tab`, `Backspace` | Only single characters can be rebound. These keep their meaning in every form, dialog and filter. |
| `Ctrl-C`, `Ctrl-X` | nuvem handles them before any hotkey, so a binding would never fire. `Ctrl-C` quits. `Ctrl-X` cancels running plugins. |
| `Ctrl-F`, `Ctrl-S`, `Ctrl-T` | A binding would take over FinOps, saving logs, or Tunnels with no warning. |

## Keys to avoid

Validation accepts these keys. A hotkey bound to one of them takes the key
away from what it did before, without a warning:

- `l` (open Logging) and `Y` (copy the last diagnostic).
- A key the current view uses, such as `v` (show or hide the sidebar in
  Resources). The hotkey wins in that view.

## When a hotkey set is invalid

nuvem checks the whole `[hotkeys]` table at startup and on
[reload](reload.md): unknown action names, keys it cannot parse, the keys
above, two actions on one key, and a set that leaves no way to reach `quit` or
`help`. If it finds any problem, it puts every hotkey back on its default key
and reports a warning. See [Config problems](config-problems.md).

The footer and the help overlay always show the keys in effect.

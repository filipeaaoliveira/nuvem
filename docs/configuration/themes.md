---
title: Themes and frame colors
description: Pick a built-in theme and recolor the frame for a gcloud configuration or project.
sidebar:
  order: 2
---

Pick a theme with `theme` in the `[general]` table:

```toml
[general]
theme = "high-contrast"
```

| Theme | What it looks like |
| --- | --- |
| `default` | The standard color scheme. |
| `no-color` | No foreground or background colors. Status reads from the `[OK]`/`[ERR]` text markers, and emphasis from bold, underline and reverse video. Use it for accessibility or a terminal without color. |
| `high-contrast` | Bright, clearly distinct colors with bold emphasis, for low vision or glare. |

The name is case-insensitive, and `_` works in place of `-`: `No_Color` and
`nocolor` both select `no-color`. An unknown name keeps `default` and reports a
warning. `nuvem info` prints the active theme. [Reload configuration](reload.md)
applies a new theme without a restart.

## Frame overrides

`[theme.overrides]` and `[theme.project_overrides]` recolor the pane borders
while a dangerous context is active, so it is unmistakable at a glance —
`prod = "red"` in the [config file example](config-file.md#full-example) is the whole feature.

The two tables differ in what they match, and the difference matters:

- **`[theme.overrides]`** keys on the **gcloud configuration name**, matched
  verbatim against what `gcloud` reports. It is only as reliable as your
  naming convention: if the configuration that reaches production is called
  `default`, `work`, or `client`, an override written for `prod` never fires.
- **`[theme.project_overrides]`** keys on the **project id**, the same
  vocabulary `[favorites].projects` uses. Every configuration pointing at that
  project gets the frame regardless of what it is called, and a project id is
  the same string for everyone who can reach it, where a configuration name is
  a local, renameable, per-machine alias.

Prefer the project table for the guardrail itself, and the configuration table
for colouring one particular way in. When both match the active context, the
**configuration name wins**: several configurations can point at one project,
so naming one of them is the more specific statement, and the project override
is the catch-all for every configuration you never named.

Each value is a color: a name (`red`, `light-red`), a hex triplet
(`#ff0000`), or a 256-color index (`9`).

Project overrides live in their own table rather than nested inside
`[theme.overrides]`: a gcloud configuration may legitimately be *named*
`projects`, and reserving that key inside a table of configuration names would
leave that configuration uncolourable. Writing `[theme.overrides.projects]`
anyway is reported, with the table that works.

Patterns are deliberately not supported. `*-prod` would reintroduce this
feature's worst failure one level up: a pattern matching nothing is a silent
no-op, and unlike an exact id it cannot be checked against the projects your
configurations actually reach.

The override replaces the *hue* of both border roles — focused and unfocused
alike, so the frame does not fall back to the base accent as you move between
panes. Everything else (titles, tabs, selection, status colors) stays on the
base theme picked by `[general].theme`, and bold/underline emphasis survives,
so an override on top of `no-color` or `high-contrast` does not undo that
theme's non-color cues.

Problems are reported as config warnings rather than being fatal, and each bad
entry is dropped on its own:

- a color nuvem cannot parse, a non-string value, or a blank configuration
  name or project id drops that entry and keeps its siblings.
- an override naming a configuration that does not exist, or a project id no
  configuration targets, is reported once the configuration list loads. Neither
  can be checked when the file is read, because the list comes from `gcloud`
  later in startup — and a silent no-op is the worst failure this feature has,
  since the frame then looks ordinary while you believe it is guarded.

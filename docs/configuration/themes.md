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
while a configuration or project you chose to mark is active, such as
production, so you can see it at a glance. The whole
feature is one line, such as `prod = "red"` in the
[config file example](config-file.md#full-example).

The two tables match different things:

- `[theme.overrides]` keys on the gcloud configuration name, matched verbatim
  against what `gcloud` reports. It is only as reliable as your naming
  convention. If the configuration that reaches production is called
  `default`, `work`, or `client`, an override written for `prod` never fires.
- `[theme.project_overrides]` keys on the project ID, the same vocabulary
  `[favorites].projects` uses. Every configuration that points at that project
  gets the frame, whatever it is called. A project ID is the same string for
  everyone who can reach it. A configuration name is a local, renameable alias
  on one machine.

We recommend `[theme.project_overrides]` to mark a project in every
configuration that reaches it, and `[theme.overrides]` to color one
configuration differently. If both match the active context, the
configuration name wins. Several configurations can point at one project, so
naming one of them is the more specific statement. The project override
catches every configuration you never named.

Each value is a color: a name (`red`, `light-red`), a hex triplet
(`#ff0000`), or a 256-color index (`9`).

Project overrides have their own table, outside `[theme.overrides]`, because
a gcloud configuration can be named `projects`. A reserved `projects` key in a
table of configuration names would leave that configuration impossible to
color. If you write `[theme.overrides.projects]` anyway, nuvem reports it and
names the table that works.

nuvem does not support patterns such as `*-prod`. A pattern that matches
nothing is a silent no-op. Unlike an exact ID, it cannot be checked against
the projects your configurations reach.

The override replaces the *hue* of both border roles, focused and unfocused,
so the frame does not fall back to the base accent as you move between panes.
Everything else (titles, tabs, selection, status colors) stays on the base
theme picked by `[general].theme`. Bold and underline emphasis survive, so an
override on top of `no-color` or `high-contrast` keeps that theme's cues that
do not depend on color.

nuvem reports problems as config warnings, not fatal errors, and drops each
bad entry on its own:

- A color nuvem cannot parse, a value that is not a string, or a blank
  configuration name or project ID drops that entry and keeps its siblings.
- An override that names a configuration that does not exist, or a project ID
  that no configuration targets, is reported once the configuration list
  loads. nuvem cannot check either when it reads the file, because the list
  comes from `gcloud` later in startup. nuvem warns here because an override
  that matches nothing leaves the frame ordinary while you believe it is
  marked.

---
title: Config file
description: Where config.toml lives, how settings are layered, and every key it accepts.
sidebar:
  order: 1
---

nuvem reads an optional, versioned TOML configuration file. It holds your
presentation and navigation preferences: refresh cadence, the startup
service, sidebar visibility, theme, extra service aliases, rebindable
hotkeys, each service's table columns and default sort, and the
related-resource jumps `J` offers. Nothing in this file can run a command.

Executable declarations live in a separate file,
[`plugins.toml`](../plugins/overview.md). nuvem reads it only from your
platform config directory or from an explicit `--plugins <path>`. It never
reads it from the directory a `--config` path points at. A `config.toml` that
you were handed, synced, or checked into a repository thus cannot bring
commands with it.

## Where it lives

| Platform | Path |
| --- | --- |
| Linux | `~/.config/nuvem/config.toml` (or `$XDG_CONFIG_HOME/nuvem/config.toml`) |
| macOS | `~/Library/Application Support/nuvem/config.toml` |

To read a different file, pass `--config <path>`. The file is optional. If
there is no file, every setting keeps its built-in default.

nuvem refuses a file that is not an ordinary file (a named pipe, a directory)
or is larger than 1 MiB, and reports why. nuvem follows a symbolic link, so
dotfile managers work.

## Precedence

Lowest to highest: built-in defaults, then the config file, then command-line
flags. A flag always wins. Two settings exist in both places:

| Config key | Flag | How they combine |
| --- | --- | --- |
| `general.poll_interval_secs` | `--poll-interval` | The flag wins. |
| `general.read_only` | `--read-only` | Either one turns read-only on. Neither can turn it off. |

Every other flag has no config key. See [Command line](../reference/cli.md).

## Check what nuvem read

Run `nuvem info` to see the resolved config, plugin, log, and cache paths and
the active theme. It does not start the TUI:

```
nuvem 0.1.0
config:  /home/dev/.config/nuvem/config.toml (loaded)
plugins: /home/dev/.config/nuvem/plugins.toml (2 loaded)
log:     /home/dev/.cache/nuvem/nuvem.log
cache:   /home/dev/.cache/nuvem
theme:   default
```

`config:` reports `loaded`, `not found`, or `error: <message>` for a file that
exists but failed to parse. `plugins:` reports `not found` for a file that is
absent, which is the normal case. For a file that is present, it reports how
many entries loaded. You can thus tell a file nuvem never opened from one
that declares nothing. nuvem also prints any field-level warnings (see
[Config problems](config-problems.md)) to stderr and lists them under
`nuvem info`.

## Full example

```toml
version = 1                     # schema version; missing = 1; unknown = rejected

[general]
poll_interval_secs = 5          # refresh cadence in seconds; 0 disables background refresh
default_service = "gce"         # fallback service for a configuration with no remembered view: "gce" | "gke" (restart-required)
sidebar_visible = true          # service navigator sidebar shown by default
theme = "default"               # "default" | "no-color" | "high-contrast"
read_only = false               # refuse cloud-resource mutations (see Read-only mode);
                                 # OR'd with --read-only, so this can only turn it on

[aliases]                       # extra service aliases: alias -> service id
vms = "gce"
k8s = "gke"

[hotkeys]                       # rebindable normal-mode action -> single key
home = "H"
refresh = "R"

[favorites]                     # scopes to order first in the context picker
projects = ["prod-project"]     # matched against project id
regions  = ["europe-west1"]     # matched against region name
zones    = ["europe-west1-b"]   # matched against zone name

[theme.overrides]               # frame color per gcloud configuration
prod = "red"                    # configuration name -> border color

[theme.project_overrides]       # frame color per project, whatever reaches it
prod-project = "red"            # project id -> border color

[columns.compute]               # per-service table columns and default sort
columns = ["name", "status", "zone", "machine-type"]
sort = "status"                 # must be sortable and present in `columns`
sort_direction = "desc"         # "asc" (default) | "desc"; needs a `sort`

[[jumps]]                       # one related-resource jump `J` offers
from = "compute"                # source service
to = "compute:network"          # destination "<service>:<tab>"
name = "{name}"                 # templated target name; default "{name}"
location = "{zone}"             # templated target zone/region; default: the row's own
label = "Network of {name}"     # picker wording; default "<tab> of <name>"
```

## Keys

The table lists every key the file accepts. `<service>` is a service id from
[Services](../reference/services/overview.md). `<action>` is a hotkey action.

| Key | Type | Default | Allowed values | Details |
| --- | --- | --- | --- | --- |
| `version` | integer | `1` | `1`. Any other value makes nuvem ignore the whole file. | this page |
| `general.poll_interval_secs` | integer | `5` | seconds; `0` turns background refresh off | [First run](../getting-started/first-run.md#background-refresh) |
| `general.default_service` | string | `"gce"` | `gce`, `compute`, `compute_instances`, `gke`, `clusters`, `gke_clusters` | [Remembered last view](remembered-view.md) |
| `general.sidebar_visible` | boolean | `true` | `true`, `false` | [Browse resources](../guides/browse-resources.md) |
| `general.theme` | string | `"default"` | `default`, `no-color`, `high-contrast` | [Themes](themes.md) |
| `general.read_only` | boolean | `false` | `true`, `false` | [Read-only mode](../guides/read-only-mode.md) |
| `aliases.<alias>` | string | none | a service id, alias or label | [Service aliases](aliases.md) |
| `hotkeys.<action>` | string | see [Hotkeys](hotkeys.md) | one character, or `ctrl-<character>` | [Hotkeys](hotkeys.md) |
| `favorites.projects` | list of strings | `[]` | project ids | [Favorites](favorites.md) |
| `favorites.regions` | list of strings | `[]` | region names | [Favorites](favorites.md) |
| `favorites.zones` | list of strings | `[]` | zone names | [Favorites](favorites.md) |
| `theme.overrides.<configuration>` | string | none | a color | [Themes](themes.md#frame-overrides) |
| `theme.project_overrides.<project>` | string | none | a color | [Themes](themes.md#frame-overrides) |
| `columns.<service>.columns` | list of strings | the service's built-in columns | column ids; must include `name` | [Columns and sort](columns-and-sort.md) |
| `columns.<service>.sort` | string | `"name"` | a sortable column id | [Columns and sort](columns-and-sort.md) |
| `columns.<service>.sort_direction` | string | `"asc"` | `asc`, `desc` | [Columns and sort](columns-and-sort.md) |
| `jumps[].from` | string | required | a service id | [Jump rules](jump-rules.md) |
| `jumps[].to` | string | required | `"<service>"` or `"<service>:<tab>"` | [Jump rules](jump-rules.md) |
| `jumps[].name` | string | `"{name}"` | a template | [Jump rules](jump-rules.md) |
| `jumps[].location` | string | the source row's location | a template | [Jump rules](jump-rules.md) |
| `jumps[].label` | string | `"<tab> of <name>"` | a template | [Jump rules](jump-rules.md) |
| `jumps[].select` | string | none | `targets-this-instance` | [Jump rules](jump-rules.md) |
| `plugins` | none | none | Not allowed here. Plugins belong in `plugins.toml`; nuvem warns and ignores them. | [Plugins](../plugins/overview.md) |

nuvem reports any other key as unknown and ignores it. The rest of the file
still applies. A misspelled key such as `poll_interval_sec` is reported the
same way.

`general.default_service`, the starting gcloud configuration and the plugin
file take effect only at startup. Every other key can be applied with
[Reload configuration](reload.md).

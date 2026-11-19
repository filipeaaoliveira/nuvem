---
title: Keys
description: Every key nuvem binds, view by view, with the label the footer shows.
sidebar:
  order: 1
---

nuvem is driven entirely from the keyboard: it has no mouse support. Every key, grouped by the
view it works in. The **Footer label** column is the
text nuvem shows in the footer and in help (`?`), so you can match a row to
what is on screen. Keys written as an uppercase letter need `Shift`: `L` is
`Shift-L`. A key shown as `type` means any printable character, and `j/k` also
works with the arrow keys.

Seven keys can be rebound in the config file; see
[Hotkeys](../configuration/hotkeys.md). The tables show the default keys.

## Global keys

These work in every normal view: Home, Configurations, the Resources list and detail, Logging, Tunnels and FinOps. `Ctrl-C` and `Ctrl-x` also work inside modals, the palette, help and text fields. The other global keys are not available while a modal, the palette or a text field has the keyboard.

<!-- help-context: global -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `:` | Command palette | Open the [command palette](../guides/command-palette.md). Rebindable as `command_palette`. |
| `?` | Help | Show the keys for the current view. Rebindable as `help`. |
| `r` | Refresh | Refresh the current view: reload the service list, the log window, the configurations, FinOps, or Home's health probe. Rebindable as `refresh`. |
| `q` | Quit | Quit nuvem from Home and from list views. In a detail view, `q` goes back instead. Rebindable as `quit`. |
| `Ctrl-C` | Quit | Quit from any view, including modals and text fields. |
| `Y` | Copy diagnostic | Copy the sanitized detail of the last diagnostic to the clipboard. |
| `Ctrl-x` | Cancel running plugin | Cancel every running plugin, from any view, including modals and text fields. |
| `h` | Home | Open [Home](../guides/home-overview.md). Rebindable as `home`. |
| `c` | Configurations | Open [Configurations](../guides/switch-configurations.md). Rebindable as `configurations`. |
| `g` | Resources | Open [Resources](../guides/browse-resources.md). Rebindable as `resources`. |
| `l` | Logging workspace: all project logs (L: selection only) | Open [Logging](../guides/cloud-logging.md) on the project's recent logs. `L` opens logs for the selected resource instead. |
| `Ctrl-t` | Tunnels | Open [Tunnels](../guides/tunnels.md). |
| `Ctrl-f` | FinOps | Open [FinOps](../guides/finops.md). |
| `Alt-←` | Back | Go back through navigation history. |
| `Alt-→` | Forward | Go forward through navigation history. |

## Home

Home has no keys of its own beyond the global keys.

<!-- help-context: Home -->

## Configurations

The list of gcloud configurations. See [Switch gcloud configurations](../guides/switch-configurations.md).

<!-- help-context: ConfigList -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `/` | Filter | Filter the configurations by name, account, project, region or zone. Case-insensitive. |
| `u` | Clear filter | Clear the configuration filter. |
| `a` | Create configuration | Create a gcloud configuration. |
| `e` | Edit active context | Edit the active configuration's project, region and zone. |
| `j/k` | Move selection | Move the selection down or up. The arrow keys do the same. |
| `Enter` | Activate | Activate the selected configuration. A confirmation shows the command first. |

## Configuration filter

While typing a filter in Configurations.

<!-- help-context: ConfigFilter -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `type` | Enter text | Type into the field. |
| `Backspace` | Delete char | Delete the last character. |
| `Enter` | Apply | Apply the text as the filter. |
| `Esc` | Cancel | Drop the draft and keep the filter already applied. |

## Create a configuration

The form `a` opens.

<!-- help-context: ConfigCreate -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `type` | Enter text | Type into the field. |
| `Backspace` | Delete char | Delete the last character. |
| `Tab` | Next field | Move to the next field. `↓` does the same. |
| `Shift-Tab` | Previous field | Move to the previous field. `↑` does the same. |
| `Enter` | Submit | Create the configuration. Only the name is required. |
| `Esc` | Cancel | Cancel and close, changing nothing. |

## Edit the active context

The form `e` opens.

<!-- help-context: ConfigContextEdit -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `Ctrl-Space` | Pick value | Open a picker of projects, regions or zones for the focused field. |
| `type` | Enter text | Type into the field. |
| `Backspace` | Delete char | Delete the last character. |
| `Tab` | Next field | Move to the next field. `↓` does the same. |
| `Shift-Tab` | Previous field | Move to the previous field. `↑` does the same. |
| `Enter` | Submit | Save. A blank field unsets that gcloud property. |
| `Esc` | Cancel | Cancel and close, changing nothing. |

## Context picker

The list `Ctrl-Space` opens on the Project, Region or Zone field.

<!-- help-context: ConfigContextPicker -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `/` | Filter | Filter the picker's rows. |
| `j/k` | Move selection | Move the selection down or up. The arrow keys do the same. |
| `r` | Retry discovery | Ask gcloud for the values again. |
| `Enter` | Select | Copy the selected value into the field. |
| `Esc` | Cancel | Close the picker and go back to the form. |

## Context picker filter

While typing a filter in the context picker.

<!-- help-context: ConfigContextPickerFilter -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `type` | Enter text | Type into the field. |
| `Backspace` | Delete char | Delete the last character. |
| `Enter` | Apply | Apply the text as the filter. |
| `Esc` | Cancel | Drop the draft and keep the filter already applied. |

## Resource list

A service's list in Resources.

<!-- help-context: ResourceList -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `v` | Toggle sidebar | Show or hide the service sidebar. |
| `Tab` | Focus sidebar | Move focus between the service sidebar and the list or detail pane. |
| `/` | Filter | Edit the filter for this list. See [Filter a list](../guides/filter-a-list.md). |
| `s` | Sort column | Sort by the selected column. Press again to reverse the direction. |
| `w` | Wide columns | Switch between narrow columns (drop columns to fit) and wide columns (keep every column). |
| `←/→` | Select column | Move the selected column, which `s` sorts by. |
| `j/k` | Move selection | Move the selection down or up. The arrow keys do the same. |
| `Enter` | Open | Open the selected row: a detail pane, the Cloud Storage object browser, or the next BigQuery level. |
| `Space` | Mark | Mark or unmark the selected row (GCE instances and Cloud SQL only). See [Batch actions](../guides/batch-actions.md). |
| `V` | Range mark | Mark every row from the last marked row to the selected row (`Shift-V`). |
| `\` | Clear marks | Clear every mark. |
| `U` | Unmark visible | Unmark only the rows the filter shows (`Shift-U`). |
| `t` | Start tunnel | On a GCE or Cloud SQL instance, start a tunnel to it. A preview shows the command first. See [Tunnels](../guides/tunnels.md). |
| `i` | Bucket metadata | In the Cloud Storage bucket list, open the selected bucket's metadata. |
| `y` | Copy gs:// URI | In Cloud Storage, copy the selected bucket's, prefix's or object's `gs://` URI. |
| `n` | Next page | In the Cloud Storage object browser, load the next page of objects. |
| `L` | Logs for the selected resource (l: all project logs) | Open Cloud Logging scoped to the selected resource (`Shift-L`). |
| `J` | Related-resource jumps | Open the [related-resource jump](../guides/related-resource-jumps.md) picker (`Shift-J`). |
| `,` | Run plugin | Arm the plugin prefix: the next key runs the [plugin](../plugins/overview.md) bound to it. |
| `Backspace/-` | Up one level | Go up one level: one prefix in the Cloud Storage object browser, one level in BigQuery. |
| `Esc/Alt-←` | Back | Go back to the previous view in navigation history. |

## Service sidebar

The service navigator on the left of Resources, when it has focus.

<!-- help-context: ResourceNavigator -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `v` | Toggle sidebar | Show or hide the service sidebar. |
| `Tab` | Focus sidebar | Move focus between the service sidebar and the list or detail pane. |
| `/` | Filter services | Search the services by name, category or alias. |
| `j/k` | Move selection | Move through the matching services. |
| `Enter` | Open service | Open the selected service. |
| `Esc/Alt-←` | Back | Go back to the previous view in navigation history. |

## Resource detail

The tabbed detail pane `Enter` opens.

<!-- help-context: ResourceDetail -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `v` | Toggle sidebar | Show or hide the service sidebar. |
| `Tab` | Focus sidebar | Move focus between the service sidebar and the list or detail pane. |
| `j/k` | Move selection | Move the selection down or up. The arrow keys do the same. |
| `[` | Previous tab | Show the previous detail tab. |
| `]` | Next tab | Show the next detail tab. |
| `n` | Next field | Select the next copyable field, wrapping at the end. |
| `p` | Previous field | Select the previous copyable field, wrapping at the start. |
| `PgUp/PgDn` | Scroll / release field | Scroll the detail pane a page. Paging also deselects the selected field. |
| `Enter` | Open revision | On a Cloud Run service's Revisions tab, open the selected revision. |
| `y` | Copy field / document | Copy the selected field. With no field selected, copy the whole document on a JSON tab, or the `gs://` URI of the bucket or object. |
| `L` | Logs for the selected resource (l: all project logs) | Open Cloud Logging scoped to the selected resource (`Shift-L`). |
| `J` | Related-resource jumps | Open the [related-resource jump](../guides/related-resource-jumps.md) picker (`Shift-J`). |
| `,` | Run plugin | Arm the plugin prefix: the next key runs the [plugin](../plugins/overview.md) bound to it. |
| `q/Esc/Alt-←` | Back | Close the detail pane and return to the list. `q` follows a `quit` rebinding. |

## BigQuery drill-down

The tables of a dataset. `Enter` opens a table's column schema.

<!-- help-context: ResourceDrilldown -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `v` | Toggle sidebar | Show or hide the service sidebar. |
| `Tab` | Focus sidebar | Move focus between the service sidebar and the list or detail pane. |
| `/` | Filter | Edit the filter for this list. See [Filter a list](../guides/filter-a-list.md). |
| `w` | Wide columns | Switch between narrow columns (drop columns to fit) and wide columns (keep every column). |
| `j/k` | Move selection | Move the selection down or up. The arrow keys do the same. |
| `Enter` | Open | Open the selected row: a detail pane, the Cloud Storage object browser, or the next BigQuery level. |
| `Space` | Mark | Mark or unmark the selected row (GCE instances and Cloud SQL only). See [Batch actions](../guides/batch-actions.md). |
| `V` | Range mark | Mark every row from the last marked row to the selected row (`Shift-V`). |
| `\` | Clear marks | Clear every mark. |
| `U` | Unmark visible | Unmark only the rows the filter shows (`Shift-U`). |
| `t` | Start tunnel | On a GCE or Cloud SQL instance, start a tunnel to it. A preview shows the command first. See [Tunnels](../guides/tunnels.md). |
| `i` | Bucket metadata | In the Cloud Storage bucket list, open the selected bucket's metadata. |
| `y` | Copy gs:// URI | In Cloud Storage, copy the selected bucket's, prefix's or object's `gs://` URI. |
| `n` | Next page | In the Cloud Storage object browser, load the next page of objects. |
| `L` | Logs for the selected resource (l: all project logs) | Open Cloud Logging scoped to the selected resource (`Shift-L`). |
| `J` | Related-resource jumps | Open the [related-resource jump](../guides/related-resource-jumps.md) picker (`Shift-J`). |
| `,` | Run plugin | Arm the plugin prefix: the next key runs the [plugin](../plugins/overview.md) bound to it. |
| `Backspace/-` | Up one level | Go up one level: one prefix in the Cloud Storage object browser, one level in BigQuery. |
| `Esc` | Leave drill-down | Go up one BigQuery level: from a table's schema to the tables, from the tables to the datasets. |

## BigQuery column schema

The bottom BigQuery level, where `Enter` opens nothing.

<!-- help-context: ResourceDrilldownLeaf -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `v` | Toggle sidebar | Show or hide the service sidebar. |
| `Tab` | Focus sidebar | Move focus between the service sidebar and the list or detail pane. |
| `/` | Filter | Edit the filter for this list. See [Filter a list](../guides/filter-a-list.md). |
| `w` | Wide columns | Switch between narrow columns (drop columns to fit) and wide columns (keep every column). |
| `j/k` | Move selection | Move the selection down or up. The arrow keys do the same. |
| `Space` | Mark | Mark or unmark the selected row (GCE instances and Cloud SQL only). See [Batch actions](../guides/batch-actions.md). |
| `V` | Range mark | Mark every row from the last marked row to the selected row (`Shift-V`). |
| `\` | Clear marks | Clear every mark. |
| `U` | Unmark visible | Unmark only the rows the filter shows (`Shift-U`). |
| `t` | Start tunnel | On a GCE or Cloud SQL instance, start a tunnel to it. A preview shows the command first. See [Tunnels](../guides/tunnels.md). |
| `i` | Bucket metadata | In the Cloud Storage bucket list, open the selected bucket's metadata. |
| `y` | Copy gs:// URI | In Cloud Storage, copy the selected bucket's, prefix's or object's `gs://` URI. |
| `n` | Next page | In the Cloud Storage object browser, load the next page of objects. |
| `L` | Logs for the selected resource (l: all project logs) | Open Cloud Logging scoped to the selected resource (`Shift-L`). |
| `J` | Related-resource jumps | Open the [related-resource jump](../guides/related-resource-jumps.md) picker (`Shift-J`). |
| `,` | Run plugin | Arm the plugin prefix: the next key runs the [plugin](../plugins/overview.md) bound to it. |
| `Backspace/-` | Up one level | Go up one level: one prefix in the Cloud Storage object browser, one level in BigQuery. |
| `Esc` | Leave drill-down | Go up one BigQuery level: from a table's schema to the tables, from the tables to the datasets. |

## Filters

While typing a filter: a resource filter, a sidebar service search, or a Logging search.

<!-- help-context: ResourceFilter ServiceFilter LoggingFilter -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `type` | Enter text | Type into the field. |
| `Backspace` | Delete char | Delete the last character. |
| `Enter` | Apply | Apply the text as the filter. |
| `Esc` | Cancel | Drop the draft and keep the filter already applied. |

## Logging list

The list of log entries. See [Cloud Logging](../guides/cloud-logging.md).

<!-- help-context: LoggingList -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `j/k` | Move selection | Move the selection down or up. The arrow keys do the same. |
| `Enter` | Open entry | Open the selected entry's payload and labels. |
| `t` | Pause/resume tail | Pause or resume the live tail. |
| `T` | Time range | Cycle the since window, shortest to longest. |
| `0-6` | Since bucket | Pick a since window directly: `0` 1m, `1` 5m, `2` 15m, `3` 30m, `4` 1h, `5` 6h, `6` 24h. |
| `s` | Raise severity floor | Raise the severity floor one step. |
| `S` | Lower severity floor | Lower the severity floor one step. |
| `w` | Wrap | Wrap or truncate long lines. |
| `/` | Filter | Edit the search over the loaded entries. |
| `y` | Copy entry | Copy the selected entry. |
| `i` | Copy insert ID | Copy the selected entry's insert ID. |
| `Ctrl-S` | Save visible logs | Save the visible entries to a file beside nuvem's log. |
| `Esc/Alt-←` | Back | Go back to the previous view in navigation history. |

## Log entry

An open log entry.

<!-- help-context: LoggingDetail -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `Enter/Esc/q` | Close entry | Close the entry and return to the list. `q` follows a `quit` rebinding. |
| `PgUp/PgDn` | Scroll | Scroll the entry a page. |
| `w` | Wrap | Wrap or truncate long lines. |
| `y` | Copy entry | Copy the selected entry. |
| `i` | Copy insert ID | Copy the selected entry's insert ID. |

## Tunnels

The tunnel list. See [Tunnels](../guides/tunnels.md).

<!-- help-context: TunnelsList -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `j/k` | Move selection | Move the selection down or up. The arrow keys do the same. |
| `K` | Kill tunnel | Stop the selected tunnel (`Shift-K`). |
| `d` | Dismiss finished | Remove a stopped or failed tunnel from the list. |
| `Esc/Alt-←` | Back | Go back to the previous view in navigation history. |

## FinOps

See [FinOps](../guides/finops.md).

<!-- help-context: FinOps -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `Esc/Alt-←` | Back | Go back to the previous view in navigation history. |

## Command palette

While the palette is open. Printable keys type into the search.

<!-- help-context: CommandPalette -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `↑/↓` | Select | Move through the matching commands. `j` and `k` type into the search here. |
| `type` | Enter text | Type into the field. |
| `Backspace` | Delete char | Delete the last character. |
| `Enter` | Run | Run the selected command. |
| `Esc` | Cancel | Cancel and close, changing nothing. |

## Resource action confirmation

The confirmation before a lifecycle action, single or batch. A batch that stops, resets, suspends, restarts or fails over instances asks you to type the number of targets first.

<!-- help-context: ConfirmResourceAction -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `Enter` | Confirm | Confirm and run the command shown. |
| `Esc` | Cancel | Cancel. Nothing runs. |

## Configuration activation

The confirmation before nuvem activates a gcloud configuration.

<!-- help-context: ConfirmConfigurationActivation -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `Enter` | Confirm | Confirm and run the command shown. |
| `y` | Confirm | Confirm the activation. |
| `n` | Cancel | Cancel the activation. |
| `Esc` | Cancel | Cancel and close, changing nothing. |

## Command preview

A preview that copies a command, such as **Show SSH command**.

<!-- help-context: ResourceCommandPreview -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `Enter` | Copy | Copy the command to the clipboard without running it. |
| `Esc` | Cancel | Close the preview. |

## SSH and tunnel launch

The preview before an SSH session or a tunnel starts.

<!-- help-context: SshLaunchPreview TunnelLaunchPreview -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `Enter` | Launch | Run the command: an SSH session takes over the terminal; a tunnel starts in the background. |
| `Esc` | Cancel | Cancel and close, changing nothing. |

## Plugin confirmation

The confirmation before a plugin runs.

<!-- help-context: PluginConfirmation -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `Enter` | Run command | Run the command shown. For a dangerous plugin, only after typing `1`. |
| `0-9` | Type 1 to acknowledge a dangerous plugin | For a dangerous plugin, type `1` to unlock `Enter`. |
| `Backspace` | Delete char | Delete the last character. |
| `Esc` | Cancel | Cancel and close, changing nothing. |

## Plugin prefix

After `,`, while nuvem waits for a plugin key. Any key a plugin in this service declares runs that plugin.

<!-- help-context: PluginPrefix -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `Esc` | Cancel | Disarm the plugin prefix. |

## Plugin input

The prompt for a plugin's [typed inputs](../plugins/typed-inputs.md).

<!-- help-context: PluginPrompt -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `type` | Enter text | Type into the field. |
| `Backspace` | Delete char | Delete the last character. |
| `Tab` | Next field | Move to the next input. |
| `Shift-Tab` | Previous field | Move to the previous input. |
| `←/→` | Change a dropdown's choice | Change a `bool` or `dropdown` input. |
| `Enter` | Continue to the confirmation | Go on to the confirmation. Works only when every field holds a valid value. |
| `Esc` | Cancel | Close the prompt. Nothing runs and nothing is logged. |

## Jump picker

The picker `J` opens.

<!-- help-context: ResourceJumpPicker -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `j/k` | Move selection | Move the selection down or up. The arrow keys do the same. |
| `Enter` | Jump to selected | Jump to the selected destination. |
| `Esc` | Cancel | Cancel and close, changing nothing. |

## Help

The help overlay `?` opens.

<!-- help-context: Help -->

| Key | Footer label | What it does |
| --- | --- | --- |
| `j/k` | Scroll | Scroll a line. |
| `PgUp/PgDn` | Page | Scroll a page. |
| `g/G` | Top/Bottom | Jump to the top or the bottom. |
| `Esc/?` | Close | Close help. |

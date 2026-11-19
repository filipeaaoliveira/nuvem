---
title: Switch gcloud configurations
description: Activate, create and edit gcloud configurations, and pick a project, region or zone.
sidebar:
  order: 4
---

In the Configurations workspace (`c`), you can see your gcloud configurations,
activate one, create one, or edit the active one.

## See your configurations

The Configurations workspace shows the name, account, project, region, and
zone of each loaded gcloud configuration. A missing value shows as `-`. Press
`/` to edit a case-insensitive filter across those fields, `Enter` to apply
it, and `Esc` to cancel the draft. Press `u` to clear the applied
configuration filter.

## Activate a configuration

To activate the selected configuration when it is not active, press `Enter`,
or open `:` and choose `Activate selected gcloud configuration`. Both paths
show a confirmation with the target context and the exact `gcloud` command.
Press `Enter` or `y` to confirm, or `Esc` or `n` to cancel. If the terminal
cannot show the full command preview, enlarge it before you confirm.

If you ask to activate the configuration that is already active, nuvem only
shows a status message and changes nothing. After gcloud succeeds, nuvem
reloads the list. If the reload does not show the target as the only active
configuration, nuvem keeps the target context and shows a reload warning.

## Create a configuration

Press `a` to create a gcloud configuration. The form takes a name, account,
project, region, and zone. The name is required, and nuvem skips the optional
fields you leave blank. Press `Enter` to create it, or `Esc` to cancel.

## Edit the active configuration

Press `e` to edit the active gcloud configuration's project, region, and zone.
A field you leave blank unsets that gcloud property. Press `Enter` to save, or
`Esc` to cancel.

While you edit, press `Ctrl-Space` on the focused Project, Region, or Zone
field to look up the values you can choose. In that picker:

- Press `/` to filter. While you filter, `Backspace` removes characters,
  `Enter` applies the filter, and `Esc` cancels the draft and restores the
  filter that was applied.
- Press `j`/`k` or the arrows to select a row, and `Enter` to copy it into the
  draft. Press `Enter` again to save.
- Press `r` to retry the lookup, or `Esc` to go back from the picker.

You can filter project rows by ID, name, or lifecycle state, region rows by
name or status, and zone rows by name, region, or status. The selection stays
on its row while the filter narrows or widens the list, so rows that appear or
disappear above the cursor never move it to a different item. If the selected
row stops matching, the selection moves to the nearest row that is still
visible.

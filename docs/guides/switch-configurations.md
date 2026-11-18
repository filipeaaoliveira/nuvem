---
title: Switch gcloud configurations
description: Activate, create and edit gcloud configurations, and pick a project, region or zone.
sidebar:
  order: 4
---

In the Configurations workspace, press `a` to create a native gcloud
configuration. The form accepts name, account, project, region, and zone. Name
is required; optional blank fields are skipped. Press `Enter` to create or
`Esc` to cancel.

In the Configurations workspace, press `e` to edit the active gcloud
configuration's project, region, and zone. Blank submitted fields unset the
corresponding native gcloud property. Press `Enter` to save or `Esc` to cancel.

While editing the active gcloud context, press Ctrl-Space on the focused
Project, Region, or Zone field to discover values. Use / to filter, j/k
or the arrows to select a row, and Enter to copy it into the draft. While
filtering, use Backspace to remove filter characters, Enter to apply the
filter, or Esc to cancel the filter draft and restore the applied picker
filter. Press Enter again to save, r to retry discovery, or Esc to go back
from the picker. Project rows can be filtered by ID, name, or lifecycle state;
region rows by name or status; and zone rows by name, region, or status. The
selection follows the row it is on while the filter narrows or widens the
visible list, so rows appearing or disappearing above the cursor never move it
onto a different item; when the selected row itself stops matching, the
selection falls back to the nearest still-visible row.

The Configurations workspace renders name, account, project, region, and zone
for each loaded native gcloud configuration, using `-` for missing values. Press
`/` to edit a case-insensitive filter across those fields, `Enter` to apply it,
and `Esc` to cancel the draft. Press `u` to clear the applied configuration
filter.

To activate an inactive selected configuration, press `Enter` or open `:` and
choose `Activate selected gcloud configuration`. Both paths show a confirmation
with the target context and exact `gcloud` command. Press `Enter` or `y` to
confirm, or `Esc` or `n` to cancel. If the terminal cannot show the full command
preview, enlarge it before confirming. Requesting activation for the active
configuration only shows a no-op status. If the reload does not show the target
as the sole active configuration after gcloud succeeds, the app keeps the target
context and shows a reload warning.

---
title: Cloud Functions
description: List 1st and 2nd gen functions across every region.
sidebar:
  order: 8
---

For Cloud Functions, pressing `r` runs:

```bash
gcloud --configuration <active-config> functions list --regions=- --v2 \
  --format=json --limit=500
```

`--regions=-` asks for the whole project rather than the region a
configuration's `functions/region` property happens to name, and `--v2` makes
both generations come back in one shape instead of 1st gen rows arriving in the
older format. The list is read-only and shows name, region, state,
generation, update time and invoke URL. Name and region lead because a
function's name is unique within its region, not within the project, so both
are needed to tell two rows apart. STATE is colour-coded like every other
lifecycle column: a function that is deploying or being deleted reads as busy
rather than as broken, and a state nuvem does not recognise is left uncoloured
rather than guessed at.

There is no detail pane: `Enter` does nothing on a function. `functions
describe` returns the same resource `functions list` already returned for every
row, so a pane would be a second gcloud call showing what is on screen. The
value with the least room in the table is the invoke URL, the first column
dropped on a narrow terminal — the preview beside the navigator gives it a line
of its own, along with the rest of the row, so it survives at widths where the
column is elided or gone.

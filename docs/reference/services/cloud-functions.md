---
title: Cloud Functions
description: List 1st and 2nd gen functions across every region.
sidebar:
  order: 8
---

For Cloud Functions, `r` reloads the list. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> functions list --regions=- --v2 \
  --format=json --limit=500
```

`--regions=-` asks for the whole project, not only the region that a
configuration's `functions/region` property names. `--v2` returns both
generations in one shape, so 1st gen rows do not arrive in the older format.

The list is read-only and shows name, region, state, generation, update time
and invoke URL. Name and region lead because a function's name is unique within
its region, not within the project. Both are needed to tell two rows apart.
STATE is colour-coded like every other lifecycle column. A function that is
deploying or being deleted reads as busy, not as broken. A state that nuvem
does not recognise stays uncoloured.

There is no detail pane, so `Enter` does nothing on a function. `functions
describe` returns the same resource that `functions list` already returned for
every row, so a pane would be a second call that shows what is on screen. The
invoke URL has the least room in the table and is the first column dropped on a
narrow terminal. The preview beside the navigator gives it a line of its own,
with the rest of the row, so you can still read it where the column is elided
or gone.

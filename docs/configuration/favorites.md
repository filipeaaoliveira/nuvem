---
title: Favorites
description: List projects, regions and zones first in the context picker.
sidebar:
  order: 5
---

`[favorites]` takes three optional keys, each a list of strings: `projects`,
`regions`, and `zones`. A `projects` entry is matched against the project ID.
A `regions` or `zones` entry is matched against the region or zone name.

The context picker lists favorited rows first. To open it, press
`Ctrl-Space` on the Project, Region, or Zone field while you edit the active
context. Favorited rows keep the order they already had in the
underlying list, not the order they were written in `[favorites]`. Each
favorited row is marked with a leading `*`.

Favoriting only reorders rows. The picker's own filter still applies, so a
favorite that the applied filter does not match stays hidden, along with
every other row that does not match.

Validation drops the smallest broken piece and keeps the rest, the same as
`[aliases]`:

- If the `[favorites]` section is not a table, nuvem ignores the whole
  section with one warning.
- If a key's value is not an array (for example `projects = "prod-project"`),
  nuvem drops that whole key with a warning. The other keys are untouched.
- If a value inside `projects`, `regions`, or `zones` is not a string that is
  non-empty once trimmed, nuvem drops it with a warning that names the key.
  Every other value, in that key and in the other two, is kept.

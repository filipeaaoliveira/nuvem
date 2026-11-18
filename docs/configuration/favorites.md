---
title: Favorites
description: List projects, regions and zones first in the context picker.
sidebar:
  order: 5
---

`[favorites]` takes three optional keys, each a list of strings: `projects`,
`regions`, and `zones`. A `projects` entry is matched against the project id;
a `regions` or `zones` entry is matched against the region or zone name.

The configuration context picker (opened with `Ctrl-Space` on the Project,
Region, or Zone field while editing the active context) lists favorited rows
first, ahead of the rest, keeping the order they already had in the
underlying list (not the order they were written in `[favorites]`); each
favorited row is marked with a leading `*`. Favoriting only reorders — it
does not exempt a row from the picker's own filter. A favorite that the
applied filter does not match stays hidden along with every other
non-matching row.

Validation drops the smallest broken piece and keeps the rest, the same as
`[aliases]`: a `[favorites]` section that is not a table is ignored in its
entirety with one warning; a key's value that is not an array (for example
`projects = "prod-project"`) drops that whole key with a warning, leaving the
other keys untouched; and a value inside `projects`, `regions`, or `zones`
that is not a non-empty (once trimmed) string is dropped with a warning
naming the key, while every other value — in that key and in the other two —
is kept.

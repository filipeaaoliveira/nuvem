---
title: Persistent disks
description: List zonal and regional persistent disks and find the ones nothing uses.
sidebar:
  order: 9
---

For persistent disks, pressing `r` runs:

```bash
gcloud --configuration <active-config> [--project <project>] compute disks list --format=json --limit=500
```

One call returns zonal and regional disks together, so LOCATION holds a zone
for some rows and a region for others. A disk's name is unique only within its
location, which is why LOCATION follows NAME.

The columns are NAME, LOCATION, STATUS, ORPHAN, SIZE, TYPE and ATTACHED.

- **ORPHAN** marks a disk that is `Ready` and attached to no instance: storage
  you pay for and nothing uses. Type `/ORPHAN` to list only those disks.
  ORPHAN is the one column you cannot sort by; filtering does the same job.
- **ATTACHED** names the instances that use the disk. An unattached disk has
  an empty cell, so sorting by ATTACHED also groups the orphans together.

Label mode (`-l k=v`) is pushed to gcloud as a label filter. Every other filter
mode narrows the loaded rows on what the table shows.

The disks list is list-only and read-only: `Enter` does nothing on a row, and
nuvem never creates, resizes or deletes a disk. To act on a disk, write a
[plugin](../../plugins/write-a-plugin.md) with `scope = "disks"`.

The [FinOps](../../guides/finops.md) workspace reads the locations of the disks
you have listed to decide where to ask for idle-disk recommendations.

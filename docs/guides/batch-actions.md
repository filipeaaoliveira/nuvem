---
title: Batch actions on marked rows
description: Mark several GCE or Cloud SQL instances and run one lifecycle action on all of them.
sidebar:
  order: 7
---

Mark several GCE instances or Cloud SQL instances, then run one lifecycle
action on all of them at once. The actions on a single instance are on the
[GCE instances](../reference/services/gce-instances.md) and
[Cloud SQL](../reference/services/cloud-sql.md) pages. Only services with
lifecycle actions offer marks. In every other service, the mark keys do
nothing.

## Mark instances

- Press `Space` to mark or unmark the selected instance. That instance becomes
  the mark anchor.
- Press `Shift-V` to range-mark every instance between the anchor and the
  cursor, inclusive. The range follows the list as you see it, filtered and
  sorted.
- Press `\` to clear every mark.
- Press `Shift-U` to unmark only the currently visible marks, the ones that
  match the filter. Marks on rows that the active filter hides stay.

Marks belong to one service. If you mark GCE instances and then switch to
Cloud SQL, you see Cloud SQL's own marks, which start empty. A batch never
mixes the two services.

While at least one instance is marked, the resource list title shows
`Marked: <n>`. If the active filter hides some marked instances, the title
shows `Marked: <n> (<hidden> hidden)`, so the total stays in view.

## Run an action on the marked instances

With at least one instance marked, open the command palette (`:`). The
service's lifecycle commands then act on the whole marked set instead of the
selected row. In GCE these are
`Start`/`Stop`/`Reset`/`Suspend`/`Resume selected GCE instance`. In Cloud SQL
they are `Start`/`Stop`/`Restart`/`Fail over selected Cloud SQL instance`. A
marked instance that is not eligible for the action is skipped, and the
confirmation lists it as skipped. For example, Stop skips a stopped instance,
and Fail over skips a zonal instance without HA.

The batch confirmation names the service it targets. It lists every eligible
target by name with the API call it makes for that target, such as
`compute.instances.stop zones/europe-west1-b/instances/web-01`. The skipped
targets follow with their reason: `ineligible (<status>)`, or
`ineligible (requires an HA (regional) instance)` for a Cloud SQL failover on
a zonal instance. The list is never summarized or truncated, so you can read
the complete target set before you confirm. If it does not fit, the
confirmation scrolls.

Destructive batch actions (GCE Stop, Reset, and Suspend; Cloud SQL
Stop, Restart, and Fail over) prompt `Type <n> to confirm` and require typing
that exact number of eligible targets, a stronger acknowledgement than the
single-resource confirmation. Non-destructive batch actions (GCE Start and
Resume; Cloud SQL Start) prompt `Enter to confirm` like the single-resource
flow.
Press `Esc` to cancel a batch confirmation.

## Follow a running batch

After you confirm, at most `--batch-concurrency` (default `4`) targets run at
a time, so a large marked set never sends every call at once. While the batch
runs, a progress panel shows a summary line
(`<ok>/<total> done · <fail> failed · <pending> pending`) and one line per
target with its current status (`PENDING`, `OK`, or `FAIL: <reason>`). After
the first 20 targets, a `+N more` line stands in for the rest. The summary
line counts every target, whatever the size of the batch.

The panel does not take the keyboard, so you can keep working while the batch
runs. When every target has finished, nuvem refreshes the instance list once.
The panel closes when that refresh shows the new rows.

The Home overview's "Recent operations" list is a separate history, and it
keeps only the most recent entries. After a batch larger than that history,
only its most recent targets remain in the list.

## Switch context during a batch

If you switch gcloud configuration or context while a batch runs, nuvem
detaches the batch. The progress panel closes at once. Targets that already
started run to the end, but nuvem discards their results. Targets that had not
started are cancelled and never run. Each target's "Recent operations"
entry records what nuvem knows about it, so none stays in progress. A target
that never sent its call shows `cancelled`. A target whose call was already in
flight shows `unknown`, because nuvem never saw its outcome.

If you confirm a new batch while one is still running, nuvem detaches the
running batch the same way. Its remaining targets settle as `cancelled` or
`unknown`, and the progress panel tracks the new batch. Those entries say the
batch was superseded by a newer one, because you confirmed a replacement and
cancelled nothing. The statuses are the same as after a context switch,
because they are all nuvem can prove about each target.

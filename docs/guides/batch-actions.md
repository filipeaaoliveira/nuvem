---
title: Batch actions on marked rows
description: Mark several GCE or Cloud SQL instances and run one lifecycle action on all of them.
sidebar:
  order: 7
---

The GCE instances and Cloud SQL instances lists support marking multiple rows
so a mutation targets all of them at once, alongside the single-instance actions on the
[GCE instances](../reference/services/gce-instances.md) and
[Cloud SQL](../reference/services/cloud-sql.md) pages. Marking is offered only on services that have lifecycle actions to
apply; every other service is read-only here, so the mark keys do nothing
there.

- Press `Space` to mark or unmark the selected instance; the toggled instance
  becomes the mark anchor.
- Press `Shift-V` to range-mark every instance between the anchor and the
  current cursor, inclusive, in the visible (filtered/sorted) order.
- Press `\` to clear every mark.
- Press `Shift-U` to unmark only the currently visible (filter-matching)
  marks, leaving marks on rows hidden by the active filter untouched.

Marks are per service: marking GCE instances and then switching to Cloud SQL
shows Cloud SQL's own (initially empty) marks, and a batch never mixes the two.

Whenever at least one instance is marked, the resource list title shows a
`Marked: <n>` indicator; if the active filter hides some marked instances it
shows `Marked: <n> (<hidden> hidden)` so the total never silently drops out of
view.

With one or more instances marked, opening the command palette (`:`) turns that
service's lifecycle commands into batch commands: instead of the single
selected row, the action targets the whole marked set. That is
`Start`/`Stop`/`Reset`/`Suspend`/`Resume selected GCE instance` in the GCE
workspace, and `Start`/`Stop`/`Restart`/`Fail over selected Cloud SQL instance`
in the Cloud SQL workspace. Marked instances that are not eligible for the
chosen action (for example a stopped instance targeted by Stop, or a zonal
non-HA instance targeted by Fail over) are skipped rather than silently
dropped.

The batch confirmation names the service it targets, and lists every eligible
target by name together with the API call it will make for that target, such
as `compute.instances.stop zones/europe-west1-b/instances/web-01`, followed by
any skipped targets and their reason (`ineligible (<status>)`, or
`ineligible (requires an HA (regional) instance)` for a Cloud SQL failover on a
zonal instance), so the operator can verify the complete target set before
committing — it is never summarized or truncated (the modal scrolls when it
overflows). Destructive batch actions (GCE Stop, Reset, and Suspend; Cloud SQL
Stop, Restart, and Fail over) prompt `Type <n> to confirm` and require typing
that exact number of eligible targets, a stronger acknowledgement than the
single-resource confirmation. Non-destructive batch actions (GCE Start and
Resume; Cloud SQL Start) prompt `Enter to confirm` like the single-resource
flow.
Press `Esc` to cancel a batch confirmation.

Once confirmed, targets run with bounded concurrency — at most
`--batch-concurrency` (default `4`) actions in flight at a time — so a large
marked set never spawns unbounded concurrent `gcloud` processes. While the
batch runs, a progress panel shows an aggregate line
(`<ok>/<total> done · <fail> failed · <pending> pending`) and one line per
target with its current status (`PENDING`, `OK`, or `FAIL: <reason>`); a very
long target list is capped with a `+N more` line. This panel is read-only —
the keyboard keeps working normally while a batch is in flight. Once every
target resolves, the app issues a single follow-up instance-list refresh, and
the progress panel is dismissed once that refresh applies fresh rows.

The live batch progress panel tracks every target regardless of batch size, so
batch **monitoring** is never limited by any cap. The Home overview's "Recent
operations" list is a separate, intentionally bounded **history** log (it keeps
only the most recent entries); for a batch larger than that capacity only the
most recent targets remain in that historical list. This cap is history-only —
it never affects live batch monitoring, which the per-target progress panel
above renders in full.

Switching gcloud configuration or context while a batch is running safely
detaches it: the progress panel disappears immediately, any targets already
running are left to finish but their results are discarded, and any
not-yet-started targets are cancelled and never run. No target is left showing
as still in progress afterwards: each one's "Recent operations" entry is
settled at what is actually known — `cancelled` for a target that never issued
its command, `unknown` for one whose command was already in flight when the
switch happened and whose real outcome is therefore unobserved.

Confirming a new batch while one is still running detaches the running batch
the same way — its remaining targets are settled as `cancelled` or `unknown` as
above — and the progress panel then tracks the newly confirmed batch. Those
entries say the batch was superseded by a newer one rather than cancelled: the
outcome is the same because it is what is provable about each target, but you
cancelled nothing here, you confirmed a replacement.

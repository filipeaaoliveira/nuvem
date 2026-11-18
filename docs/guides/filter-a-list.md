---
title: Filter a list
description: "Narrow a resource list with the filter grammar: regex, fuzzy, label and inverse modes."
sidebar:
  order: 3
---

## Filter the list you are looking at

1. Press `/` with the list focused.
2. Type the filter. A `[mode]` chip next to it shows which mode the text
   selects.
3. Press `Enter` to apply it, or `Esc` to drop the draft and keep the filter
   already applied.
4. To clear the filter, press `/`, delete the text with `Backspace`, and press
   `Enter`.

Each service keeps its own filter. Switching service shows that service's
filter, not the one you typed elsewhere.

## The filter grammar

The filter text is a small prefix grammar with four modes. It applies to every
service except Cloud Storage:

| Input          | Mode      | Meaning                                             |
| -------------- | --------- | --------------------------------------------------- |
| `term`         | `regex`   | Case-insensitive regex, partial match, any field     |
| `-f term`      | `fuzzy`   | Case-insensitive subsequence match, any field        |
| `-l k=v[,k=v]` | `label`   | GCP resource labels; every pair must match           |
| `!term`        | `inverse` | Rows the `regex` predicate does **not** match        |

`regex` is the default, so a plain word keeps behaving the way the old
substring filter did. An empty input — and a prefix with nothing after it
(`-f`, `-l`, `!`) — means "no filter": every row stays visible and nothing is
reported, because you are mid-typing and an error there would be noise. The
mode renders as a `[mode]` chip next to the filter text. While the input is
open the chip tracks the *draft*, so it flips to `[fuzzy]` the moment you type
`-f`, before anything is applied; that is what makes the grammar discoverable.

Label mode is server-side only. `-l` narrows nothing locally on any service —
almost no list row carries labels to match against, and the client-side
predicate keeps every row even where one does (a secret's does) — so the term
is pushed to gcloud as a `--filter` and whatever comes back is shown as-is.
A label-mode read therefore always goes through `gcloud`, even on the services
that otherwise read through the native client libraries: it is the one filter
mode the native path cannot honour, and an unfiltered table under a `[label]`
chip would be worse than the slower call.
Three consequences follow. Label values match
**case-sensitively** on the server, unlike every other mode, so `-l env=Prod`
will not find a `prod` label. `-l` does nothing under `--demo`, which serves
embedded fixtures and ignores `--filter`. And Cloud Run exposes no user labels
in list output, so `-l` there reports
`label filtering is not supported for Cloud Run` and leaves the table unfiltered
rather than pretending everything matched. Keys and values are
limited to ASCII letters, digits, `.`, `_`, and `-`, and a key may not contain
`.` (GCP label keys cannot); anything else reports `label selector is not
expressible as a gcloud filter`.

Which label field each service pushes:

<!-- label-filter-fields -->

| Service | `-l` is sent as |
| --- | --- |
| GCE instances | `labels` |
| GKE clusters | `resourceLabels` |
| Cloud SQL | `settings.userLabels` |
| Cloud Run | not supported |
| Cloud Storage | not supported: Cloud Storage does not use the grammar |
| Secret Manager | `labels` |
| Cloud Functions | `labels` |
| Persistent disks | `labels` |
| Pub/Sub topics | `labels` |
| Pub/Sub subscriptions | `labels` |
| BigQuery datasets | not supported: `bq` takes no filter expression |
| Service accounts | not supported: service accounts have no labels |
| IAM policy | not supported: policy bindings have no labels |
| VPC networks | not supported: networks have no labels |
| VPC subnets | not supported: subnets have no labels |
| Firewall rules | not supported: firewall rules have no labels |

On a service marked not supported, `-l` reports that label filtering is not
supported and leaves the table unfiltered.

A malformed filter never empties the table. `a(b` shows **every** loaded row
plus an inline `⚠ invalid regex: unclosed group` next to the filter text, and
`-l env` shows every row plus `⚠ malformed label selector: expected key=value`.
Nothing panics and nothing silently disappears: an empty table behind a bad
pattern is indistinguishable from a correct empty result, so nuvem shows too
much and says why. The `⚠` describes the *applied* filter, so it is dropped as
soon as your draft diverges from what is applied.

Cloud Storage keeps the plain case-insensitive substring filter and shows
no mode chip. Its object browser already pushes safe filter text down as a
server-side object prefix, and grammar prefixes would fight that pushdown for
no benefit. The grammar therefore does not apply there at all, and the
consequence is worth stating plainly: on Cloud Storage `-l env=prod`, `-f api`,
and `!x` are not modes, they are text. In the bucket list that text is
matched as literal substrings; in the object browser it goes through the same
pushdown as any other text, so `-l env=prod` (no leading `/`, no glob
characters) is sent as a server-side object prefix and the tools line reads
`-l env=prod (remote)`. Either way it normally matches no bucket or object
name and you get an empty table with no chip and no `⚠`:
Cloud Storage reports no filter error, because it never parses the grammar in
the first place.

## What runs when you apply a filter

For GCE, GKE, and Cloud SQL, applying or clearing the filter refreshes the list and
pushes the term down to gcloud as a server-side `--filter` when it is safely
expressible against raw fields, then narrows the residual client-side; a term
that could only match a transformed or derived display value falls back to
client-side-only narrowing so a matching row is never dropped. Cloud Run is
capped but always narrowed client-side (its Knative list shape isn't reliably
server-filter-expressible). Secret Manager also refreshes on apply and clear,
but pushes only label mode: a secret's replication and its `key=value` label
rendering are display values with no raw field behind them, so every other
mode narrows client-side. Cloud Functions behaves the same way and for the same
reason: label mode is pushed as `labels.k = "v"`, every other mode narrows the
loaded rows on every cell the table paints — name, region, state, generation,
update time, invoke URL and `key=value` labels. State and generation are
narrowed on the words the column shows (`Active`, `1st gen`), which appear in
no raw gcloud field, which is exactly why nothing but label mode is pushed.
Pub/Sub topics behave the same way again: label mode is pushed as
`labels.k = "v"`, and every other mode narrows the loaded rows on every cell the
table paints — name, subscription count, retention, KMS key and `key=value`
labels. The count is the clearest case for keeping the rest client-side: it is
derived from a second list call and appears in no raw field of the topic
resource at all. The displayed narrowing is still client-side: GCE
filters match name, zone, status, machine type, internal IP, and external IP;
GKE filters match name, location, status, Kubernetes version, node count, and
endpoint visibility.

Persistent disks and Pub/Sub subscriptions push label mode the same way and
refresh on apply. Service accounts and the IAM policy refresh on apply but
push nothing. BigQuery datasets and the three VPC tables run no command when
you apply a filter: they re-narrow the rows already loaded, so a transient
error cannot blank a table while you type.

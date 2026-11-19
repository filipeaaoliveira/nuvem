---
title: Filter a list
description: "Narrow a resource list with the filter grammar: regex, fuzzy, label and inverse modes."
sidebar:
  order: 3
---

Press `/` in a resource list to narrow it to the rows that match what you
type.

## Filter the list you are looking at

1. Press `/` with the list focused.
2. Type the filter. A `[mode]` chip next to it shows which mode the text
   selects.
3. Press `Enter` to apply it, or `Esc` to drop the draft and keep the filter
   already applied.
4. To clear the filter, press `/`, delete the text with `Backspace`, and press
   `Enter`.

Each service keeps its own filter. When you switch service, you see that
service's filter, not the one you typed elsewhere.

## The filter grammar

The filter text is a small prefix grammar with four modes. It applies to every
service except Cloud Storage:

| Input          | Mode      | Meaning                                             |
| -------------- | --------- | --------------------------------------------------- |
| `term`         | `regex`   | Case-insensitive regex, partial match, any searchable field |
| `-f term`      | `fuzzy`   | Case-insensitive subsequence match, any searchable field |
| `-l k=v[,k=v]` | `label`   | GCP resource labels; every pair must match           |
| `!term`        | `inverse` | Rows the `regex` predicate does **not** match        |

`regex` is the default, so a plain word matches every row that contains it in
any searchable field. The searchable fields are the values the list shows,
and a few more that some services name under
[What runs when you apply a filter](#what-runs-when-you-apply-a-filter). An
empty input means no filter, and so does a prefix with nothing after it
(`-f`, `-l`, `!`). Every row stays visible and nuvem reports no error, because
you are still typing.

The `[mode]` chip next to the filter text shows the mode. While the input is
open, the chip follows the draft. It changes to `[fuzzy]` as soon as you type
`-f`, before you apply anything.

## Label mode

Label mode is server-side only. `-l` narrows nothing locally on any service.
Almost no list row carries labels to match against, and where one does (a
secret's row), nuvem still keeps every loaded row in label mode. nuvem pushes
the term to gcloud as a `--filter` and shows whatever comes back as it is.

A label-mode read always goes through `gcloud`, even on the services that
otherwise read through the Google Cloud client libraries. The client
libraries cannot apply this filter, and an unfiltered table under a `[label]`
chip is worse than a slower call. Three consequences follow:

- Label values match case-sensitively on the server, unlike every other mode.
  `-l env=Prod` does not find a `prod` label.
- `-l` does nothing under `--demo`, which serves built-in sample data and
  ignores `--filter`.
- Cloud Run exposes no user labels in list output. There, `-l` reports
  `label filtering is not supported for Cloud Run` and leaves the table
  unfiltered.

Values can contain only ASCII letters, digits, `.`, `_`, and `-`. Keys can
contain the same characters except `.`, because GCP label keys cannot.
Anything else reports `label selector is not expressible as a gcloud filter`.

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

## A malformed filter

A malformed filter never empties the table. `a(b` shows every loaded row,
with an inline `⚠ invalid regex: unclosed group` next to the filter text.
`-l env` shows every row with `⚠ malformed label selector: expected key=value`.
An empty table behind a bad pattern looks the same as a correct empty result,
so nuvem shows too much and says why.

The `⚠` describes the applied filter. It disappears as soon as your draft
differs from the filter that is applied.

## Filters in Cloud Storage

Cloud Storage keeps the plain case-insensitive substring filter and shows no
mode chip. Its object browser already sends filter text with no glob
characters to the server as an object prefix, and grammar prefixes conflict
with that. So the grammar does not apply to Cloud Storage at all. There, `-l env=prod`,
`-f api`, and `!x` are plain text, not modes.

In the bucket list, that text is matched as literal substrings. In the object
browser, it goes to the server like any other text. `-l env=prod` has no
leading `/` and no glob characters, so nuvem sends it as an object prefix, and
the tools line reads `-l env=prod (remote)`.

Either way, the text normally matches no bucket or object name, and you get an
empty table with no chip and no `⚠`. Cloud Storage reports no filter error,
because it never parses the grammar.

## What runs when you apply a filter

Lists come from the Google Cloud client libraries, or from `bq` for BigQuery.
Apart from the Cloud Storage object prefix described above, neither is given
the filter. In every mode except label mode, the filter narrows the loaded
rows on the client. List reads are capped at 500 rows, so
in these modes the filter sees only the rows that were loaded.

GCE, GKE, and Cloud SQL refresh the list when you apply or clear the filter.
In label mode, that refresh goes through gcloud with the label selector as a
server-side `--filter`. GCE filters match name, zone, status, machine type,
internal IP, and external IP. GKE filters match name, location, status,
Kubernetes version, node count, and endpoint visibility.

Cloud Run also refreshes on apply and clear, and always narrows on the client.

Secret Manager, Cloud Functions, and Pub/Sub topics refresh on apply and
clear, and push only label mode, as `labels.k = "v"`. Every other mode
narrows the loaded rows on every cell the table shows:

- Cloud Functions: name, region, state, generation, update time, invoke URL,
  and `key=value` labels. State and generation match the words the column
  shows (`Active`, `1st gen`).
- Pub/Sub topics: name, subscription count, retention, KMS key, and
  `key=value` labels.

Persistent disks and Pub/Sub subscriptions push label mode the same way and
refresh on apply. Service accounts and the IAM policy refresh on apply but
push nothing. BigQuery datasets and the three VPC tables run no command when
you apply a filter. They narrow the rows already loaded again, so a transient
error cannot blank a table while you type.

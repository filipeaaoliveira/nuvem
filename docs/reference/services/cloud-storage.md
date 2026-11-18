---
title: Cloud Storage
description: Browse buckets and objects, read metadata, and copy gs:// URIs.
sidebar:
  order: 6
---

For Cloud Storage, pressing `r` runs:

```bash
gcloud --configuration <active-config> storage buckets list --format=json
```

The bucket list shows name, location, storage class, last-updated time,
versioning, and public access prevention where available. Press `Enter` on a
bucket to open a hierarchical, prefix-aware
object browser with breadcrumb navigation instead of the flat list used by
other services. Object rows show name, size, storage class, updated time,
content type, and encryption/retention indicators (never key material). Press
`n` to load the next bounded page of objects (100 objects per `gcloud`
request) while preserving the current selection; there is no unbounded load,
so very large buckets are browsed page by page. Press `-` or `Backspace` to
move up one prefix level, eventually returning to the bucket list from the
top-level prefix.

Press `/` in the object browser to filter the already-loaded rows; when the
filter text is also a safe prefix (no glob characters), it is additionally
sent to `gcloud` as a server-side prefix filter, and the filter tools line
labels the applied filter `(local)` or `(remote)` so it is clear whether the
current rows reflect a client-side or backend-narrowed listing. Press `i` on a
bucket, or `Enter` on an object, to view read-only metadata across Summary,
Metadata, and JSON tabs. Press `y` in the bucket list or object browser to
copy the selected item's `gs://` URI to the clipboard; inside a bucket/object
detail view `y` follows the detail-pane rules in the [key reference](../keys.md), so it
copies the selected field, or the raw document on the JSON tab, and falls back
to the `gs://` URI only when neither applies. Copied values are stripped of
terminal escape sequences and control characters (newlines and tabs survive),
so a hostile bucket or object name cannot smuggle them onto the clipboard.
Distinct states are shown for an empty bucket, a filter with no matches,
permission denied, a bucket requiring requester-pays
billing, a bucket-level retention policy, and a partial page failure (rows
already loaded are kept if a later page fails to load).

Cloud Storage browsing is read-only: uploading, downloading, deleting objects,
ACL/IAM management, and signed URLs are out of scope.

Note on pagination: the object browser pages with the token the service itself
issues, so continuing a listing resumes exactly where the previous page ended.
Demo mode and the fallback path use `gcloud storage objects list`'s own
`--page-token`/`--page-size` flags instead. The gcloud
CLI does not expose a raw, independently resumable cursor beyond what
`--format=json` returns for a given invocation, so continuing a listing
precisely is best-effort against real bucket state. Pagination itself (page
boundaries, selection preservation, and the local/remote filter interaction)
is modeled at the domain/reducer layer and is fixture-tested rather than
verified against live changing buckets.

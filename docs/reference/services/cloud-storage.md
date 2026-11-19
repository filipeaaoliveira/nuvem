---
title: Cloud Storage
description: Browse buckets and objects, read metadata, and copy gs:// URIs.
sidebar:
  order: 6
---

The Cloud Storage list shows the project's buckets. `Enter` opens a bucket's
objects, and you can copy any bucket's or object's `gs://` URI.

For Cloud Storage, `r` reloads the bucket list. The equivalent `gcloud`
command is:

```bash
gcloud --configuration <active-config> storage buckets list --format=json
```

The bucket list shows name, location, storage class, last-updated time,
versioning, and public access prevention where available. Press `Enter` on a
bucket to open the object browser. Other services show a flat list. The object
browser follows prefixes like folders and shows a breadcrumb.

Object rows show name, size, storage class, updated time, content type, and
encryption and retention indicators (never key material). Press `n` to load the
next page of objects (100 objects per request). The current selection stays in
place. There is no unbounded load, so you browse a very large bucket page by
page. Press `-` or `Backspace` to move up one prefix level. From the top-level
prefix, this returns to the bucket list.

Press `/` in the object browser to filter the rows already loaded. If the filter
text is also a safe prefix (no glob characters), nuvem also sends it to the
service as a server-side prefix filter. The filter tools line labels the
applied filter `(local)` or `(remote)`, so you can tell whether the rows come
from a client-side or a server-side narrowing.

Press `i` on a bucket, or `Enter` on an object, to view read-only metadata
across Summary, Metadata, and JSON tabs. Press `y` in the bucket list or object
browser to copy the selected item's `gs://` URI to the clipboard. Inside a
bucket or object detail view, `y` follows the detail-pane rules in the
[key reference](../keys.md). It copies the selected field, or the raw document
on the JSON tab, and copies the `gs://` URI only when neither applies. nuvem
strips terminal escape sequences and control characters from copied values
(newlines and tabs survive), so a hostile bucket or object name cannot smuggle
them onto the clipboard.

The browser shows a distinct state for each of these cases:

- An empty bucket.
- A filter with no matches.
- Permission denied.
- A bucket that requires requester-pays billing.
- A bucket-level retention policy.
- A partial page failure. If a later page fails to load, the rows already
  loaded stay.

Cloud Storage browsing is read-only. Uploading, downloading, deleting objects,
ACL/IAM management, and signed URLs are out of scope.

The object browser pages with the token that the service issues, so the next
page starts exactly where the previous page ended.

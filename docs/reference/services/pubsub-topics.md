---
title: Pub/Sub topics
description: List Pub/Sub topics with their subscription count, retention and encryption key.
sidebar:
  order: 10
---

The Pub/Sub topics list shows the project's topics, with each topic's
subscription count, retention and encryption key.

For Pub/Sub topics, `r` reloads the list with two calls. The equivalent `gcloud` commands are:

```bash
gcloud --configuration <active-config> [--project <project>] pubsub topics list --format=json --limit=500
gcloud --configuration <active-config> [--project <project>] pubsub subscriptions list --format=json --limit=500
```

The second call counts each topic's subscriptions. It is best-effort. If it
fails, for example because you are not allowed to list subscriptions, the
topics still show and SUBS reads `—`, not `0`.

The columns are NAME, SUBS (the subscription count), RETENTION, KMS KEY and
LABELS.

Label mode (`-l k=v`) is pushed to gcloud as `labels.k = "v"`. Every other
filter mode narrows the loaded rows on every cell the table shows. Those modes
stay on the client because some cells, such as the subscription count, come
from the second call and appear in no field of the topic itself.

The topics list is list-only and read-only. `Enter` does nothing on a row, and
nuvem never creates, deletes or publishes to a topic. `L` opens Cloud Logging
for the selected topic.

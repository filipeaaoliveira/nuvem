---
title: Pub/Sub topics
description: List Pub/Sub topics with their subscription count, retention and encryption key.
sidebar:
  order: 10
---

For Pub/Sub topics, pressing `r` runs two commands:

```bash
gcloud --configuration <active-config> [--project <project>] pubsub topics list --format=json --limit=500
gcloud --configuration <active-config> [--project <project>] pubsub subscriptions list --format=json --limit=500
```

The second call counts each topic's subscriptions. It is best-effort: when it
fails, for example because listing subscriptions is not allowed, the topics
still show and SUBS reads `—` rather than `0`.

The columns are NAME, SUBS (the subscription count), RETENTION, KMS KEY and
LABELS.

Label mode (`-l k=v`) is pushed to gcloud as `labels.k = "v"`. Every other
filter mode narrows the loaded rows on every cell the table paints. The count
is the clearest case for keeping the rest client-side: it is derived from the
second call and appears in no raw field of the topic.

The topics list is list-only and read-only: `Enter` does nothing on a row, and
nuvem never creates, deletes or publishes to a topic. `L` opens Cloud Logging
for the selected topic.

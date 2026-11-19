---
title: Pub/Sub subscriptions
description: List every Pub/Sub subscription in the project with its topic and delivery settings.
sidebar:
  order: 11
---

For Pub/Sub subscriptions, `r` reloads the list. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> [--project <project>] pubsub subscriptions list --format=json --limit=500
```

One flat table holds every subscription in the project. TOPIC names the topic
each one belongs to.

The columns are NAME, TOPIC, DELIVERY, STATE, ACK(S) (the acknowledgement
deadline in seconds), RETENTION, DEAD LETTER and LABELS.

If the call fails, the table shows the failure. An empty table would claim
that the project has no subscriptions.

Label mode (`-l k=v`) is pushed to gcloud as a label filter. Every other filter
mode narrows the loaded rows on what the table shows.

The subscriptions list is list-only and read-only. `Enter` does nothing on a
row, and nuvem never creates, deletes, seeks or pulls from a subscription. `L`
opens Cloud Logging for the selected subscription.

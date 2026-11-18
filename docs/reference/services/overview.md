---
title: All services
description: Every service nuvem can browse, the names it answers to, and what you can do in each.
sidebar:
  order: 1
---

Every service appears in the Resources sidebar and the command palette. The
**Id** is the name the config file uses for it, in `[columns.<id>]`, in
`[[jumps]]` and in a plugin's `scope`. Any of the **Aliases** also works there,
and in the sidebar search. Ids and aliases are case-insensitive.

| Service | Id | Aliases | Category |
| --- | --- | --- | --- |
| [GCE instances](gce-instances.md) | `compute` | `gce`, `compute engine`, `instances` | Compute |
| [GKE clusters](gke-clusters.md) | `gke` | `gke`, `kubernetes`, `clusters` | Containers |
| [Cloud SQL](cloud-sql.md) | `cloudsql` | `cloud sql`, `sql`, `database`, `postgres`, `mysql` | Databases |
| [Cloud Run](cloud-run.md) | `cloudrun` | `run`, `serverless` | Serverless |
| [Cloud Storage](cloud-storage.md) | `storage` | `gcs`, `storage`, `bucket`, `buckets` | Storage |
| [Secret Manager](secret-manager.md) | `secrets` | `secret`, `secrets`, `secret manager`, `sm` | Security |
| [Cloud Functions](cloud-functions.md) | `functions` | `functions`, `gcf`, `cloud functions` | Serverless |
| [Persistent disks](persistent-disks.md) | `disks` | `disks`, `pd`, `persistent disks`, `orphans` | Storage |
| [Pub/Sub topics](pubsub-topics.md) | `pubsub-topics` | `pubsub`, `pub/sub`, `topics`, `pubsub topics`, `pubsub-topics` | Messaging |
| [Pub/Sub subscriptions](pubsub-subscriptions.md) | `pubsub-subscriptions` | `subscriptions`, `subs`, `pubsub subscriptions`, `pubsub-subscriptions` | Messaging |
| [BigQuery datasets](bigquery-datasets.md) | `bigquery-datasets` | `bigquery`, `bq`, `datasets`, `bigquery-datasets` | Analytics |
| [Service accounts](service-accounts.md) | `service-accounts` | `iam`, `sa`, `service accounts`, `service-accounts` | Security |
| [IAM policy](iam-policy.md) | `iam-policy` | `iam policy`, `iam-policy`, `policy`, `bindings`, `roles` | Security |
| [VPC networks](vpc-networks.md) | `vpc-networks` | `vpc`, `networks`, `vpc networks`, `vpc-networks` | Networking |
| [VPC subnets](vpc-subnets.md) | `vpc-subnets` | `subnets`, `subnetworks`, `vpc subnets`, `vpc-subnets` | Networking |
| [Firewall rules](firewall-rules.md) | `firewall-rules` | `firewall`, `firewalls`, `fw`, `firewall-rules` | Networking |

The config file also accepts `compute_instances`, `compute-instances`,
`gke_clusters`, `gke-clusters`, `cloud_sql`, `cloud-sql`, `cloud_run`,
`cloud-run` and `run`, and each service's label (`"GCE instances"`). Add your
own names with [`[aliases]`](../../configuration/aliases.md).

## What each service supports

| Service | `Enter` opens | Mark rows | `t` tunnel | `L` logs | Changes resources |
| --- | --- | --- | --- | --- | --- |
| GCE instances | detail: Summary, Network, Disks, Metadata, JSON | yes | yes | yes | start, stop, reset, suspend, resume, SSH |
| GKE clusters | detail: Summary, Networking, Node Pools, Addons, JSON | no | no | yes | no (k9s opens the cluster) |
| Cloud SQL | detail: Summary, Connectivity, Maintenance, Backups, Replicas, Users, JSON | yes | yes | yes | start, stop, restart, fail over |
| Cloud Run | detail: Summary, Traffic, Revisions, Configuration, Conditions, JSON; `Enter` on a revision opens detail: Summary, Conditions, JSON | no | no | yes | no |
| Cloud Storage | the object browser; `i` opens bucket metadata, and `Enter` on an object opens detail: Summary, Metadata, JSON | no | no | yes | no |
| Secret Manager | detail: Summary, Versions, Labels, JSON | no | no | yes | no |
| Cloud Functions | nothing | no | no | yes | no |
| Persistent disks | nothing | no | no | no | no |
| Pub/Sub topics | nothing | no | no | yes | no |
| Pub/Sub subscriptions | nothing | no | no | yes | no |
| BigQuery datasets | the dataset's tables, then a table's column schema | no | no | yes | no |
| Service accounts | nothing | no | no | no | no |
| IAM policy | nothing | no | no | no | no |
| VPC networks | nothing | no | no | no | no |
| VPC subnets | nothing | no | no | yes | no |
| Firewall rules | nothing | no | no | no | no |

[Plugins](../../plugins/overview.md) can add your own commands to any service.

## How nuvem reads

Reads are native. Every list and detail load below is a Google Cloud
client-library call, authenticated by `gcloud config config-helper` for the
active configuration; the `gcloud` command shown with each service is the
equivalent invocation, and is what demo mode, the fallback path and a
label-mode (`-l`) read actually run. BigQuery is the exception: it always
shells out to `bq`. Mutations always run the `gcloud` command shown.

List reads are capped at 500 rows; the command shown on each page says where. Columns for each service are listed in
[Columns and sort](../../configuration/columns-and-sort.md).

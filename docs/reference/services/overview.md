---
title: All services
description: Every service nuvem can browse, the names it answers to, and what you can do in each.
sidebar:
  order: 1
---

Every service appears in the Resources sidebar and the command palette. The
Id is the name the config file uses for the service, in `[columns.<id>]`, in
`[[jumps]]` and in a plugin's `scope`. Any of the Aliases also works there, in
the sidebar search and in the command palette. Ids and aliases are
case-insensitive.

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

Status and state columns show each value as a short label, such as `Running`
or `Not ready`. When the API reports a value this version of nuvem does not
know yet, nuvem shows it the same way: `MAINTENANCE_WINDOW` appears as
`Maintenance window`, in the neutral color.

## How nuvem reads

Reads go through the Google Cloud client libraries. Every list and detail
load below is a client-library call, authenticated by
`gcloud config config-helper` for the active configuration. The `gcloud`
command shown with each service is the equivalent command, and only a
label-mode (`-l`) read runs it. BigQuery is the exception: it always runs
`bq`. For changes, see [Before a change runs](#before-a-change-runs).

List reads are capped at 500 rows. Where a list has the cap, its page's
command shows it as `--limit=500` or `-n 500`.
[Columns and sort](../../configuration/columns-and-sort.md) lists the columns
for each service.

## Before a change runs

nuvem always shows what it is about to do and waits for you to confirm it.
What it shows depends on how the change is made.

Lifecycle actions go through the Google Cloud client libraries, so there is
no command line to show. Their confirmation lists the `Action`, the
`Instance`, its `Zone` (GCE) or `Region` (Cloud SQL), the `Configuration`,
the `Project`, the `Impact`, and the `Call`: the API method and the resource
it acts on, for example `compute.instances.stop
zones/europe-west1-b/instances/web-01`. A batch on marked rows lists the same
`Call` line for every target.

| Service | Action | Call |
| --- | --- | --- |
| GCE instances | Start instance | `compute.instances.start` |
| GCE instances | Stop instance | `compute.instances.stop` |
| GCE instances | Reset instance | `compute.instances.reset` |
| GCE instances | Suspend instance | `compute.instances.suspend` |
| GCE instances | Resume instance | `compute.instances.resume` |
| Cloud SQL | Start instance | `sql.instances.patch` with `activationPolicy=ALWAYS` |
| Cloud SQL | Stop instance | `sql.instances.patch` with `activationPolicy=NEVER` |
| Cloud SQL | Restart instance | `sql.instances.restart` |
| Cloud SQL | Fail over instance | `sql.instances.failover` |

Everything else that changes something runs a `gcloud` command, or another
command-line tool, and its confirmation shows the command line:

- Activating a configuration shows `gcloud config configurations activate`
  with the configuration's name.
- SSH shows the `gcloud compute ssh` command. A tunnel shows the command that
  holds it open. See [Tunnels](../../guides/tunnels.md).
- Opening a GKE cluster in k9s shows the `gcloud container clusters
  get-credentials` command, when one is needed, and the `k9s` command.
- A plugin shows its whole command line. See
  [Plugins](../../plugins/overview.md).

Creating a configuration and editing the active context run `gcloud config`
commands when you submit the form. The form itself shows the values they set.

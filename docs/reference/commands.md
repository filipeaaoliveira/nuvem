---
title: Palette commands
description: Every command the command palette offers, its search aliases, and when it appears.
sidebar:
  order: 3
---

Press `:` to open the [command palette](../guides/command-palette.md), type
part of a command's name or one of its aliases, and press `Enter`. The palette
lists only the commands that apply to the current view and selection.

A command marked **mutating** changes a cloud resource or opens a session that
can. Under `--read-only` it stays in the list, dimmed and marked
`— read-only`, and choosing it reports the refusal.

With rows marked, the lifecycle commands act on every marked row and their
label changes to say so, for example `Stop 3 marked GCE instances`. See
[Batch actions on marked rows](../guides/batch-actions.md).

## Navigation

| Command | Aliases | Offered | Mutating |
| --- | --- | --- | --- |
| Home | `home` | always | no |
| Configurations | `configurations`, `configs`, `config` | always | no |
| Tunnels | `tunnels`, `pf`, `forward`, `port forward` | always | no |
| FinOps | `finops`, `cost`, `costs`, `savings`, `budget`, `spend` | always | no |
| Open project logs | `logs`, `logging` | when a gcloud configuration is active | no |
| Refresh | `refresh`, `reload` | in Configurations, Resources and Logging | no |
| Reload configuration | `reload config`, `config reload` | always | no |
| Help | `help` | always | no |
| Switch project / context | `project`, `context`, `switch` | when a gcloud configuration is active; opens the active-context editor in Configurations | no |
| Plugin output | `plugin output`, `output` | once a plugin run has printed something | no |

## Services

One command per service opens it in Resources. Each answers to the same
aliases as the Resources sidebar search, listed in
[All services](services/overview.md). Your own `[aliases]` also match these
commands.

| Command | Aliases | Offered | Mutating |
| --- | --- | --- | --- |
| GCE instances | `gce`, `compute engine`, `instances` | always | no |
| GKE clusters | `gke`, `kubernetes`, `clusters` | always | no |
| Cloud SQL | `cloud sql`, `sql`, `database`, `postgres`, `mysql` | always | no |
| Cloud Run | `run`, `serverless` | always | no |
| Cloud Storage | `gcs`, `storage`, `bucket`, `buckets` | always | no |
| Secret Manager | `secret`, `secrets`, `secret manager`, `sm` | always | no |
| Cloud Functions | `functions`, `gcf`, `cloud functions` | always | no |
| Persistent disks | `disks`, `pd`, `persistent disks`, `orphans` | always | no |
| Pub/Sub topics | `pubsub`, `pub/sub`, `topics`, `pubsub topics`, `pubsub-topics` | always | no |
| Pub/Sub subscriptions | `subscriptions`, `subs`, `pubsub subscriptions`, `pubsub-subscriptions` | always | no |
| BigQuery datasets | `bigquery`, `bq`, `datasets`, `bigquery-datasets` | always | no |
| Service accounts | `iam`, `sa`, `service accounts`, `service-accounts` | always | no |
| IAM policy | `iam policy`, `iam-policy`, `policy`, `bindings`, `roles` | always | no |
| VPC networks | `vpc`, `networks`, `vpc networks`, `vpc-networks` | always | no |
| VPC subnets | `subnets`, `subnetworks`, `vpc subnets`, `vpc-subnets` | always | no |
| Firewall rules | `firewall`, `firewalls`, `fw`, `firewall-rules` | always | no |

## Configurations

| Command | Aliases | Offered | Mutating |
| --- | --- | --- | --- |
| Activate selected gcloud configuration | none | in Configurations, on a configuration that is not active | no |

## GCE instances

Offered in the GCE instances list with an instance selected, not from the
detail pane. With rows marked, a lifecycle command appears when any marked
instance is eligible.

| Command | Aliases | Offered | Mutating |
| --- | --- | --- | --- |
| Start selected GCE instance | none | on a stopped instance | yes |
| Stop selected GCE instance | none | on a running instance | yes |
| Reset selected GCE instance | none | on a running instance | yes |
| Suspend selected GCE instance | none | on a running instance | yes |
| Resume selected GCE instance | none | on a suspended instance | yes |
| Show SSH command for selected GCE instance | none | on an instance; shows the command so you can copy it | no |
| SSH to selected GCE instance | `ssh` | on an instance | yes |
| Start tunnel for selected GCE instance | `tunnel`, `pf`, `forward` | on an instance | no |
| Copy selected instance name | none | on an instance | no |
| Copy selected instance zone | none | on an instance | no |
| Copy selected instance internal IP | none | on an instance with an internal IP | no |
| Copy selected instance external IP | none | on an instance with an external IP | no |
| Copy selected instance self link | none | on an instance, when the configuration has a project | no |

## Cloud SQL

Offered in the Cloud SQL list with an instance selected, not from the detail
pane. With rows marked, a lifecycle command appears when any marked instance
is eligible.

| Command | Aliases | Offered | Mutating |
| --- | --- | --- | --- |
| Start selected Cloud SQL instance | none | on a stopped instance | yes |
| Stop selected Cloud SQL instance | none | on a runnable instance | yes |
| Restart selected Cloud SQL instance | none | on a runnable instance | yes |
| Fail over selected Cloud SQL instance | none | on a runnable HA (regional) instance | yes |
| Copy selected Cloud SQL connection name | none | on an instance, when the configuration has a project | no |
| Start tunnel for selected Cloud SQL instance | `tunnel`, `pf`, `forward` | on an instance, when the configuration has a project | no |

## GKE clusters

| Command | Aliases | Offered | Mutating |
| --- | --- | --- | --- |
| Open selected GKE cluster in k9s | `k9s`, `open` | on a cluster with a location, when the configuration has a project; also from the detail pane | yes |

## Logs for the selection

These open [Cloud Logging](../guides/cloud-logging.md) scoped to the resource
on screen, like `L`. Each appears only when that service has a resource the
query can be built for. Persistent disks, service accounts, the IAM policy,
VPC networks and firewall rules have no log scope, so no such command exists
for them, and `L` there says so.

| Command | Aliases | Offered | Mutating |
| --- | --- | --- | --- |
| Open logs for selected GCE instance | `logs` | on an instance | no |
| Open logs for selected GKE cluster | `logs` | on a cluster | no |
| Open logs for selected Cloud SQL instance | `logs` | on an instance, when the configuration has a project | no |
| Open logs for selected Cloud Run service | `logs`, `logging` | on a service | no |
| Open logs for selected Cloud Run revision | `logs` | inside a revision's detail | no |
| Open logs for selected Cloud Storage bucket | `logs` | on a bucket, or inside one | no |
| Open logs for selected secret | `logs` | on a secret, when the configuration has a project | no |
| Open logs for selected Cloud Function | `logs` | on a function | no |
| Open logs for selected Pub/Sub topic | `logs` | on a topic | no |
| Open logs for selected Pub/Sub subscription | `logs` | on a subscription | no |
| Open logs for selected BigQuery dataset | `logs` | on a dataset | no |
| Open logs for selected VPC subnet | `logs` | on a subnet | no |

## Plugins

Each [plugin](../plugins/overview.md) whose templates the selected row can fill
appears under its own `label`, rendered for that row. Plugins are offered from
a Resources list with a row selected, not from a detail pane.

Under `--read-only`, a plugin runs only when nuvem recognises the start of its
`args` as a gcloud command that only reads, such as
`["compute", "instances", "describe", ...]`. Every other plugin stays in the
list, dimmed and marked `— read-only`, and choosing it reports the refusal.
See [Read-only mode](../guides/read-only-mode.md).

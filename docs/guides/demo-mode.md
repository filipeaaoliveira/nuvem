---
title: Demo mode
description: Run the full UI offline against built-in fixtures, with no project and no credentials.
sidebar:
  order: 12
---

Demo mode runs the whole interface against built-in sample data. It needs no
Google Cloud account, no project and no credentials, and it never touches a
real project or your local gcloud configurations.

```bash
nuvem --demo
```

## What you see

Every workspace shows sample data: Configurations, Compute Engine, GKE,
Cloud Run, Cloud Functions, Cloud SQL, Cloud Storage, Secret Manager,
persistent disks, Pub/Sub, BigQuery, VPC, IAM, Cloud Logging and FinOps. Each
service lists a handful of resources in several states, the way a small real
project would. The data uses fixture names such as `dev-project` and
`example.com`, addresses from private and documentation ranges, and the ranges
Google publishes for its own health checks and IAP.

Demo mode opens on [Home](home-overview.md) with every service already listed
and gcloud already checked, so the first screen shows the inventory rather
than sixteen `Not loaded` rows. Against a real project Home never loads a list
on its own; demo mode does because its lists cost nothing.

Commands that would change something — activating a configuration, starting or
stopping an instance — succeed without doing anything. Demo mode starts no
process: SSH and k9s are not opened, and the status line says so; a tunnel is
simulated and listed in Tunnels without one running; and a plugin's command is
answered from the sample data, or refused, instead of being run.

## Things to try

1. Press `g` and pick a service in the sidebar with `j`/`k` and `Enter`.
2. Press `Enter` on a row to open its detail, and `[`/`]` to change tab.
3. Press `/` and type part of a name to filter the list.
4. Press `:` and type `logs` to open Cloud Logging.
5. On a GCE instance, press `:` and choose **Stop selected GCE instance** to see
   the confirmation and the API call it would make.
6. Press `?` for every key in the current view.

## What differs from a real project

- Label filters (`-l key=value`) do not narrow the sample data.
- FinOps asks about the locations of rows you have already listed, as it does
  against a real project. Demo mode lists every service as it opens, so FinOps
  has its locations from the start.

A few reads always go through a command-line tool rather than a Google Cloud
client library: BigQuery (`bq`), FinOps budgets, gcloud's own configuration
list and account check, and label-filtered lists. In demo mode built-in sample
output answers them instead of the tool.

Demo mode is useful for trying nuvem, recording demos and screenshots, and
reproducing a bug report without sharing project data.

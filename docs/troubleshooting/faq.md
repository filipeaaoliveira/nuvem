---
title: Questions
description: Short answers about credentials, platforms, safety and why BigQuery needs bq.
sidebar:
  order: 2
---

## Does nuvem store my credentials?

No. It asks `gcloud config config-helper` for a short-lived access token and
keeps it in memory. Nothing is written to disk.

## Does nuvem run on Windows?

Only inside WSL. Process control uses POSIX APIs.

## Can nuvem change my resources without asking?

No. Every command that changes a cloud resource shows the exact command and
waits for `Enter`. `--read-only` refuses them all.

## Why does BigQuery need `bq`?

BigQuery has no generally available gcloud commands, and nuvem runs no alpha
commands. `bq` ships with the Google Cloud CLI.

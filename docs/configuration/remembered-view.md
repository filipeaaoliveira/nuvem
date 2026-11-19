---
title: Remembered last view
description: nuvem reopens the service you last used in each gcloud configuration.
sidebar:
  order: 8
---

nuvem opens on the service you last had open in Resources, for each gcloud
configuration. If you switch to Cloud SQL while `dev` is active, nuvem opens
on Cloud SQL the next time you come back to `dev`. This holds in a new
session and after a crash. Only the active service is remembered. Sort order,
filter text, the open detail tab, and the selected row reset to their own
defaults each time.

`general.default_service` is the fallback for a configuration that has no
memory yet. It decides the start service for every configuration you have
not switched services in. Once a configuration has a remembered service,
`default_service` has no effect on it.

The memory is written to `state.toml`, a file nuvem creates beside whichever
`config.toml` is in use (including one named by `--config`). It is a separate
file so that `config.toml` keeps your comments and formatting exactly as you
wrote them: an automatic rewrite of `state.toml` can never disturb them. You
do not need to open or edit `state.toml` yourself.

A newer version of nuvem can write a service name that this build does not
recognize. This build ignores that name for that configuration and falls back
to `default_service`. It keeps the name in the file the next time it saves.

nuvem records the remembered service the moment you switch, not on a clean
exit, so the memory survives a crash or a killed terminal. `--read-only` does
not stop this. That flag promises no changes to cloud resources, and nuvem
makes other local-only changes regardless of it: activating a gcloud
configuration, opening a tunnel, writing the clipboard.

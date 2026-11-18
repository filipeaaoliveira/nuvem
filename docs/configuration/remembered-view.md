---
title: Remembered last view
description: nuvem reopens the service you last used in each gcloud configuration.
sidebar:
  order: 8
---

nuvem opens on the resources service you last had open in each gcloud
configuration — switch to Cloud SQL while `dev` is active, come back to `dev`
later (even in a new session, even after a crash), and it opens on Cloud SQL
again. Only the active service is remembered: sort order, filter text, the
open detail tab, and the selected row are not, and reset to their own defaults
each time.

This changes what `general.default_service` means. It is no longer an
override applied on every launch; it is the **fallback for a configuration
that has no memory yet**. Once a configuration has a remembered service,
`default_service` no longer has any effect on it — it still decides every
configuration you have not switched services in, and still decides for
everyone until they do.

The memory is written to `state.toml`, a file the app creates beside whichever
`config.toml` is in use (including one named by `--config`). It is deliberately
a separate file from the one you hand-edit: `config.toml` keeps your comments
and formatting exactly as you wrote them, and an automatic rewrite of a second
file can never disturb that. You are not expected to open or edit `state.toml`
yourself. A service name in it that this build of nuvem does not recognise
(written by a newer version) is ignored for that configuration — it falls
back to `default_service` — and is kept rather than dropped the next time this
build saves.

The remembered service is recorded the moment you switch, not on a clean exit,
so it survives a crash or a killed terminal, not just a quit. `--read-only`
does not suppress this: that flag promises no cloud-resource mutations, and
nuvem already makes other local-only changes regardless of it (activating a
gcloud configuration, opening a tunnel, writing the clipboard).

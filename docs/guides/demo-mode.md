---
title: Demo mode
description: Run the full UI offline against built-in sample data, with no project and no credentials.
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

Every workspace shows sample data: Configurations, every service in
Resources, Cloud Logging and FinOps. Each service lists a handful of
resources in several states, like a small real project. The data uses example
names such as `dev-project` and `example.com`. Its addresses come from private
and documentation ranges, and from the ranges Google publishes for its own
health checks, IAP and Private Google Access (`199.36.153.8/30`).

Demo mode opens on [Home](home-overview.md) with every service already listed
and gcloud already checked, so the first screen shows the inventory instead
of sixteen `Not loaded` rows. Against a real project, Home never loads a list
on its own. Demo mode does, because its lists cost nothing.

Commands that change resources, such as starting or stopping an instance,
report success but do nothing. Configurations are the exception. Activating
one, creating one or editing the active context takes effect for the rest of
the session, and Home and the header follow the change. nuvem keeps the
change in memory and never saves it, so your own gcloud configurations stay
untouched. The next demo session starts on `dev` again.

If you set a project that the sample data does not contain, every list shows
an error. Demo mode starts no process, except your system's clipboard tool
when you copy a value:

- SSH and k9s do not open, and the status line says so.
- A tunnel is simulated. Tunnels lists it, but nothing runs.
- A plugin's command is answered from the sample data, or refused. It never
  runs.

## Things to try

1. Press `g` and pick a service in the sidebar with `j`/`k` and `Enter`.
2. Press `Enter` on a row to open its detail, and `[`/`]` to change tab.
3. Press `/` and type part of a name to filter the list.
4. Press `:` and type `logs` to open Cloud Logging.
5. On a GCE instance, press `:` and choose **Stop selected GCE instance** to see
   the confirmation and the API call it names.
6. Press `?` for every key in the current view.

## What differs from a real project

- Label filters (`-l key=value`) do not narrow the sample data.
- FinOps asks about the locations of rows you have already listed, as it does
  against a real project. Demo mode lists every service as it opens, so FinOps
  has its locations from the start.

A few reads always go through a command-line tool rather than a Google Cloud
client library: BigQuery (`bq`), FinOps budgets, gcloud's own configuration
list and account check, and label-filtered lists. In demo mode, built-in
sample output answers them instead of the tool.

Use demo mode to try nuvem, to record demos and screenshots, or to reproduce a
bug report without sharing project data.

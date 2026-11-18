---
title: Service accounts
description: List the project's IAM service accounts.
sidebar:
  order: 13
---

For service accounts, pressing `r` runs:

```bash
gcloud --configuration <active-config> [--project <project>] iam service-accounts list --format=json --limit=500
```

The columns are EMAIL, DISPLAY NAME, STATE and UNIQUE ID. The email is the
account's identity; in `[columns]` and in templates it is called `name`.

STATE reads as a word, so `/disabled` lists the disabled accounts. Filtering
is entirely client-side here: the state is a word nuvem paints, not a value in
any raw field, and service accounts carry no labels, so `-l k=v` reports that label
filtering is not supported here.

The list is list-only and read-only: `Enter` does nothing on a row, and nuvem
never creates, disables or deletes an account, or touches its keys. To see who
holds which role, open [IAM policy](iam-policy.md).

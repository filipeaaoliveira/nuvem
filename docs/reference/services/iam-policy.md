---
title: IAM policy
description: Read the project IAM policy as one row per role and member.
sidebar:
  order: 14
---

For the project IAM policy, pressing `r` runs:

```bash
gcloud --configuration <active-config> --project <project> \
  projects get-iam-policy <project> --format=json
```

The project is named positionally there, not with `--project` alone, so a
configuration with no project set says it has no policy to read rather than
shelling out to a command that would fail with gcloud's own wording. There is
no `--limit`: the call returns one policy object rather than a list, so a limit
would cap the number of policies, not the number of bindings inside one.

That object is flattened into a table of one row per (role, member) pair. A
binding granting one role to three members is three rows; a member holding four
roles is four rows. The shape is what lets one table answer both audit
questions with `/`: filter by a member to see what that principal can do, and
filter by a role to see who holds it. The columns are MEMBER, ROLE, TYPE (the
member's kind — user, group, service account, domain, principal, principal set,
or one of the two public populations) and CONDITION, which shows a conditional
binding's title, not its CEL expression, and a dash on the unconditional
majority.

The role is shown verbatim, exactly as the policy spells it. A custom role
reads as `projects/<project>/roles/editor` where the predefined role it shadows
reads as `roles/editor`, and shortening either would make the two
indistinguishable in the column an audit reads down.

The member cell leads with the badges the row wears:

- `PUBLIC` — the grant is held by `allUsers` (anyone on the internet, with or
  without a Google account) or by `allAuthenticatedUsers` (any authenticated
  Google account, including accounts outside your organisation).
- `DELETED` — the principal was deleted after the binding was written, so the
  grant names an identity that no longer exists.

Both are stale policy rather than errors, so a badged row is coloured as a
warning, the way an orphaned disk is. Both are also text `/` can match:
`/PUBLIC` isolates the badged rows because the badge is a searched field of its
own, and `/deleted` does the same because the member itself arrives prefixed
`deleted:` and the filter is case-insensitive. The badges lead the cell rather
than trail it because the member column is the one that elides first on a
narrow terminal, and a trailing badge would be the first thing lost on exactly
the rows it exists to mark.

The policy table is list-only and read-only: `Enter` does nothing on a row,
there is no detail pane, and nuvem never issues `set-iam-policy`. Editing an
IAM policy is out of scope for this service.

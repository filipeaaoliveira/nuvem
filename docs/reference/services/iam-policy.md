---
title: IAM policy
description: Read the project IAM policy as one row per role and member.
sidebar:
  order: 14
---

For the project IAM policy, `r` reloads the table. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> --project <project> \
  projects get-iam-policy <project> --format=json
```

The command names the project positionally, not with `--project` alone. With
no project set in the configuration, nuvem says it has no policy to read, and
does not run a command that would fail with gcloud's own wording. There is no
`--limit`. The call returns one policy object, not a list, so a limit would cap
the number of policies and not the number of bindings inside one.

nuvem flattens that object into a table of one row per (role, member) pair. A
binding that grants one role to three members is three rows. A member that
holds four roles is four rows. With this shape, one table answers both audit
questions with `/`: filter by a member to see what that principal can do, and
filter by a role to see who holds it.

The columns are MEMBER, ROLE, TYPE and CONDITION. TYPE is the member's kind:
user, group, service account, domain, principal, principal set, or one of the
two public populations. CONDITION shows a conditional binding's title, not its
CEL expression, and a dash on the unconditional majority.

The role is shown verbatim, exactly as the policy spells it. A custom role
reads as `projects/<project>/roles/editor`, and the predefined role it shadows
reads as `roles/editor`. If either were shortened, the two would look the same
in the column an audit reads down.

The member cell leads with the badges the row wears:

- `PUBLIC` — the grant is held by `allUsers` (anyone on the internet, with or
  without a Google account) or by `allAuthenticatedUsers` (any authenticated
  Google account, including accounts outside your organisation).
- `DELETED` — the principal was deleted after the binding was written, so the
  grant names an identity that no longer exists.

Both are stale policy, not errors, so a badged row is coloured as a warning,
the way an orphaned disk is. `/` can match both. `/PUBLIC` isolates the badged rows
because the badge is a searched field of its own. `/deleted` does the same because the member itself arrives prefixed
`deleted:` and the filter is case-insensitive. The badges lead the cell because
the member column is the one that elides first on a narrow terminal. A trailing
badge would be the first thing lost on the rows it exists to mark.

The policy table is list-only and read-only. `Enter` does nothing on a row,
there is no detail pane, and nuvem never issues `set-iam-policy`. Editing an
IAM policy is out of scope for this service.

---
title: Firewall rules
description: List firewall rules with their action, targets and source or destination ranges.
sidebar:
  order: 17
---

The Firewall rules list shows every firewall rule in the project, with what it
allows or denies, which instances it targets, and its source or destination
ranges.

For firewall rules, `r` reloads the list. The equivalent `gcloud` command is:

```bash
gcloud --configuration <active-config> --project <project> \
  compute firewall-rules list --format=json --limit=500
```

The list is flat and project-wide, like the other two VPC tables. A firewall
rule is a *global* resource that names the network it applies to, so there is
no per-network fan-out and no `--network` flag. NETWORK is a column you sort
and filter by, not a scope you enter.

The columns are NAME, NETWORK, DIRECTION, PRIORITY, ACTION, TARGETS and
SOURCE/DEST.

ACTION is derived. The API reports no action field. It reports an `allowed`
array or a `denied` array, and exactly one of the two is ever present. nuvem
shows `ALLOW` or `DENY` from whichever array is there.

A rule that carries neither key, or both, shows a dash and no verdict. A wrong
`ALLOW` or `DENY` is worse than a blank cell that you go and check. On a
narrow terminal, ACTION is the last column dropped after NAME.

A disabled rule is badged on its name and gets no column of its own. A
disabled DENY rule looks exactly like a live DENY rule in every other cell:
same name, same direction, same priority, same ranges. The traffic you believe
is blocked can then be flowing. So `DISABLED` goes in the name column, which
the layout never drops.

The badge leads the cell, because a long name is cut from the right. For the
same reason, the preview pane heads itself `rule-name (DISABLED)`.

PRIORITY sorts numerically, and ascending priority is evaluation order. Lower
priority wins, so that is the order in which traffic meets the rules. A rule
whose listing reported no priority sorts as absent. Sorting it as zero would
put it ahead of every real rule, as if it were evaluated first.

The cell shows a dash and not the API's `1000` default, for the same reason
that the networks table shows a dash for an absent routing mode.

TARGETS is the rule's target tags or target service accounts, whichever scopes
it. A rule with neither applies to every instance in the network, which is the
widest scope there is, so the cell reads `all instances`. A blank there would
read as the narrowest scope and invert the fact.

SOURCE/DEST follows the direction: source ranges on an INGRESS rule,
destination ranges on an EGRESS one. Each direction uses only one of the two.

NETWORK shows the network's name, not the self-link URL the API sends, as in
the other two VPC tables.

`/` narrows wholly client-side. `ALLOW`, `DENY`, `all instances` and the
`DISABLED` badge are all words nuvem puts on the screen and no raw field
contains, so a filter sent to the server would drop rows you can read. All
four are searchable, which is what lets `/DENY` answer "what does this project
block".

Label filtering (`-l k=v`) reports unsupported, because the `compute/v1`
firewalls resource has no labels field.

As on the other two VPC tables, applying a filter makes no call. A refresh
that fails clears the rows. So if you typed `/DENY` to ask what a project
blocks, a short failure of a needless call would give you a blank table
instead of the answer. `r` still fetches, because `r` is an explicit request.

The rules table is list-only and read-only. `Enter` does nothing on a row,
there is no detail pane, and nuvem never creates, deletes, enables or disables
a rule. The one action that would be useful here is also the most dangerous one
nuvem can offer.

`L` is refused, as on the networks table, for a stronger version of the same
reason. Cloud Logging does log a rule's hits, but against the subnetwork the
traffic crossed, with the rule named in the payload. No entry has the rule as
its resource. A rule-scoped query would return zero rows, and on a DENY rule
zero rows reads as "nothing was blocked".

`tests/fixtures/vpc_firewall_rules.json` is the one *mixed* fixture in this
group. Its four `default-allow-*` rules are a live capture taken on 2026-08-25
with the project id replaced by `nuvem-fixture-project`. The other three,
`allow-web-to-tagged`, `deny-all-egress` and `disabled-sa-scoped-ssh`, are
constructed from Google's published `compute/v1/firewalls` reference. No
project the capturing account can reach has an egress rule, a `denied` rule, a
target-tagged rule, a service-account-scoped rule or a disabled rule. Every
interesting parser arm in this table therefore rests on transcription rather
than on bytes gcloud produced, including the two whose misrendering is
dangerous, and the tests that use them say so.

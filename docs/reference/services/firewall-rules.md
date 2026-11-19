---
title: Firewall rules
description: List firewall rules with their action, targets and source or destination ranges.
sidebar:
  order: 17
---

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

ACTION is derived, and it is the most important cell in this table. The API
reports no action field. It reports an `allowed` array or a `denied` array, and
exactly one of the two is ever present. The parser collapses that into one
value, so no later code can find the pair in a state the API never sends and
guess at it. A rule that carries neither key, or both, paints a dash and no
verdict. `ALLOW` over a rule that blocks, or `DENY` over one that does not, is
worse than a blank cell that you go and check. For this reason ACTION sits
immediately behind NAME in the drop order. A list of rules that cannot say
what any of them does is no use.

A disabled rule is badged on its name and gets no column of its own. A
disabled DENY rule looks exactly like a live DENY rule in every other cell:
same name, same direction, same priority, same ranges. Meanwhile, the traffic
you believe is blocked is flowing. So `DISABLED` goes in the identity column,
which is the one column the layout can never drop. It leads the cell, because
the name elides from the right. For the same reason, the preview pane heads
itself `rule-name (DISABLED)`.

PRIORITY sorts numerically, and ascending priority is evaluation order. Lower
priority wins, so that is the order in which traffic meets the rules. A rule
whose listing reported no priority sorts as absent, not as zero. Zero would put
it ahead of every real rule and claim that it is evaluated first, which is a
statement about a value nothing reported. The cell shows a dash and not the
API's `1000` default, for the same reason that the networks table shows a dash
for an absent routing mode.

TARGETS is the rule's target tags or target service accounts, whichever scopes
it. An empty list is not a blank. A rule with neither applies to every instance
in the network, which is the widest scope there is, so the cell reads
`all instances`. A blank there would read as the narrowest scope and invert the
fact. SOURCE/DEST follows the direction: source ranges on an INGRESS rule,
destination ranges on an EGRESS one. Each direction uses only one of the two.

NETWORK is painted as an id, not as the self-link URL the API sends. The other
two VPC tables use the same helper.

`/` narrows wholly client-side. `ALLOW`, `DENY`, `all instances` and the
`DISABLED` badge are all words nuvem puts on the screen and no raw field
contains, so a pushed clause would drop rows you can read. All four are
searchable, which is what lets `/DENY` answer "what does this project block".
Label filtering (`-l k=v`) reports unsupported, because the `compute/v1`
firewalls resource has no labels field.

As on the other two VPC tables, applying a filter makes no call. This table
needs that most. A refresh that fails clears the rows. If you typed `/DENY` to
ask what a project blocks, any short failure of a needless call would give you
a blank table instead of the answer. `r` still fetches, because `r` is an
explicit request.

The rules table is list-only and read-only. `Enter` does nothing on a row,
there is no detail pane, and nuvem never creates, deletes, enables or disables
a rule. The one action that would be useful here is also the most dangerous one
nuvem could offer.

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

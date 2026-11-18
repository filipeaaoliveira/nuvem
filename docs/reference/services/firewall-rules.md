---
title: Firewall rules
description: List firewall rules with their action, targets and source or destination ranges.
sidebar:
  order: 17
---

For firewall rules, pressing `r` runs:

```bash
gcloud --configuration <active-config> --project <project> \
  compute firewall-rules list --format=json --limit=500
```

Flat and project-wide like the other two VPC tables. A firewall rule is a
*global* resource that names the network it applies to, so there is no
per-network fan-out and no `--network` flag; NETWORK is a column you sort and
filter by rather than a scope you enter.

The columns are NAME, NETWORK, DIRECTION, PRIORITY, ACTION, TARGETS and
SOURCE/DEST.

**ACTION is derived, and it is the cell this table exists for.** The API
reports no action field. It reports an `allowed` array or a `denied` array, and
exactly one of the two is ever present; the parser collapses that into one
value, so no later reader can find the pair in a state the API never sends and
guess at it. A rule carrying neither key, or both, paints a dash rather than a
verdict: `ALLOW` over a rule that blocks, or `DENY` over one that does not, is
worse than a blank cell an operator goes and checks. ACTION therefore sits
immediately behind NAME in the drop order — a list of rules that cannot say
what any of them does says nothing at all.

**A disabled rule is badged on its name, not given a column.** A disabled DENY
rule reads exactly like a live DENY rule in every other cell — same name, same
direction, same priority, same ranges — while the traffic an operator believes
is blocked is flowing. So `DISABLED` rides the identity column, which is the
one column the layout can never drop, and it leads the cell rather than
trailing it, because the name elides from the right. The preview pane heads
itself `rule-name (DISABLED)` for the same reason.

PRIORITY sorts numerically, and ascending by it is evaluation order: lower
priority wins, so that is the order traffic actually meets the rules in. A rule
whose listing reported no priority sorts as absent rather than as zero — zero
would put it ahead of every real rule and claim it is evaluated first, which is
a statement about a value nothing reported. The cell dashes rather than
painting the API's `1000` default, for the reason the networks table dashes an
absent routing mode.

TARGETS is the rule's target tags or target service accounts, whichever scopes
it. An empty list is **not** a blank: a rule with neither applies to every
instance in the network, which is the widest scope there is, so the cell reads
`all instances`. A blank there would read as the narrowest scope and invert the
fact. SOURCE/DEST follows the direction — source ranges on an INGRESS rule,
destination ranges on an EGRESS one — because the two are never both the
question.

NETWORK is painted as an **id**, not as the self-link URL the API sends, using
the same helper the other two VPC tables use.

`/` narrows wholly client-side. `ALLOW`, `DENY`, `all instances` and the
`DISABLED` badge are all words this application puts on the screen and no raw
field contains, so a pushed clause would drop rows an operator can read. All
four are searchable, which is what lets `/DENY` answer "what does this project
block" at all. Label filtering (`-l k=v`) reports unsupported: the `compute/v1`
firewalls resource has no labels field.

Applying a filter runs no gcloud command, as on the other two VPC tables — and
this is the table where that matters most. A refresh that fails clears the
rows, so an operator who types `/DENY` to ask what a project blocks would, on
any transient failure of a call that was never needed, get a blank table back
instead of the answer. `r` still fetches, because `r` is an explicit ask.

The rules table is list-only and read-only: `Enter` does nothing on a row,
there is no detail pane, and nuvem never creates, deletes, enables or disables
a rule. The one action worth having here would be the most dangerous button in
the application.

`L` is refused, as it is on the networks table and for a sharper version of the
same reason. A rule's hits *are* logged — against the subnetwork the traffic
crossed, with the rule named in the payload — so nothing writes an entry whose
resource is the rule. A rule-scoped query would return zero rows, and on a DENY
rule zero rows reads as "nothing was blocked".

`tests/fixtures/vpc_firewall_rules.json` is the one **mixed** fixture in this
group. Its four `default-allow-*` rules are a live capture taken on 2026-08-25
with the project id replaced by `nuvem-fixture-project`. The other three —
`allow-web-to-tagged`, `deny-all-egress` and `disabled-sa-scoped-ssh` — are
constructed from Google's published `compute/v1/firewalls` reference, because
no project the capturing account can reach has an egress rule, a `denied` rule,
a target-tagged rule, a service-account-scoped rule or a disabled rule. Every
interesting parser arm in this table therefore rests on transcription rather
than on bytes gcloud produced, including the two whose misrendering is
dangerous, and the tests that use them say so.

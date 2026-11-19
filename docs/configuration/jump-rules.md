---
title: Jump rules
description: Write [[jumps]] rules that decide what the related-resource picker offers.
sidebar:
  order: 7
---

`J` opens the related-resource jump picker (see
[Related-resource jumps](../guides/related-resource-jumps.md)). A list of
`[[jumps]]` rules decides which jumps it offers. Each array entry is one row
the picker can show:

```toml
[[jumps]]
from = "compute"
to = "compute:network"
name = "{name}"
location = "{zone}"
label = "Network of {name}"
```

| key        | required | meaning |
|------------|----------|---------|
| `from`     | yes      | the service the jump is offered from |
| `to`       | yes      | where it lands: a detail tab as `"<service>:<tab>"`, or a service list as `"<service>"` |
| `name`     | no       | template naming the resource to open; `"{name}"` by default |
| `location` | no       | template for that resource's zone/region/location; the source row's own by default, and not accepted at all when the destination identifies its rows by name alone |
| `label`    | no       | the picker row's wording; `"<tab> of <name>"` for a detail tab and `"<resource> <name>"` for a list, by default |
| `select`   | no       | narrows the destination list to a computed set instead of landing on one row |

`from` takes the same service vocabulary as `[columns]` and `[aliases]`
(`compute`/`gce`, `gke`, `cloudsql`, `run`/`cloudrun`, `storage`, …). A `to`
with a `:` in it is a service and one of that service's detail tabs. Its
service half takes that same vocabulary: `to = "gce:disks"` and
`to = "compute:disks"` name the same tab. The tab half is exact. It is the tab
name the `[`/`]` tab bar shows, lowercased, with spaces hyphenated. The
following table spells each service the canonical way:

| service    | destination tabs |
|------------|------------------|
| `compute`  | `compute:summary`, `compute:network`, `compute:disks`, `compute:metadata`, `compute:json` |
| `gke`      | `gke:summary`, `gke:networking`, `gke:node-pools`, `gke:addons`, `gke:json` |
| `cloudsql` | `cloudsql:summary`, `cloudsql:connectivity`, `cloudsql:maintenance`, `cloudsql:backups`, `cloudsql:replicas`, `cloudsql:users`, `cloudsql:json` |
| `cloudrun` | `cloudrun:summary`, `cloudrun:traffic`, `cloudrun:revisions`, `cloudrun:configuration`, `cloudrun:conditions`, `cloudrun:json` |

A `to` with no `:` in it names a service list, not a detail tab. The jump
switches to that service's table and puts the cursor on the row that `name`
resolves to. Fifteen of the sixteen services are legal list destinations,
including the twelve whose tabs a jump cannot name. You can write
`to = "vpc-networks"` and `to = "secrets"`, for example:

```toml
[[jumps]]
from = "firewall-rules"
to = "vpc-networks"
name = "{network}"
```

Cloud Storage has no detail tabs. Its `Enter` opens an object browser, not a
tabbed pane. `to = "storage:objects"` thus names nothing, and a rule that
writes it is reported as such, not as an unknown tab. `to = "storage"` is a
legal list destination. Secret Manager, Cloud Functions and Pub/Sub topics are
the same shape. A jump cannot name a tab of theirs, so they have no
`"<service>:<tab>"` destination, but they are list destinations like every
other service. Cloud
Functions has no detail pane at all, by design. The reason is recorded with
the function list under
[Cloud Functions](../reference/services/cloud-functions.md).

The IAM policy table is the one service that is neither a tab destination nor
a list destination. A binding is identified by its member *and* its role,
because two bindings can share a member, and no `name`/`location` pair renders
a role. `to = "iam-policy"` is refused at load with that reason, so it never
becomes a rule that lands nowhere. It is still a legal `from`.

A list jump lands on a row, not on a filtered table. `to = "firewall-rules"`
with a `{network}` name does not show the rules on that network. It puts the
cursor on the rule *called* that.

To get a list narrowed to a computed set instead, a rule sets `select`. There
is one computation, the built-in instance-to-firewall jump:

```toml
[[jumps]]
from = "compute"
to = "firewall-rules"
select = "targets-this-instance"
```

It shows the firewall rules that target the instance. These are the rules on
its network whose target tags or target service accounts the instance carries,
plus every rule on that network that names no target at all, because those
apply to every instance in it. A narrowing rule ignores `name` and `location`.
nuvem computes the set from the instance, and no template names it, so there
is no single row for either key to point at. An empty result is an answer (no
rule governs this instance), not a row that has gone.

You can still filter the narrowed table with `/`, and the header says which
instance it is narrowed to. It does not narrow by
whether a rule is in force. A DENY rule and a disabled rule both still target
the instance, so both stay in the list. `select` is
only defined from `compute` to `firewall-rules`. nuvem rejects any other
pairing at load.

`secrets`, `pubsub-topics`, `pubsub-subscriptions`, `service-accounts`,
`iam-policy`, `vpc-networks` and `firewall-rules` are the services with no
location field at all. A secret, a topic, a subscription, a service account,
the project policy, a VPC network and a firewall rule are all global, so their
rows are identified by name alone. A rule to one of these lists must not
carry a `location`. If it does, nuvem rejects it at load with that reason, and
the template never silently renders an empty string.

`name`, `location` and `label` are templates: literal text with `{field}`
placeholders filled from the row you press `J` on. The fields that exist
depend on the *source* service, so a template is checked against `from`:

| `from`     | fields |
|------------|--------|
| `compute`  | `{name}`, `{zone}`, `{machine_type}`, `{status}`, `{internal_ip}`, `{external_ip}`, `{network}`, `{subnetwork}`, `{subnetwork_region}` |
| `gke`      | `{name}`, `{location}` |
| `cloudsql` | `{name}`, `{region}`, `{engine}`, `{tier}`, `{status}` |
| `cloudrun` | `{name}`, `{region}` |
| `storage`  | `{name}`, `{location}` |
| `secrets`  | `{name}` |
| `functions` | `{name}`, `{region}` |
| `disks`    | `{name}`, `{location}` |
| `pubsub-topics` | `{name}` |
| `pubsub-subscriptions` | `{name}` |
| `bigquery-datasets` | `{name}`, `{location}` |
| `service-accounts` | `{name}` |
| `iam-policy` | `{name}` |
| `vpc-networks` | `{name}` |
| `vpc-subnets` | `{name}`, `{location}`, `{network}` |
| `firewall-rules` | `{name}`, `{network}` |

`{network}`, `{subnetwork}` and `{subnetwork_region}` on `compute` are the only
fields that do not come from the row. A GCE instance's list row carries no
network at all. The
values live on the detail pane, so a rule that names any of them resolves once
that pane is open and fetched, and offers nothing from the list. `J` tells
this case apart from a missing value. It says `needs network, which
is on this GCE instance's detail — open it and try again`. For an instance
whose fetched pane has no network interface, it says the ordinary
`has no value for`. An instance with several interfaces reports its first,
the same one `{internal_ip}` and `{external_ip}` come from.

`{subnetwork_region}` exists because `{subnetwork}` does not identify a subnet
on its own. An auto-mode network calls every one of its subnets `default`, one
per region, so a rule keyed on the name alone names as many rows as the
project has regions. The region is read out of the subnetwork's self-link, from
the segment following `regions`, not by stripping the last part off the
instance's `{zone}`. Stripping the zone gives the right answer on most
topologies, but nothing in the API guarantees that an instance's subnet is in
the region its zone belongs to. A link that names no region leaves the field
absent, so a jump keyed on it offers nothing instead of landing on the wrong
subnet.

There is no `{network_region}` beside it. A network's self-link is
`.../global/networks/<name>`: a network is global within its project, so the
name is the whole identity and nothing is discarded when it is shortened.

Substitution is a single pass. A value that itself contains `{` or `}` is
written out as it stands and never expanded again. There is no escape for a
literal brace.

In the following example, the built-in Network row stays, the Disks row is
reworded, and every GCE instance also offers a row for the GKE cluster named
after it. That last row lands only under the conditions at the end of this
page:

```toml
[[jumps]]
from = "compute"
to = "compute:network"

[[jumps]]
from = "compute"
to = "compute:disks"
label = "Disks attached to {name}"

[[jumps]]
from = "compute"
to = "gke:node-pools"
name = "{name}-cluster"
location = "{zone}"
label = "Node pools of the {name} cluster"
```

A `[[jumps]]` array replaces the built-in set. It does not merge with it,
so that you can remove a built-in jump. This is why the example repeats the
Network rule. Without any `[[jumps]]` entry, you get the built-in rules (the
table under [Related-resource jumps](../guides/related-resource-jumps.md)).
With `jumps = []`, no jump is configured at all, and `J` only reports that
none are configured for the service you are in.

nuvem validates each rule on its own. It drops a rule with any of these, with a
warning that names the rule's index: an unknown `from` or `to`, an unknown
key, a value of the wrong type, an empty or unbalanced template, or a
`{field}` its `from` service does not have. Only that rule goes, and the rules
beside it keep working:

```
jumps[2]: unknown destination tab "compute:netwrok"; ignoring this rule
```

Jump warnings appear like any other config warning: the first one in the
startup status line, all of them on stderr and under `nuvem info`. Reload (`:` →
**Reload configuration**) reads the whole array again while nuvem runs.

A valid rule can still have nothing to offer for a particular row. No config
check can know that *this* instance has no external IP. If the rule's `name`
or `location` renders empty for the selected row, that rule is
not offered for that row. The other rules still are. If nothing at all
resolves, the picker stays closed, and the status line names the rule and the
field:

```
Jump "compute:network" needs external_ip, which this GCE instance has no value for
```

A `label` is wording, not identity, so it is treated differently. If a label
does not render for the selected row, it
falls back to the built-in wording (`"<tab> of <name>"`), and the jump is
still offered.

The rendered `name` and `location` must also look like GCP resource names
(non-empty, at most 253 characters, `A-Z a-z 0-9 . _ -`), because they end up as
`gcloud describe` arguments. A rule whose rendered name fails that check is not
offered either, and the status line says so:

```
Jump "compute:disks" renders an invalid name for this GCE instance
```

Use `{status}` only in a `label`. A status is not a resource name, so a rule
that selects on one names nothing that exists. Statuses worded with a space
(Cloud SQL's `Pending create`, or a new status such as `Maintenance window`)
fail the check outright, so such a rule is offered for some rows and refused
for others.

A jump that *is* offered lands only on a resource
already in the destination service's loaded list, matched on both its
name and its zone/region. The picker never fetches a single resource into
existence to land on. At most, it starts loading a list that was never loaded,
the first case below. If there is no such row, the status line says which
case it is:

```
Loading GKE clusters, so web-01-cluster cannot be opened yet — try again once they arrive
No GKE cluster loaded yet, so web-01-cluster cannot be opened
web-01-cluster is no longer available
```

- The first is a destination service this session has never loaded. The jump
  starts its list load. Repeat the jump once the rows are in, and it lands.
- The second is a list that is not loaded and is not being fetched: a load
  that failed *in this configuration*, waiting on the retry backoff.
  A failure inherited from a configuration you have since left does not count.
  That load never ran in this configuration, so the jump fetches.
- The third is a list that is loaded and does not have that row in it.

The cross-service rule in the example thus lands only once the GKE list is
fetched and holds a cluster called `web-01-cluster` whose location is exactly
the instance's zone. That means a zonal cluster in `europe-west1-b`, not a
regional one in `europe-west1`, because `location = "{zone}"` hands over the
zone verbatim. Cross-service rules work where your names and locations line
up across services. Where they do not, the picker row still appears, and the
jump reports why instead of landing.

---
title: Service aliases
description: Add your own names for services in the [aliases] table.
sidebar:
  order: 4
---

The `[aliases]` table adds your own names for services. An alias works in
two places:

- the service sidebar search (`/` with the sidebar focused);
- the command palette (`:`), where it finds the command that opens the
  service.

The two places do not match the same built-in names today:

- The sidebar search matches each service's label, its category, and every
  alias in the **Aliases** column of [Services](../reference/services/overview.md).
- The command palette matches each command's label and its own palette
  aliases. Only five services have palette aliases: GCE instances, GKE
  clusters, Cloud SQL, Cloud Storage and Cloud Run; see
  [Palette commands](../reference/commands.md#services). For the others, type
  part of the label: `:secret` finds **Secret Manager**, but `:sm` does not.

An alias you add in `[aliases]` works in both places.

```toml
[aliases]
vms = "gce"
k8s = "gke"
warehouse = "bigquery"
```

The key is the new name. The value names an existing service, in the same
vocabulary `[columns]` and `[[jumps]]` use: a service id such as `compute` or
`vpc-networks`, one of its built-in aliases, or its label such as
`"Cloud SQL"`. Matching is case-insensitive.

## Rules

- An alias is stored lowercased and trimmed. An empty alias is dropped with a
  warning.
- A value that names no service drops that alias with a warning.
- An alias that is already a built-in alias of a *different* service is
  dropped with a warning: `sql = "gke"` cannot take `sql` away from Cloud SQL.
  Repeating a built-in alias for the same service is harmless and kept.
- Each bad entry is dropped on its own; the rest of the table still applies.

[Reload configuration](reload.md) applies alias changes without a restart.

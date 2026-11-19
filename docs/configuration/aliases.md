---
title: Service aliases
description: Add your own names for services in the [aliases] table.
sidebar:
  order: 4
---

The `[aliases]` table adds your own names for services. An alias works in
two places:

- The service sidebar search (`/` with the sidebar focused).
- The command palette (`:`), where it finds the command that opens the
  service.

Both places match the built-in aliases in the **Aliases** column of
[Services](../reference/services/overview.md), so `:sm` and `/sm` both find
**Secret Manager**:

- The sidebar search matches each service's label, its category, and every
  alias.
- The command palette matches each command's label and every alias, but not
  the category. See [Palette commands](../reference/commands.md#services).

```toml
[aliases]
vms = "gce"
k8s = "gke"
warehouse = "bigquery"
```

The key is the new name. The value names an existing service, in the same
vocabulary `[columns]` and `[[jumps]]` use: a service ID such as `compute` or
`vpc-networks`, one of its built-in aliases, or its label such as
`"Cloud SQL"`. Matching is case-insensitive.

## Rules

- nuvem stores an alias lowercased and trimmed. It drops an empty alias with
  a warning.
- If a value names no service, nuvem drops that alias with a warning.
- If an alias is already a built-in alias of a *different* service, nuvem
  drops it with a warning: `sql = "gke"` cannot take `sql` away from Cloud
  SQL. An alias that repeats a built-in alias of the same service is
  harmless, and nuvem keeps it.
- nuvem drops each bad entry on its own. The rest of the table still applies.

[Reload configuration](reload.md) applies alias changes without a restart.

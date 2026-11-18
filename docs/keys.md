# Key reference

Every key nuvem binds, in one place. The first seven are rebindable through the
config file's `[hotkeys]` table — see
[configuration.md](configuration.md#hotkeys).

Initial keys (`home`, `configurations`, `resources`, `refresh`, `help`,
`quit`, and `command_palette` are rebindable via the config file's
`[hotkeys]` table — see [Configuration](configuration.md#configuration)):

- `h`: home
- `c`: configurations
- `g`: resources
- `l`: the Logging workspace, on all recent project logs (never the selection —
  that is `Shift-L`)
- `Ctrl-t`: tunnels (the managed-tunnel list). Deliberately not a bare `t`,
  which is the Logging tail toggle
- `Ctrl-x`: cancel every running plugin, from anywhere. A plugin's argv
  is whatever the operator declared, so unlike a lifecycle action it can wait
  forever; this is what stops it. It resolves ahead of the mode match, like
  `Ctrl-C`, so no modal, palette, help view, filter or armed plugin prefix
  swallows it — the run it kills outlived the view it started in, and the key
  is not scoped to one either. The
  entry it leaves in the operations log reads `Unknown` if its command had
  already been issued, because a killed child may have changed something before
  it died
- `v`: show or hide the Resources service sidebar
- `r`: refresh current workspace
- `a`: create gcloud configuration from the Configurations workspace
- `e`: edit active gcloud context from the Configurations workspace
- `Ctrl-Space`: discover the focused Project, Region, or Zone while editing the active context
- `:`: open the command palette
- `/`: search services on Home or in the focused Resources sidebar; edit the
  active resource or configuration filter when content is focused; edit the
  Logging search when the Logging list is focused (see
  [Cloud Logging](features.md#cloud-logging) for its grammar)
- `u`: clear active configuration filter
- `←`/`→`: move the selected column in the resource list
- `s`: sort by the selected column (press again to reverse direction); raise
  the Logging severity floor in the Logging list
- `S`: lower the Logging severity floor
- `w`: toggle wide columns in the resource list; toggle line wrapping in the
  Logging workspace
- `Space`: mark or unmark the selected row in a mark-capable service (GCE
  instances, Cloud SQL instances)
- `Shift-V`: range-mark from the mark anchor to the selected row
- `\`: clear all marks
- `Shift-U`: unmark only the currently visible (filter-matching) marks
- `t`: on a GCE instance or Cloud SQL row, start a tunnel to the selected
  resource (a preview shows the exact command first); in the Logging
  workspace, pause or resume the tail
- `Shift-L`: in a resource list or resource detail pane, open Cloud Logging
  scoped to the resource on screen (see [Cloud Logging](features.md#cloud-logging)).
  Distinct from `l`, which opens the Logging workspace on all project logs
- `Shift-J`: in a resource list or resource detail pane, open the
  related-resource jump picker for the resource on screen; `j`/`k` move,
  `Enter` jumps, `Esc` cancels (see
  [Related-resource jumps](features.md#related-resource-jumps))
- `,`: in a resource list or resource detail pane, arm the plugin prefix — the
  status line names the keys this service's plugins declare, and the next key
  you press runs that plugin through its confirmation; `Esc` disarms (see
  [Plugins](plugins.md#plugins))
- `Shift-K`: kill the selected tunnel in the Tunnels workspace
- `d`: dismiss a stopped or failed row in the Tunnels workspace
- `T`: cycle the Logging since window
- `0`-`6`: select a Logging since window directly (`1m`, `5m`, `15m`, `30m`,
  `1h`, `6h`, `24h`)
- `Ctrl-S`: save the visible Logging entries to a file
- `i`: copy the selected Logging entry's insert ID
- `PageUp`/`PageDown`: scroll an open Logging entry's detail
- `Tab`: move focus between the Resources sidebar and resource content,
  including in resource detail; in create/edit forms, move to the next field
- `Shift-Tab`: move to the previous create/edit form field
- `j` / `k` or the arrows: on Home or in the focused Resources sidebar, move
  through matching services; elsewhere move through configurations, resource
  rows, or Logging entries (in the command palette they type into the search
  query; use the arrow keys to move through palette entries)
- `Enter`: on Home or in the focused Resources sidebar, open the selected
  service directly; otherwise create a configuration, save an active-context edit, request
  activation for the selected configuration, apply a filter draft, open selected
  resource detail, drill into a BigQuery dataset's tables and from there into a
  table's column schema, open or close a
  Logging entry's detail, choose a palette
  command, confirm a visible action preview, run the plugin a visible plugin
  confirmation is showing, copy the previewed SSH command line from its
  preview, or launch an SSH session from its preview
- `y`: confirm a configuration activation when its preview is visible; copy
  the selected Logging entry (from the list or an open entry's detail); in the
  Cloud Storage bucket list or object browser, copy the selected
  bucket/prefix/object `gs://` URI;
  `n`: cancel a configuration activation
- `i`: in the Cloud Storage service, open metadata for the selected bucket
- `n`: in the Cloud Storage object browser, load the next bounded page
- `-` / `Backspace`: in the Cloud Storage object browser, move up one prefix
  level; in either BigQuery drill-down, ascend one level — the schema view for
  the tables list, the tables list for the datasets — which is the same thing
  `Esc` does there
- `[` / `]`: cycle resource detail tabs
- `n` / `p`: in a resource detail pane, move the field cursor to the next /
  previous copyable field, wrapping at both ends
- `y`: in a resource detail pane, copy the selected field's value; with no
  field selected, copy the whole document when a JSON tab is open, or the
  `gs://` URI of the bucket/object being viewed
- `PgUp` / `PgDn`: in a resource detail pane, page through the body; paging also
  releases the field cursor, which is how a selected field is deselected
- `?`: help
- `Esc`: close help, cancel a configuration activation, cancel create/edit forms
  or filter drafts, close the command palette, cancel a pending GCE action,
  cancel an SSH command preview, a plugin's input prompt, a plugin confirmation
  or an SSH launch preview; close an open Logging entry's detail, or cancel the
  Logging filter draft; in either BigQuery drill-down with no filter draft
  open, ascend one level — one press leaves a table's schema for the tables
  list, the next leaves that for the datasets — since entering a row pushes no
  history entry and there is none to walk back; anywhere else in the
  Resources workspace (list or detail), the
  Logging list, or the Tunnels list,
  go back to the prior view via navigation history
- `Alt-Left` / `Alt-Right`: go back/forward through navigation history from
  anywhere in the Resources workspace
- `q`: in resource detail or an open Logging entry's detail, return to the
  list; from home and list views, quit
- `Ctrl-C`: quit from any mode

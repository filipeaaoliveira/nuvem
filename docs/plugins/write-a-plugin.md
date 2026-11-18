---
title: Write a plugin
description: Add a gcloud command of your own to a service, step by step.
sidebar:
  order: 2
---

This adds a **Describe** command to the GCE instances list. It runs
`gcloud compute instances describe` against the selected instance.

1. Find where nuvem looks for the plugin file:

   ```bash
   nuvem info
   ```

   The `plugins:` line names the path, for example
   `~/.config/nuvem/plugins.toml (not found)`.

2. Create that file with one plugin:

   ```toml
   version = 1

   [[plugins]]
   id    = "describe"
   scope = "compute"
   key   = "d"
   label = "Describe {name}"
   args  = ["compute", "instances", "describe", "{name}", "--zone", "{zone}"]
   ```

   `scope` names the service. `{name}` and `{zone}` are filled from the
   selected row. [Plugin file reference](plugin-file-reference.md) lists every
   field and the template fields each service has.

3. Make sure only you can write it:

   ```bash
   chmod o-w ~/.config/nuvem/plugins.toml
   ```

4. Run `nuvem info` again. The line should now read `(1 loaded)`. If it reads
   `(0 loaded)`, the warning below it says what is wrong.

5. Start nuvem, open GCE instances (`g`, then pick the service), and select an
   instance. Then either:

   - press `,` and then `d`, or
   - press `:`, type `describe`, and press `Enter`.

6. nuvem shows the exact command, for example:

   ```text
   gcloud --configuration dev compute instances describe web-01 --zone europe-west1-b
   ```

   Press `Enter` to run it or `Esc` to cancel.

7. To read what it printed, press `:` and choose **Plugin output**. See
   [Plugin output](plugin-output.md).

The run appears in Home's **Recent operations**. To stop a run that does not
end, press `Ctrl-x`.

## Ask for a value

To ask for a value the row does not have, add an input. This resizes a disk to
a size you type:

```toml
[[plugins]]
id    = "grow-disk"
scope = "disks"
key   = "G"
label = "Grow {name} to {new_size}GB"
args  = ["compute", "disks", "resize", "{name}", "--size", "{new_size}"]

[[plugins.inputs]]
name    = "new_size"
type    = "integer"
default = 200
```

See [Typed inputs](typed-inputs.md).

## Mark a plugin as dangerous

Add `dangerous = true` to a plugin that deletes or changes something you
cannot undo. Its confirmation then asks you to type `1` before `Enter` runs it.

## Changes need a restart

nuvem reads the plugin file once, at startup. After editing it, quit and start
nuvem again.

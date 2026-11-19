---
title: Plugin output
description: Read what a plugin run printed.
sidebar:
  order: 4
---

The Recent operations entry says that a run finished, not what the run printed.
To read that, run **Plugin output** from the command palette (`:`). It opens
what the command printed, as lines:

```text
┌plugin output──────────────────────────────────────────┐
│Describe web-01 · Succeeded                            │
│name: web-01                                           │
│machineType: e2-standard-2                             │
│status: RUNNING                                        │
└run 1 of 3 · line 1 of 84 · ↑↓ scroll · n/p run · y ───┘
```

| Key | Action |
| --- | --- |
| `↑`/`↓` (or `j`/`k`) | Scroll one line. |
| `PgUp`/`PgDn` | Scroll twenty lines. |
| `n`/`p` | Move between the runs Recent operations still holds, most recent first. Stops at either end and does not wrap. |
| `y` | Copy the run on screen to the clipboard. |
| `Esc` | Close the viewer. |

A run shorter than one page does not scroll at all. A longer one stops with
its tail on screen, not above a screen of blank rows.

The palette offers **Plugin output** only once some run has printed
something, so choosing it never opens an empty box. A run that failed before
it printed anything (for example, with no gcloud on `PATH`) settles in Recent
operations with no output to attach, and the palette entry stays absent.

nuvem keeps both streams. stderr has a reserved share of the size bound, so a
`--format=json` listing far past the bound cannot silence the warning printed
beside it. A line the command wrote to stderr is shown marked `stderr │`, even
for a run that exited 0, so a quota warning printed beside a successful answer
is not lost. `y` copies the marks too, so you can tell a failed run's error
text from its answer in the clipboard.

A run that *failed* keeps its stderr as well. That is the case with the most
to read. The Recent operations detail line is bounded at 200 bytes, and the
error text gcloud prints is often longer.

nuvem keeps at most 256 KiB per run, and Recent operations holds ten records. Past that
bound, nuvem keeps the *head*. `gcloud` puts the identifying fields first, so
a truncated `describe` still names what it described. The last line says how
much was dropped. A single line longer than the bound (`--format=json` prints
one) is cut mid-line on a character boundary, not dropped. Nothing after the
cut is shown, even a line that fits, so the head never has a hole in it. `y`
copies what was kept.

nuvem redacts output as it arrives, by the same rule the status line uses. A
token that carries a credential becomes `[redacted]`. A token that only names
something (`name: db-password`) is left alone. Indentation survives, because
YAML output is read by its shape. Control characters do not survive.

Plugin output lives in memory only, and only for as long as its record is in
Recent operations, which keeps ten operations, or until you quit. Nothing is written to disk. A run
you are *reading* survives the eviction of its record. The viewer holds the
copy it opened, and says `run no longer in the log` instead of a position
among runs it has left.

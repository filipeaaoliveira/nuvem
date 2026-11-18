---
title: Plugin output
description: Read what a plugin run printed.
sidebar:
  order: 4
---

The operations-log entry says a run finished; it does not say what the run
said. **Plugin output** in the command palette (`:`) opens what the command
printed, as lines:

```text
┌plugin output──────────────────────────────────────────┐
│Describe web-01 · Succeeded                            │
│name: web-01                                           │
│machineType: e2-standard-2                             │
│status: RUNNING                                        │
└run 1 of 3 · line 1 of 84 · ↑↓ scroll · n/p run · y ───┘
```

`↑`/`↓` (or `j`/`k`) scroll a line, `PgUp`/`PgDn` twenty — a run shorter than
one page does not scroll at all, and a longer one stops with its tail on
screen rather than above a screen of blank rows — and `n`/`p` move
between the runs the log still holds — most recent first, stopping at either
end rather than wrapping. `y` copies the run on screen to the clipboard and
`Esc` closes. The command is offered only once some run has actually printed something, so
choosing it never opens an empty box: a run that failed before producing any
output — no gcloud on `PATH`, say — settles in the log with no output to
attach, and the entry stays absent.

Both streams are kept, and stderr has a reserved share of the bound, so a
`--format=json` listing far past it cannot silence the warning printed beside
it. A line the command wrote to stderr is shown marked `stderr │`, including
for a run that exited 0 — a quota warning printed alongside a successful
answer is not lost. `y` copies the marks too, so a failed run's trailer stays
distinguishable from its answer in the clipboard. A run that *failed* keeps its
stderr too, which is the case with the most to read: the log's own detail line
is bounded at 200 bytes, and a gcloud error trailer is routinely longer than
that.

What is kept is bounded: 256 KiB per run, and the log holds ten records. Past
that bound the **head** is kept — `gcloud` puts the identifying fields first,
so a truncated `describe` still names what it described — and the last line
says how much was dropped rather than simply ending. A single line longer than
the bound (`--format=json` prints one) is cut mid-line rather than dropped, on
a character boundary. Nothing after the cut is shown even if it would have
fitted, so the head never has a hole in it. `y` copies what was kept.

Output is redacted on the way in, by the same rule the status line uses: a
token that carries a credential becomes `[redacted]`, while one that merely
names something (`name: db-password`) is left alone. Indentation survives,
because YAML output is read by its shape; control characters do not.

Plugin output lives in memory only, and only for as long as its record is in
the log — ten operations, or until you quit. Nothing is written to disk. A
run you are *reading* survives its record being evicted: the viewer holds the
copy it opened, and says `run no longer in the log` instead of claiming a
place among runs it has left.

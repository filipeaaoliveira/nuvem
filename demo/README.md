# Demo recordings and frames

Everything here comes from `nuvem --demo`, which runs the whole UI against the
built-in fixtures: no Google Cloud account, and no names but fixture ones such
as `dev-project` and `dev@example.com`.

| Path | What it is |
|---|---|
| `tour.tape` | The [VHS](https://github.com/charmbracelet/vhs) script for the recorded tour |
| `out/nuvem-demo.gif` | README recording |
| `out/nuvem-demo.webm`, `out/nuvem-demo.mp4` | Website hero video, no audio track |
| `out/nuvem-demo-poster.png` | Still frame: the video poster and the reduced-motion fallback |
| `frames/*.html` | Single screens as coloured HTML text, for the docs pages |

The files in `out/` and `frames/` are committed, because the README and the
website use them as they are.

Re-rendering always changes the GIF and the videos byte for byte, even when
nothing on screen changed: VHS sends each key a few milliseconds earlier or
later from run to run. So commit new recordings only when the tour shows
something different, such as a UI change that appears in it or an edit to the
tape. Otherwise restore them with `git checkout -- demo/out`. The frames in
`frames/` are text, so they change only when the screen does.

## What you need

- A nuvem binary. Both scripts use `NUVEM_BIN` if it is set (a path, or a
  command name on `PATH`), then `target/release/nuvem`, then
  `target/debug/nuvem`. Run `cargo build --release` first if you have neither.
- For the recordings: [VHS](https://github.com/charmbracelet/vhs), with its
  two dependencies `ttyd` and `ffmpeg`. VHS's README lists the packages for
  each platform: Homebrew on macOS and Linux, Charm's apt and yum
  repositories, and release binaries. Most Linux distributions package `ttyd`
  and `ffmpeg` themselves.
- For the frames: `tmux` and `python3`.

## Render the recordings

```bash
scripts/render-demo.sh
```

This runs `tour.tape` and writes the GIF, both videos and the poster to
`demo/out/`. It takes about two minutes, most of it VHS encoding. The script
checks each file against its size budget before copying anything into
`demo/out/`: 3 MB for the GIF and 1.5 MB for each video. A file over budget
fails the run and leaves the committed files as they were. If you lengthen the
tour, keep it under 25 seconds.

The recording is 1280 by 760 pixels, which at the tape's 16 px font is a
terminal of 121 columns by 34 rows.

### Isolation

Both scripts start the app through a small launcher that runs it under
`env -i`, with `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` and
`XDG_DATA_HOME` all pointing into a scratch directory. Only `PATH`, `TERM` and
the locale (`LANG`, `LC_ALL`, `LC_CTYPE`) pass through. nuvem's config file,
plugin file, saved state, log and cache all resolve inside the scratch
directory, so your own setup never reaches a recording or a frame, and no
`NUVEM_*`, `CLOUDSDK_*` or `GOOGLE_APPLICATION_CREDENTIALS` variable reaches
the app. In the recordings, the command that starts the launcher is typed
while VHS is hidden, so no shell prompt or path appears either.

### Font

The tape asks for `JetBrains Mono,Menlo,DejaVu Sans Mono,monospace`. This is
a CSS font list: the terminal uses the first font in it that is installed,
character by character. The fallback is intended. All three named fonts
cover the box-drawing characters nuvem draws with and have nearly the same
advance width at 16 px, so the terminal stays 121 by 34 whichever one is
used, and only the letterforms differ. Requiring one font would stop anyone
without it from rendering at all. The committed recordings use Menlo. If you
want them to look the same on every machine, install JetBrains Mono before
rendering.

To change the tour, edit `tour.tape`. Its comments name each step. Leave a
second or more on each screen: the recording is for someone seeing nuvem for
the first time.

## Capture a frame for the docs

```bash
scripts/capture-frame.sh home            # one screen
scripts/capture-frame.sh detail help     # several
scripts/capture-frame.sh --all           # every screen
```

The screens are `home`, `resources`, `detail`, `palette` and `help`. Each one
is written to `demo/frames/<screen>.html` as a single `<pre>` element with
inline colours, ready to paste into a page. `FRAME_COLS` and `FRAME_ROWS`
change the terminal size; the default, 121 by 34, matches the recordings.

The script starts the demo in a private tmux server, whose socket lives in a
scratch directory that is removed when the script exits. It presses the keys
listed for that screen in `keys_for` and passes `tmux capture-pane -e` output
to `scripts/ansi2html.py`. If the app exits before the screen is captured,
the script stops and prints the app's stderr. To add a screen, add a line to `keys_for` and its name
to `screens`.

`ansi2html.py` also works on its own:

```bash
tmux capture-pane -e -p -t <pane> | scripts/ansi2html.py > frame.html
```

`ansi2html.py` carries its own test cases: colour forms, out-of-range
values, escape sequences it must drop, control characters and dim text. Run
them after changing it:

```bash
scripts/ansi2html.py --self-test
```

## Colours

nuvem draws with the 16 ANSI colours, so the terminal palette decides how it
looks. The tape and `ansi2html.py` use the same palette, Wada Sanzo's
combination 47 from *A Dictionary of Color Combinations*:

| ANSI colour | Hex | nuvem uses it for |
|---|---|---|
| background | `#1b1816` | |
| foreground | `#f1ece6` | body text |
| cyan, green | `#54afb4` | titles, focused borders, key hints, healthy status |
| yellow | `#fbe6a0` | active tab, sort column, pending status |
| red | `#e2625e` | error status |
| magenta | `#f58e84` | overlay borders, changed values |
| blue | `#423b36` | selected row background |
| bright black | `#8c8178` | borders, muted text |

If you change one, change it in both files.

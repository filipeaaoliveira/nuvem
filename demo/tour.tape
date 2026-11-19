# A short tour of `nuvem --demo`, rendered by scripts/render-demo.sh.
#
# Run it through that script rather than with `vhs` directly: the script picks
# the binary, writes the launcher this tape types (it runs the app with a
# scrubbed environment, so no local config reaches the recording), prepends
# the terminal colours (the palette in scripts/ansi2html.py), and turns VHS's
# raw capture into the published GIF, WebM, MP4 and poster.
#
# The tour: Home, the GCE instances list, a filter, an instance's detail and
# its Network tab, a Stop up to its confirmation (then cancelled), the command
# palette jumping to BigQuery by its alias and into a dataset's tables, help,
# and Home again.
#
# 138 columns is the narrowest terminal where the context line names every
# field of every screen in the tour, with "BigQuery datasets" the longest
# service in it. Cloud SQL, Cloud Run and Secret Manager shorten some cells
# at this width, so the tour leaves them out.

Set Shell "bash"
Set FontFamily "JetBrains Mono,Menlo,DejaVu Sans Mono,monospace"
Set FontSize 16
Set LineHeight 1.15
Set Width 1444
Set Height 760
Set Padding 18
Set Framerate 24
Set TypingSpeed 90ms
Set CursorBlink false

# Start the app off camera, so no shell prompt or path appears in a frame.
Hide
Type 'clear; "$NUVEM_DEMO_LAUNCHER"'
Enter
Sleep 2.5s
Show

# Keys that repaint most of the screen are pressed off camera, with a short
# pause for the repaint to finish: a terminal mid-repaint shows half of each
# screen, and a frame captured then reads as a glitch. Typing stays on camera.

# Home: the active context and every service's inventory.
Sleep 2.2s

# Resources opens on GCE instances, with the service navigator beside them.
# The poster frame: a whole list, nothing cut.
Hide
Type "g"
Sleep 0.2s
Show
Sleep 0.3s
Screenshot poster.png
Sleep 1.9s

# Hide the navigator to give the columns room, and filter the list.
Hide
Type "v"
Sleep 0.2s
Show
Sleep 0.5s
Type "/"
Sleep 0.3s
Type "web"
Sleep 0.4s
Hide
Enter
Sleep 0.2s
Show
Sleep 2.2s

# An instance's detail, then its Network tab.
Hide
Enter
Sleep 0.2s
Show
Sleep 2.7s
Hide
Type "]"
Sleep 0.2s
Show
Sleep 2.7s
Hide
Escape
Sleep 0.2s
Show
Sleep 0.4s

# The command palette offers what the selected instance takes. Stop shows the
# API call and its impact before anything runs; Esc cancels it.
Hide
Type ":"
Sleep 0.2s
Show
Sleep 0.3s
Type "stop"
Sleep 1s
Hide
Enter
Sleep 0.2s
Show
Sleep 3s
Hide
Escape
Sleep 0.2s
Show
Sleep 0.5s

# The palette also jumps to any service by its short name.
Hide
Type ":"
Sleep 0.2s
Show
Sleep 0.3s
Type "bq"
Sleep 0.8s
Hide
Enter
Sleep 0.2s
Show
Sleep 2.2s

# Into a dataset: its tables. Their row counts and sizes arrive a moment after
# the names; the pause off camera lets them land before the view is shown.
Hide
Enter
Sleep 1s
Show
Sleep 2.7s

# Help lists every key the current view takes.
Hide
Type "?"
Sleep 0.2s
Show
Sleep 2.8s
Hide
Escape
Sleep 0.2s
Show
Sleep 0.5s

# Home again. The app restarts off camera on the way, so the last frame is
# the first one and the loop has no seam: the tour itself would leave a count
# trend beside the services it loaded and an older "Updated" time.
Hide
Type "h"
Sleep 0.3s
Type "q"
Sleep 0.5s
Type 'clear; "$NUVEM_DEMO_LAUNCHER"'
Enter
Sleep 2.5s
Show
Sleep 1.2s

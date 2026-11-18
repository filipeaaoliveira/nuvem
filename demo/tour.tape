# A short tour of `nuvem --demo`, rendered by scripts/render-demo.sh.
#
# Run it through that script rather than with `vhs` directly: the script picks
# the binary, writes the launcher this tape types (it runs the app with a
# scrubbed environment, so no local config reaches the recording), and turns
# VHS's raw capture into the published GIF, WebM, MP4 and poster.
#
# The tour: home, the service navigator, a resource list, a filter, a detail
# view with its tabs, the command palette, help, and back home, where the
# inventory now counts what the tour loaded.

Set Shell "bash"
Set FontFamily "JetBrains Mono,Menlo,DejaVu Sans Mono,monospace"
Set FontSize 16
Set LineHeight 1.15
Set Width 1280
Set Height 760
Set Padding 18
Set Framerate 24
Set TypingSpeed 140ms
Set CursorBlink false

# Wada Sanzo, combination 47. nuvem draws with the 16 ANSI colours only, so the
# terminal palette is what themes the app.
Set Theme { "name": "nuvem-47", "background": "#1b1816", "foreground": "#f1ece6", "cursor": "#fbe6a0", "selection": "#423b36", "black": "#1b1816", "red": "#e2625e", "green": "#54afb4", "yellow": "#fbe6a0", "blue": "#423b36", "magenta": "#f58e84", "cyan": "#54afb4", "white": "#f1ece6", "brightBlack": "#8c8178", "brightRed": "#f58e84", "brightGreen": "#9fd3d5", "brightYellow": "#fbe6a0", "brightBlue": "#54afb4", "brightMagenta": "#f58e84", "brightCyan": "#9fd3d5", "brightWhite": "#faf7f3" }

# Start the app off camera, so no shell prompt or path appears in a frame.
Hide
Type 'clear; "$NUVEM_DEMO_LAUNCHER"'
Enter
Sleep 2.5s
Show

# Home: the active context, all fixture data.
Sleep 1.6s

# Resources opens on GCE instances.
Type "g"
Sleep 1.1s

# The service navigator: focus it and search for a service.
Tab
Sleep 0.5s
Type "/"
Sleep 0.3s
Type "run"
Sleep 0.6s
Enter
Sleep 0.3s
Enter
Sleep 1.2s

# Back to the list, with the navigator hidden to give the columns room.
Tab
Sleep 0.3s
Type "v"
Sleep 1s

# Filter the list.
Type "/"
Sleep 0.3s
Type "api"
Sleep 0.5s
Enter
Sleep 1s

# Open a service's detail, then its Traffic tab.
Type "j"
Sleep 0.5s
Enter
Sleep 1.6s
Type "]"
Sleep 1.4s
Escape
Sleep 0.5s

# The command palette jumps anywhere.
Type ":"
Sleep 0.5s
Type "sql"
Sleep 0.8s
Enter
Sleep 1.2s

# The poster frame. The pause after it keeps the next key out of the shot.
Screenshot poster.png
Sleep 0.4s

# Help lists every key the current view takes.
Type "?"
Sleep 2s
Escape
Sleep 0.4s

# Home again: the inventory now counts what the tour loaded.
Type "h"
Sleep 2.6s

#!/usr/bin/env python3
"""Turn ANSI-coloured terminal text into an HTML <pre> fragment.

Reads text with SGR escapes on stdin (for example `tmux capture-pane -e -p`)
and writes one <pre> element to stdout. Colours are inline styles, so the
fragment needs no stylesheet.

SGR handling: the 16 ANSI colours, 256-colour indexes and truecolour, in both
the semicolon form (38;5;n, 38;2;r;g;b) and the colon form (38:5:n,
38:2::r:g:b, 38:2:r:g:b); bold, dim, italic, underline and reverse video. An
index or component out of range is ignored rather than turned into CSS.
Every other escape sequence (other CSI sequences such as cursor movement or
private modes, OSC titles and hyperlinks, DCS and the like) is dropped, and so
are control characters other than newline and tab.

The 16-colour palette below is also the recordings' terminal theme:
scripts/render-demo.sh asks for it with --vhs-theme and hands it to VHS, so a
frame on a docs page and the recordings share one definition.

Usage: scripts/ansi2html.py [--class NAME] < capture.txt > frame.html
       scripts/ansi2html.py --vhs-theme
       scripts/ansi2html.py --self-test
"""

import argparse
import html
import json
import re
import sys

# Wada Sanzo, combination 47. nuvem draws with the 16 ANSI colours only, so
# this palette is what themes the app in the recordings and the frames.
BACKGROUND = "#1b1816"
FOREGROUND = "#f1ece6"
# The terminal's own cursor and mouse selection. nuvem hides the cursor, and
# the recordings never select text, so neither shows in a frame.
CURSOR = "#fbe6a0"
SELECTION = "#423b36"

# ANSI 0-7, then 8-15 (the bright variants), in the order VHS names them.
ANSI_NAMES = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    "brightBlack", "brightRed", "brightGreen", "brightYellow",
    "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
]
# nuvem paints red only as text (error statuses), and magenta both as overlay
# borders and as changed-value text, so both are Coral Red. Eugenia Red would
# be fine for a border or a fill but falls under 4.5:1 as text, and no slot
# here is painted only as a border or a fill. Blue is the selected row's
# background, and bright black the muted text and the borders.
PALETTE = [
    "#1b1816", "#f58e84", "#64b5b9", "#fbe6a0",
    "#423b36", "#f58e84", "#64b5b9", "#f1ece6",
    "#afa69e", "#f58e84", "#9fd3d5", "#fbe6a0",
    "#64b5b9", "#f58e84", "#9fd3d5", "#faf7f3",
]
# Slots nuvem paints as backgrounds rather than text: black and blue.
BACKGROUND_SLOTS = (0, 4)
# The background of the selected row.
SELECTED_ROW = PALETTE[4]
# WCAG AA for normal text, the size every cell in a terminal frame is.
MIN_CONTRAST = 4.5

# How far dim text moves from its colour toward the background behind it.
DIM_TOWARD_BACKGROUND = 0.4

# One token per escape sequence. Only an SGR (CSI ... m) is acted on; the rest
# are matched so they can be dropped whole instead of leaking as text.
ESCAPE = re.compile(
    r"\x1b\[(?P<csi_params>[0-?]*)(?P<csi_inter>[ -/]*)(?P<csi_final>[@-~])"  # CSI
    r"|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)"  # OSC, ended by BEL or ST
    r"|\x1b[PX^_][^\x1b]*(?:\x1b\\)"  # DCS, SOS, PM, APC, ended by ST
    r"|\x1b[ -/]*[0-~]"  # any other escape (charset, keypad mode, ...)
    r"|\x1b"  # a lone or truncated escape
)
# C0 and C1 controls, apart from newline and tab.
CONTROL = re.compile(r"[\x00-\x08\x0b-\x1f\x7f-\x9f]")


def indexed(n):
    """The colour for a 256-colour index, as xterm defines 16-255."""
    if not 0 <= n <= 255:
        return None
    if n < 16:
        return PALETTE[n]
    if n < 232:
        n -= 16
        steps = [0, 95, 135, 175, 215, 255]
        return "#%02x%02x%02x" % (steps[n // 36], steps[n // 6 % 6], steps[n % 6])
    grey = 8 + (n - 232) * 10
    return "#%02x%02x%02x" % (grey, grey, grey)


def rgb(components):
    if len(components) != 3 or any(c is None or not 0 <= c <= 255 for c in components):
        return None
    return "#%02x%02x%02x" % tuple(components)


def blend(colour, toward, amount):
    a = [int(colour[i:i + 2], 16) for i in (1, 3, 5)]
    b = [int(toward[i:i + 2], 16) for i in (1, 3, 5)]
    return "#%02x%02x%02x" % tuple(round(x + (y - x) * amount) for x, y in zip(a, b))


def number(text):
    return int(text) if text.isdigit() else None


class Style:
    def __init__(self):
        self.reset()

    def reset(self):
        self.fg = None
        self.bg = None
        self.bold = False
        self.dim = False
        self.italic = False
        self.underline = False
        self.reverse = False

    def set_colour(self, code, colour):
        if colour is None:
            return
        if code == 38:
            self.fg = colour
        else:
            self.bg = colour

    def apply(self, params):
        groups = params.split(";") if params else [""]
        i = 0
        while i < len(groups):
            group = groups[i]
            if ":" in group:
                # Colon form: the whole extended colour sits in one group.
                subs = group.split(":")
                code = number(subs[0])
                if code in (38, 48) and len(subs) >= 3:
                    if subs[1] == "5":
                        self.set_colour(code, indexed(number(subs[2]) if subs[2] else -1))
                    elif subs[1] == "2":
                        # 38:2:<colour space>:r:g:b, or 38:2:r:g:b without it.
                        parts = subs[3:6] if len(subs) >= 6 else subs[2:5]
                        self.set_colour(code, rgb([number(p) for p in parts]))
                elif code == 4:
                    # 4:0 turns underline off; 4:1 to 4:5 are underline styles.
                    self.underline = subs[1:2] != ["0"]
                i += 1
                continue
            code = number(group) if group else 0
            if code is None:
                i += 1
                continue
            if code == 0:
                self.reset()
            elif code == 1:
                self.bold = True
            elif code == 2:
                self.dim = True
            elif code == 3:
                self.italic = True
            elif code == 4:
                self.underline = True
            elif code == 7:
                self.reverse = True
            elif code == 22:
                self.bold = self.dim = False
            elif code == 23:
                self.italic = False
            elif code == 24:
                self.underline = False
            elif code == 27:
                self.reverse = False
            elif 30 <= code <= 37:
                self.fg = PALETTE[code - 30]
            elif 40 <= code <= 47:
                self.bg = PALETTE[code - 40]
            elif 90 <= code <= 97:
                self.fg = PALETTE[code - 90 + 8]
            elif 100 <= code <= 107:
                self.bg = PALETTE[code - 100 + 8]
            elif code == 39:
                self.fg = None
            elif code == 49:
                self.bg = None
            elif code in (38, 48) and i + 1 < len(groups):
                mode = number(groups[i + 1])
                if mode == 5 and i + 2 < len(groups):
                    self.set_colour(code, indexed(number(groups[i + 2]))
                                    if groups[i + 2].isdigit() else None)
                    i += 2
                elif mode == 2 and i + 4 < len(groups):
                    self.set_colour(code, rgb([number(g) for g in groups[i + 2:i + 5]]))
                    i += 4
            i += 1

    def css(self):
        fg, bg = self.fg, self.bg
        if self.reverse:
            fg, bg = bg or BACKGROUND, fg or FOREGROUND
        if self.dim:
            # Fade the text toward what is behind it; the background keeps
            # its own colour.
            fg = blend(fg or FOREGROUND, bg or BACKGROUND, DIM_TOWARD_BACKGROUND)
        rules = []
        if fg:
            rules.append("color:" + fg)
        if bg:
            rules.append("background:" + bg)
        if self.bold:
            rules.append("font-weight:700")
        if self.italic:
            rules.append("font-style:italic")
        if self.underline:
            rules.append("text-decoration:underline")
        return ";".join(rules)


def luminance(colour):
    """WCAG relative luminance of a #rrggbb colour."""
    def channel(value):
        value /= 255
        return value / 12.92 if value <= 0.03928 else ((value + 0.055) / 1.055) ** 2.4
    r, g, b = (int(colour[i:i + 2], 16) for i in (1, 3, 5))
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def contrast(a, b):
    """WCAG contrast ratio of two #rrggbb colours."""
    hi, lo = sorted((luminance(a), luminance(b)), reverse=True)
    return (hi + 0.05) / (lo + 0.05)


def vhs_theme():
    """The palette as a VHS theme, the JSON a tape's `Set Theme` takes."""
    theme = {"name": "nuvem-47", "background": BACKGROUND, "foreground": FOREGROUND,
             "cursor": CURSOR, "selection": SELECTION}
    theme.update(zip(ANSI_NAMES, PALETTE))
    return json.dumps(theme, separators=(", ", ": "))


def convert(text):
    """The HTML for `text`, without the enclosing <pre>."""
    style = Style()
    runs = []  # [css, text] pairs, adjacent runs with equal css merged

    def emit(chunk):
        chunk = CONTROL.sub("", chunk)
        if not chunk:
            return
        css = style.css()
        if runs and runs[-1][0] == css:
            runs[-1][1] += chunk
        else:
            runs.append([css, chunk])

    pos = 0
    for match in ESCAPE.finditer(text):
        emit(text[pos:match.start()])
        if match.group("csi_final") == "m" and not match.group("csi_inter"):
            params = match.group("csi_params")
            if not params[:1] in ("<", "=", ">", "?"):
                style.apply(params)
        pos = match.end()
    emit(text[pos:])

    return "".join(
        '<span style="%s">%s</span>' % (css, html.escape(chunk, quote=False)) if css
        else html.escape(chunk, quote=False)
        for css, chunk in runs
    )


# (input, expected output of convert) pairs, run by --self-test.
SELF_TEST = [
    ("plain <&>", "plain &lt;&amp;&gt;"),
    ("\x1b[31mred\x1b[0m", '<span style="color:#f58e84">red</span>'),
    ("\x1b[38;5;8mgrey", '<span style="color:#afa69e">grey</span>'),
    ("\x1b[38;5;196mx", '<span style="color:#ff0000">x</span>'),
    ("\x1b[38;5;244mx", '<span style="color:#808080">x</span>'),
    # An index past 255 sets nothing, and the sequence is still consumed.
    ("\x1b[38;5;300mx", "x"),
    ("\x1b[38:5:300mx", "x"),
    ("\x1b[38;2;1;2;3mx", '<span style="color:#010203">x</span>'),
    ("\x1b[38;2;1;2;999mx", "x"),
    # Colon forms, with and without the colour-space field.
    ("\x1b[38:2::1:2:3mx", '<span style="color:#010203">x</span>'),
    ("\x1b[38:2:1:2:3mx", '<span style="color:#010203">x</span>'),
    ("\x1b[48:5:1mx", '<span style="background:#f58e84">x</span>'),
    ("\x1b[38:5:8;1mx", '<span style="color:#afa69e;font-weight:700">x</span>'),
    # Private modes, cursor movement and erase are dropped whole.
    ("a\x1b[?25hb\x1b[?1049lc", "abc"),
    ("a\x1b[2;5Hb\x1b[2Kc\x1b[>4;1md", "abcd"),
    # OSC: a window title and a hyperlink, ended by BEL and by ST.
    ("\x1b]0;title\x07text", "text"),
    ("\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\", "link"),
    ("\x1b]2;title\x1b\\text", "text"),
    # Other escapes: charset selection, keypad mode, DCS, a trailing ESC.
    ("\x1b(Ba\x1b=b\x1bP1$r\x1b\\c\x1b", "abc"),
    # Stray control characters go; newline and tab stay.
    ("a\x07b\x08c\rd\x00e\x9bf\tg\nh", "abcdef\tg\nh"),
    # Dim moves the text toward the background, which keeps its colour.
    ("\x1b[2mx", '<span style="color:#9b9793">x</span>'),
    ("\x1b[2;44mx", '<span style="color:#aba5a0;background:#423b36">x</span>'),
    ("\x1b[7mx", '<span style="color:#1b1816;background:#f1ece6">x</span>'),
]


def palette_problems():
    """Every text colour that reads under MIN_CONTRAST on a background nuvem
    puts it on: the terminal background and the selected row."""
    problems = []
    texts = [FOREGROUND] + [c for i, c in enumerate(PALETTE) if i not in BACKGROUND_SLOTS]
    for colour in dict.fromkeys(texts):
        for ground in (BACKGROUND, SELECTED_ROW):
            ratio = contrast(colour, ground)
            if ratio < MIN_CONTRAST:
                problems.append("%s on %s is %.2f:1" % (colour, ground, ratio))
    return problems


def self_test():
    failures = 0
    for given, expected in SELF_TEST:
        got = convert(given)
        if got != expected:
            failures += 1
            print("FAIL %r\n  expected %r\n  got      %r" % (given, expected, got))
    print("%d of %d cases passed" % (len(SELF_TEST) - failures, len(SELF_TEST)))
    problems = palette_problems()
    for problem in problems:
        print("FAIL palette: %s, under %.1f:1" % (problem, MIN_CONTRAST))
    if not problems:
        print("every text colour reaches %.1f:1 on the background and the selected row"
              % MIN_CONTRAST)
    return 1 if failures or problems else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--class", dest="cls", default="terminal-frame",
                        help="class attribute for the <pre> (default: terminal-frame)")
    parser.add_argument("--self-test", action="store_true",
                        help="run the built-in conversion cases and the palette's "
                             "contrast check, and exit")
    parser.add_argument("--vhs-theme", action="store_true",
                        help="print the palette as a VHS theme (JSON) and exit")
    args = parser.parse_args()
    if args.self_test:
        sys.exit(self_test())
    if args.vhs_theme:
        print(vhs_theme())
        return
    body = convert(sys.stdin.read().rstrip("\n"))
    sys.stdout.write(
        '<pre class="%s" style="background:%s;color:%s">%s</pre>\n'
        % (html.escape(args.cls), BACKGROUND, FOREGROUND, body)
    )


if __name__ == "__main__":
    main()

---
name: nuvem
description: A terminal UI for Google Cloud. Documentation site and shared tokens.
colors:
  ground: "#423b36"
  naples-yellow: "#fbe6a0"
  coral-red: "#f58e84"
  pale-methyl-blue: "#54afb4"
  eugenia-red: "#e2625e"
  plate: "#1b1816"
  plate-raised: "#2c2622"
  text: "#f1ece6"
  text-muted: "#cbc1b6"
  rule: "#5e554e"
  rule-strong: "#9a8f84"
  on-accent: "#1b1816"
  ink: "#1b1816"
  ink-soft: "#423b36"
typography:
  display:
    fontFamily: "Schibsted Grotesk Variable, -apple-system, BlinkMacSystemFont, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "clamp(2rem, 1.4rem + 2.4vw, 2.625rem)"
    fontWeight: 800
    lineHeight: 1.2
    letterSpacing: "-0.03em"
  display-home:
    fontFamily: "Schibsted Grotesk Variable, sans-serif"
    fontSize: "clamp(2.25rem, 1.5rem + 3.4vw, 4.25rem)"
    fontWeight: 800
    lineHeight: 0.98
    letterSpacing: "-0.035em"
  display-section:
    fontFamily: "Schibsted Grotesk Variable, sans-serif"
    fontSize: "clamp(2rem, 1.3rem + 2.6vw, 3.5rem)"
    fontWeight: 800
    lineHeight: 1
    letterSpacing: "-0.035em"
  headline:
    fontFamily: "Schibsted Grotesk Variable, sans-serif"
    fontSize: "1.625rem"
    fontWeight: 650
    lineHeight: 1.2
    letterSpacing: "-0.01em"
  title:
    fontFamily: "Schibsted Grotesk Variable, sans-serif"
    fontSize: "1.3125rem"
    fontWeight: 650
    lineHeight: 1.2
    letterSpacing: "-0.01em"
  title-sm:
    fontFamily: "Schibsted Grotesk Variable, sans-serif"
    fontSize: "1.125rem"
    fontWeight: 650
    lineHeight: 1.2
    letterSpacing: "-0.01em"
  body:
    fontFamily: "Schibsted Grotesk Variable, sans-serif"
    fontSize: "1.0625rem"
    fontWeight: 400
    lineHeight: 1.65
  lead:
    fontFamily: "Schibsted Grotesk Variable, sans-serif"
    fontSize: "clamp(1.0625rem, 1rem + 0.2vw, 1.125rem)"
    fontWeight: 400
    lineHeight: 1.55
  body-sm:
    fontFamily: "Schibsted Grotesk Variable, sans-serif"
    fontSize: "0.9375rem"
    fontWeight: 400
    lineHeight: 1.55
  label:
    fontFamily: "Schibsted Grotesk Variable, sans-serif"
    fontSize: "0.875rem"
    fontWeight: 400
    lineHeight: 1.65
  command:
    fontFamily: "ui-monospace, SFMono-Regular, SF Mono, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "clamp(0.8125rem, 0.75rem + 0.4vw, 1rem)"
    lineHeight: 1.6
  code:
    fontFamily: "ui-monospace, SFMono-Regular, SF Mono, Menlo, Consolas, Liberation Mono, monospace"
    fontSize: "0.875rem"
    lineHeight: 1.6
rounded:
  base: "2px"
spacing:
  space-1: "0.25rem"
  space-2: "0.5rem"
  space-3: "0.75rem"
  space-4: "1rem"
  space-5: "1.5rem"
  space-6: "2rem"
  space-7: "3rem"
  space-8: "4rem"
components:
  link:
    textColor: "{colors.coral-red}"
  page-title:
    textColor: "{colors.naples-yellow}"
    typography: "{typography.display}"
  code-plate:
    backgroundColor: "{colors.plate}"
    textColor: "{colors.text}"
    typography: "{typography.code}"
    rounded: "{rounded.base}"
  inline-code:
    backgroundColor: "{colors.plate-raised}"
    textColor: "{colors.text}"
    rounded: "{rounded.base}"
    padding: "0.1em 0.35em"
  kbd:
    backgroundColor: "{colors.plate-raised}"
    textColor: "{colors.text}"
    rounded: "{rounded.base}"
    padding: "0 0.4em"
  aside:
    backgroundColor: "{colors.plate}"
    textColor: "{colors.text}"
    rounded: "{rounded.base}"
  aside-caution:
    backgroundColor: "{colors.eugenia-red}"
    textColor: "{colors.on-accent}"
    rounded: "{rounded.base}"
  search-field:
    backgroundColor: "{colors.ground}"
    textColor: "{colors.text-muted}"
    rounded: "{rounded.base}"
  table-cell:
    textColor: "{colors.text}"
    padding: "0.5rem 0.75rem"
  home-title:
    textColor: "{colors.naples-yellow}"
    typography: "{typography.display-home}"
  install-plate:
    backgroundColor: "{colors.plate}"
    textColor: "{colors.text}"
    typography: "{typography.command}"
    rounded: "{rounded.base}"
  plate-button:
    backgroundColor: "{colors.plate}"
    textColor: "{colors.coral-red}"
    rounded: "{rounded.base}"
    padding: "0 0.75rem"
  recording-plate:
    backgroundColor: "{colors.plate}"
    textColor: "{colors.text-muted}"
    rounded: "{rounded.base}"
  colophon:
    textColor: "{colors.text-muted}"
    typography: "{typography.label}"
  sky-day:
    backgroundColor: "{colors.pale-methyl-blue}"
    textColor: "{colors.ink}"
  sky-dusk:
    backgroundColor: "{colors.naples-yellow}"
    textColor: "{colors.ink}"
  sky-night:
    backgroundColor: "{colors.ground}"
    textColor: "{colors.text}"
  sky-close:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.text}"
  lit-plate:
    backgroundColor: "{colors.naples-yellow}"
    textColor: "{colors.ink}"
  service-tile:
    textColor: "{colors.ink}"
    padding: "0.75rem 1rem 1rem"
  tour-cap:
    backgroundColor: "{colors.plate-raised}"
    textColor: "{colors.naples-yellow}"
    rounded: "{rounded.base}"
  tour-cap-pressed:
    backgroundColor: "{colors.naples-yellow}"
    textColor: "{colors.ink}"
    rounded: "{rounded.base}"
  gate:
    backgroundColor: "{colors.eugenia-red}"
    textColor: "{colors.ink}"
---

# Design System: nuvem

## Overview

**Creative North Star: "One Sky"**

The site takes its colors from a plate in Wada Sanzo's *A Dictionary of Color Combinations*, combination 47, and its drawing from the terminal cell. On the home page the colors become one sky, from day to night, down the page; the docs are the calm night of that sky, a reference volume on the brown ground.

- **The sky.** The home page's ground changes three times over the whole page: Pale Methyl Blue day, Naples Yellow dusk, the brown night, and near-black at the close. Each change happens at an edge drawn in cells (cloud tops rising, or a dusk hanging down and thinning into half-tone), never at a straight line and never exactly on a section boundary.
- **The cell.** Edges, clouds, stars, the moon and the wordmark are all drawn from square cells on whole pixels, in flat palette colors. Clouds keep a one-pixel gap between cells and a half-tone underside.
- **Plates.** On the night, content that needs light sits on a contained plate: the service tiles, the lit Naples plate behind the diagram, the terminal windows.

The readers have a terminal open beside the browser. On the home page they decide whether to install in a minute; in the docs they arrive by search or a deep link, find one option, copy a command and leave.

There is one theme, dark at heart. There is no light mode and no theme switch.

**Key Characteristics:**
- Five named colors from one historical combination. On the home page they are the grounds of one sky; in the docs they are marks on the brown.
- One grotesk for all reading text, set heavy and tight for display; system monospace for code and terminal screens.
- Terminal cells as the one drawing system: sky edges, clouds, stars, moon, wordmark, window controls.
- Real screens of the tool, as text, wherever the product is shown.
- Flat. Depth comes from tone, and from one offset block behind the recording.

All tokens live in `src/styles/tokens.css` as CSS custom properties with the `--nv-` prefix. The docs map them onto Starlight's variables in `src/styles/docs.css`; `src/styles/identity.css` adds the strip, the wordmark and the frame details. The home page's sky is drawn by `src/lib/sky-cells.mjs`. Code plates are configured in `astro.config.mjs`, `src/styles/code-theme.json` and `src/lib/code-plates.mjs`.

## Colors

The palette is combination 47 from Wada Sanzo's *A Dictionary of Color Combinations*: Dull Purplish Black, Naples Yellow, Coral Red, Pale Methyl Blue and Eugenia Red. The five colors are used as given. The supporting tones are the ground mixed toward the off-white text or toward the darker plate, so no new hue enters the page.

Contrast ratios below are measured against the ground (`--nv-bg`) unless stated.

### Ground
- **Dull Purplish Black** (`--nv-bg`): the docs' ground for header, sidebar, column and controls, and the night of the home page's sky (services, how it works, the closing).

### The four colors, as marks on the ground
- **Naples Yellow** (`--nv-naples`, 8.86:1): page titles, the wordmark, the `$` prompt, the current position, the focus ring, search highlights, the legend on key caps.
- **Coral Red** (`--nv-coral`, 4.76:1): links and copy controls on the ground and on plates. Keywords and numbers inside code.
- **Pale Methyl Blue** (`--nv-methyl`, 4.28:1): rules, markers, the scroll cue. Large text or decoration only, never small text on the ground.
- **Eugenia Red** (`--nv-eugenia`, 3.22:1): never text on the ground. A fill only.

### The grounds of the home page's sky
Text takes the ink of the ground it can sit on. A text block that can straddle two grounds (the hero's pitch and install lines across blue and the rising yellow) uses an ink that passes on both.

| Ground | Where | Text | Links | Focus ring |
|---|---|---|---|---|
| Pale Methyl Blue | header, title, pitch | ink, 6.87:1 | ink, 700, 2px underline | 3px ink |
| Naples Yellow | dusk: behind the pitch and through the key tour; the lit plate | ink, 14.24:1 | ink, 700, 2px underline | 3px ink |
| Dull Purplish Black (`--nv-bg`) | night: services, how it works, the closing | off-white 9.36:1, muted 6.20:1, Naples titles 8.86:1 | Coral, 4.76:1 | 2px Naples, 8.86:1 |
| Near-black (`--nv-ink`) | the closing's lower edge and the footer | off-white 15.04:1, muted 9.96:1 | Coral, 7.64:1 | 2px Naples, 14.24:1 |

Tiles and labels may be filled with any of the four colors; text on them is ink (Naples 14.24:1, Coral 7.64:1, Methyl 6.87:1, Eugenia 5.17:1). `--nv-ink-soft` (`#423b36`) is allowed as secondary text on Naples (8.86:1) and Coral (4.76:1) only. A dark plate (code, a terminal, the install command) keeps its own Naples ring and Coral controls wherever it sits.

### Supporting tones
- **Plate** (`--nv-plate`): the ground of code and terminal plates, notes, the search input and the current sidebar row. Off-white text on it is 15.04:1.
- **Raised plate** (`--nv-plate-raised`): inline code, keys, window title bars. Off-white on it is 12.70:1, Naples 12.03:1.
- **Ink** (`--nv-ink`, the same value as the plate): text on blue and yellow grounds and on filled tiles, frames of the windows, lines of the diagram, the grid between service tiles, and the near-black ground of the close and the footer.
- **Text** (`--nv-text`, 9.36:1): body text and headings below the page title.
- **Muted text** (`--nv-text-muted`, 6.20:1): sidebar and contents links, captions, the search placeholder, footer metadata.
- **Rule** (`--nv-rule`): hairlines between regions, under table rows, around plates. Decorative.
- **Strong rule** (`--nv-rule-strong`, 3.47:1; 5.58:1 on the plate): outlines on controls such as the search field, the copy button, keys and the menu toggle.
- **On accent** (`--nv-on-accent`): text on a filled label or aside; the same value as ink.

On the plate, the code colors measure: off-white 15.04:1, Coral 7.64:1, Methyl 6.87:1, comments (`#a39a90`, italic) 6.38:1, line numbers (`#958b81`) 5.29:1. Strings are Naples Yellow and punctuation is muted text.

**The Ground Rule.** Text takes the ink of every ground it can touch: dark ink on blue and yellow, off-white on brown and near-black. Never set Naples, Coral, off-white or muted text on blue or yellow.

**The Methyl Floor Rule.** Pale Methyl Blue fails 4.5:1 on the ground. On the ground it is for lines, markers and text at 24px or larger only. As a ground it takes ink.

**The Eugenia Rule.** Eugenia Red is never text and never a line. It is a fill: caution and danger asides, the confirmation in the diagram, the Security tiles, the last band of the strip.

**The No New Hue Rule.** Never introduce another hue. A new tone is the ground mixed toward `--nv-text` or `--nv-plate`, written with `color-mix()` from existing tokens. Starlight's status colors are folded into the palette in `docs.css`.

## Typography

**Reading face:** Schibsted Grotesk Variable (`--nv-font-sans`), self-hosted from `@fontsource-variable` and declared in `src/styles/fonts.css` with `font-display: optional`: it is used when it is ready at first paint and never swapped in later, so text does not rewrap after the page has drawn. The home page preloads the upright Latin file, so on a normal connection it is ready. Until it is cached, text is set in Arial scaled to its metrics (`Schibsted Grotesk Fallback` in `tokens.css`), in weight bands: Arial for regular text, Arial Bold scaled to Schibsted at 650, 700 and 800 for emphasis, links and titles. The italic file loads only on pages whose text is italic.
**Code face:** the system monospace stack (`--nv-font-mono`), for code, commands, terminal screens, key legends and the pronunciation. No web font is loaded for code.

**Character:** one family across a strict scale. Display sizes are set at 800 with tight tracking (-0.035em) and a line height near 1, so a title reads as a block of type, the way the plate's colors read as blocks. Reading text stays at 400.

### Hierarchy
- **Page title** (800, `--nv-text-h1`, 32px to 42px fluid, line height 1.2, -0.03em): one per page, in Naples Yellow, balanced wrap.
- **Section heading** (650, `--nv-text-h2`, 26px): off-white, with a hairline above and 1rem of padding.
- **Subheading** (650, `--nv-text-h3`, 21px; `--nv-text-h4`, 18px): off-white, -0.01em.
- **Home title** (800, `--nv-text-hero`, 36px to 68px, line height 0.98, -0.035em): the home page's one title, in ink on the day sky, one line from 1280px.
- **Home section title** (800, `--nv-text-section`, 32px to 56px, line height 1, -0.035em): ink on the dusk, Naples on the night.
- **Pitch** (400, 17px to 20px fluid, line height 1.5): the home page's pitch, under its title.
- **Body** (400, `--nv-text-body`, 17px, line height `--nv-leading-body` 1.65): within `--nv-measure` (68ch). The docs column is 38rem, about 70 characters.
- **Small body** (400, `--nv-text-body-sm`, 15px): table cells, the home page's feature lines and diagram lanes.
- **Small** (400, `--nv-text-small`, 14px): sidebar, table of contents, captions, aside titles.
- **Code** (`--nv-text-code`, 14px, line height 1.6): set against the 17px body. The home page's install command is larger, `--nv-text-command` (13px to 16px), so it reads as the page's action. Inline code is 0.875em of its context.
- **Emphasis** (`strong`, 650): bold steps to 650, not 700, so it sits below the title.

Headings use `--nv-leading-tight` (1.2). The scale ratio is about 1.25.

**The One Family Rule.** All reading text is Schibsted Grotesk. Do not add a second display face. Monospace is for what the terminal prints or reads: code, commands, terminal screens, key legends, the pronunciation.

## Layout

Spacing runs on a 4px step: `--nv-space-1` (4px) through `--nv-space-8` (64px), with steps at 4, 8, 12, 16, 24, 32, 48 and 64px. Home page sections add fluid padding of 48px to 96px.

- **Docs frame:** header, a 17rem left sidebar, a content area up to `--nv-wide` (52rem) and a right-hand "On this page" list. All share the ground. Hairlines (`--nv-rule`) separate them; the strip runs under the header.
- **Measure:** prose stops at `--nv-measure` (38rem, about 70 characters). Tables and code plates use the whole content area (`--nv-wide`, 52rem), on purpose: reference tables need the room. The rules that divide the page, under the title and over each section, run to that same right edge, so the page has one structural edge and one reading measure.
- **Headings:** a section heading keeps 12px of air before the text it opens (8px for h3 and h4).
- **Contents list:** shown only when a page has at least one section heading; a page that is one block of text has none. A page without one keeps its column, and the header's search, at the same place as a page with one, so nothing shifts between pages.
- **Header:** the pixel wordmark left, search in the middle, a Docs link and the GitHub icon right, over the strip. Inside the docs, Docs is Naples at 650 with no underline, marking the section. Below 50rem on docs pages the two links move into the menu; the home page has no menu, so there they stay in the header.
- **Home page column:** 76rem. The sky's grounds and edges run edge to edge of the window while the content keeps the column; the panel that holds them clips what runs past the window, so the page never scrolls sideways.
- **Small screens:** the site holds at 360px with no sideways scroll.

**The Sky Rule for layout.** On the home page the ground changes only three times, at cell edges, and never exactly where a section starts or ends. In the docs, regions are divided by hairlines and never by fills.

## Elevation & Depth

The page is flat. Depth comes from tone: the plate is darker than the ground, and the raised plate sits between them.

There is one exception, and it is deliberate: the home page's recording sits on a flat block in the night's brown, offset down and to the right by 8px to 16px, with no blur, as a second printing plate slightly out of register. The brown reads on both the blue and the yellow the window crosses. Nothing else on the site has an offset block. The cell wordmark carries the same idea at the scale of a letter.

Starlight's floating menus keep two soft shadows: medium (`0 2px 8px rgb(0 0 0 / 0.25)`) and large (`0 8px 24px rgb(0 0 0 / 0.35)`).

## Shapes

Every corner is 2px (`--nv-radius`). Borders are 1px in the docs. On the home page, frames that hold content together (window frames, the diagram's lines and ends, the gap between service tiles) are 3px ink. Keys add a heavier bottom edge so they read as physical caps. There are no pills and no large radii.

## Motifs

### The strip
The four colors as equal flat bands, left to right: Naples, Coral, Methyl, Eugenia (`--nv-strip`). It belongs to the docs: 4px under the header and over the colophon, and a 2.5rem, 3px piece over each section heading. The home page has no strip; its sky carries the colors.

### The sky (home page)
Drawn at build time by `src/lib/sky-cells.mjs` and shown at whole pixels: edges on 12px cells (6px on screens up to 48rem), 160 cells wide, drawn twice side by side as inline SVG with crisp edges, so they stay sharp at any pixel ratio; clouds on 4px to 8px cells; stars on 4px cells; the moon on 6px cells. Every color is flat; nothing is blurred or graded. No change of ground runs straight for more than three cells: a longer flat stretch in an edge gets a step.
- **Day:** the header and the title on Pale Methyl Blue. From 64rem two off-white clouds with Coral half-tone undersides sit beside the title, the large one running off the page, inside the title's own box in a column the title is kept out of.
- **Dusk:** Naples cloud tops (`skyline`) rise into a 48px gap kept between the hero's text and the recording, at most 36px high, and the yellow turns flat a little below the top of the recording, so the window sits across the change. On a phone the edge is half the size and 12px of its tops clear the window's top edge, into a 24px gap. Coral clouds with Eugenia half-tone sit to the right of the tour's title: on a phone in the free part of the title's row, from 64rem in a column beside the title and intro that no text enters.
- **Night:** below the tour the yellow ends in a hanging edge with a row of half-tone cells and a sparse row under it (`fringeSvg`), into the brown. Stars (single cells and five-cell plus signs, off-white and Naples) and a cell crescent moon sit in rows kept free of text: under the hanging edge, between the service plate and the diagram (with a Pale Methyl Blue cloud with an off-white half-tone underside), and above the closing (an off-white wisp). From 90rem a few stars also sit in the page's side margins.
- **Close:** near-black cloud tops rise into padding kept under the closing's text, into the near-black footer.
- **Edges never cross text.** Every line of text sits wholly on one ground, at least 12px from any cell of a ground edge, at every width; each edge is only as tall as the clear space kept for it, and `npm run check:sky` samples the grounds around every line to enforce it.
- **Placement.** No decoration is placed against reflowing text. Each sits inside a reserved row, a column the text is kept out of, or the page margin, at least 16px from any text, tile or diagram element at every width; `npm run check:sky` measures this at nineteen widths from 360 to 2560. The tour's coral clouds sit in a size container and stay out when it is too narrow to keep them clear, as when a larger root font widens the reading measure. Phones show the same order with smaller edges and fewer decorations. Nothing in the sky moves.

### Terminal cells
A pattern of characters drawn as squares with a 12% gap (`CellCloud.astro`, `Wordmark.astro`). A full cell is `#`; a half-tone cell, like the `░` shade, is a checker of two quarter cells.
- **Wordmark:** "nuvem" on a five-cell x-height. On the home page it closes the page at up to 34rem wide, Naples over a Coral impression, and stands stacked letter over letter in the diagram's nuvem block. In the header it is `src/assets/wordmark.svg`, with an empty alt, because the site title beside it is the link's name: Naples cells only, drawn on whole pixels (4px cells, 1px gaps) and shown at exactly 129 by 24 CSS pixels, so every cell stays sharp at any width. On the home page's blue header it is darkened to ink. At that size a second impression would blur the word, so the header has none.
- **Cloud:** three shapes (large, small, wisp), a body color over a half-tone underside in a second color, drawn at a whole number of pixels a cell. Decoration only, hidden from assistive technology. On the home page clouds do not move; the 404 page's cloud drifts 32px sideways in its own row, only when the reader allows motion.
- **Window controls:** three cells, Coral, Naples, Methyl, in the title bar of every terminal window.

## Components

### Code plates
The signature component. Every code block is a framed plate.
- **Ground:** `--nv-plate`, 1px `--nv-rule` border, 2px corners, no shadow.
- **Caption:** a strip at the top, left-aligned, in muted 14px sans, divided from the code by a hairline. A block with a title shows its title. A block without one is captioned with its language: Shell, Terminal, TOML, JSON, YAML, Rust, Diff or Text.
- **Prompt:** shell blocks (bash, sh, shell, zsh) draw a Naples `$ ` before each command. The prompt is CSS, not text, so copying gives the command alone. A line continuing after a trailing backslash gets no prompt.
- **Terminal output:** `console` blocks are captioned Terminal and have no prompt. The window dots are hidden.
- **Copy control:** a Coral icon on the plate ground with a strong-rule outline. The success tooltip is Naples with dark text.
- **Scroll cue:** a line too long for the plate scrolls sideways, and a soft Methyl edge shows on each side that has more to see. It disappears at the end of the scroll.
- **Tabs:** the active tab is underlined in Methyl.

### Links
On brown and near-black: Coral, underlined at 1px with a 0.2em offset; on hover the underline thickens to 2px. On blue and yellow: ink, 700, underlined at 2px; 3px on hover.

### Position marker
The current page in the sidebar is drawn as nuvem draws its selected row: a plate-colored bar with a Naples `>` in front, the label in Naples at 650. The section in view in the contents list is Naples with an 8px hairline tick. Other entries are muted and turn off-white on hover.

### Tables
Reference tables (keys, flags, config fields, commands) are the hardest content on the site.
- **Rules, not stripes.** Rows are divided by `--nv-rule`. The header row has no fill: off-white text, 650 weight, start-aligned, with a 2px Methyl rule beneath.
- **Cells:** 15px text, 8px above and below, 16px between columns, aligned to the top. The table fills the content area.
- **First column:** a key, flag or field name. It never wraps.
- **Tokens never break:** inline code breaks only at its spaces, so `--configuration <NAME>` can wrap between the flag and its argument but never inside `--configuration`. A token longer than 24 characters may break after a `/`, `=` or `:`, so `--local-host-port=localhost:<local>` fits a phone.
- **Too wide:** the table scrolls inside its own frame, with the same Methyl scroll cue as code. The frame takes keyboard focus so it can be scrolled without a mouse.
- **Below 40rem:** each row becomes a block. The first cell is its bold heading; every other cell is labeled with its column name in muted 14px text.

### Keys (kbd)
In the docs: physical-looking caps: raised-plate ground, strong-rule outline with a 2px bottom edge, 2px corners, monospace at 0.8125em with a 1.4 line height and 0.1em by 0.4em of padding. A cap is an inline box that does not wrap, so a line never breaks between a cap and the punctuation around it, and a cap never makes its line or table row taller than plain text.

The Markdown writes keys as inline code, so it reads well on GitHub. The site draws a code span as caps when it names keys (`Enter`, `Ctrl-x`, `Alt-←`, a single character, or alternatives such as `Esc/q`) in two places: the key column of a table (headed Key, Keys or Press), and the prose of the Keys page and the service pages. In that prose a single character becomes a cap only when it is a letter or one of `/ : ? , [ ]`, so values such as `0` and `-` stay code. Commands, flags, config keys and values always stay code. Alternatives are divided by a muted `/`.

On the home page's yellow: raised-plate ground, 2px ink outline with a 4px bottom edge, Naples legend.

### Asides
A labeled plate. Every aside names its kind in a bold title, so color is never the only signal.
- **Note and tip:** plate ground, hairline border, off-white text. The title sits on a filled label: Methyl for a note, Naples for a tip, ink text and icon.
- **Caution and danger:** filled Eugenia Red. Title, icon, text, links and code all switch to `--nv-on-accent`. Inline code inside is outlined in dark, not tinted. The focus ring inside these asides is dark too (Naples would be 2.76:1 on Eugenia).

### Blockquotes
A 1px Methyl edge on the start side, muted text.

### Search
An outlined field on the ground, strong-rule border, muted label. On hover, the border and label lift to muted and off-white. On wide screens a `/` key hint shows at the end of the field. Pressing `/` opens search anywhere except inside a text field; Ctrl+K and Cmd+K also work. The results dialog sits on the ground with a strong-rule border. The search input is a plate with a Naples focus ring; matches are Naples-filled with dark text.

### Pager
Previous and next links in a hairline box. The title is Coral, the direction is muted. On hover the border turns Coral.

### Focus and selection
- **Focus:** the ring takes the color of the ground it is drawn on, so it always clears 3:1 against what surrounds it. A 2px solid Naples outline (`--nv-focus`) with a 2px offset on the brown, on near-black and on dark plates wherever they sit; a 3px ink outline on blue and yellow (the home page's header, hero, key tour and lit plate) and inside filled asides; on a service tile, an ink ring inset 7px, because the tile's neighbors sit 3px away.
- **Skip link:** hidden until focused, then a Naples tab with dark text at the top left.
- **Selection:** Methyl mixed 45% into the plate, with off-white text.
- **Caret:** Naples. **Scrollbar:** strong rule on the ground.
- **Reduced motion:** animations and transitions are cut to near zero when the reader asks for it.

### Colophon
Every page ends with two lines of muted 14px text: the license and source link, then the trademark notice. Its links are Coral. In the docs the strip runs over it; on the home page it sits on the near-black of the close, with no strip.

## Home page

The home page is a Starlight splash page, so the header, search, skip link and colophon are the docs' own; on the home page they sit on the sky (the header on blue, in ink, without the strip; the colophon on near-black). Every docs link on it goes through `docHref()`, so a renamed page fails the build.

1. **Hero, on the day sky.** The title in ink, left-aligned, one line from 1280px, with day clouds beside it from 64rem. Below it, from 60rem, two columns: the pitch on the left; the install plate, the other install methods and the first step on the right, all in ink. The dusk's cloud tops rise in the gap below them, never behind them. At 1280 by 720 at least 280px of the video shows; at 360 by 740 the command, Copy and the top of the still show.
2. **The recording, across the dusk.** The window (3px ink frame, raised title bar with the three control cells and the Pause/Play control, the brown offset block) sits where the yellow turns flat. The recording loads nothing before the page has loaded, starts muted and looping once on screen, pauses off screen, and starts by itself only above 48rem without `prefers-reduced-motion` and without the data saver. Up to 40rem the window shows a still until Play is pressed, and the poster is not downloaded until then: the Inventory box of the Home screen as text, cut after the "loaded" column. Without JavaScript the window shows a still at every width (the Inventory box on a phone, the whole Home screen as text above 40rem) and the caption links to the video file.
3. **Key tour, on the dusk.** A section title in ink, coral clouds beside it, then the tour (`KeyTour.astro`): real screens of `nuvem --demo` as text, one per tab, behind a row of key caps. It opens on Services, because the recording above it opens on Home.
   - Caps are 48px, raised-plate face with a Naples legend, 2px ink frame and a 6px ink edge below. The selected cap is pressed: it sinks 4px into its edge and takes a Coral face with an ink legend, and its label turns 800 and underlined. From 64rem the caps sit in one row with 48px between groups; from 36rem to 64rem in two even rows of four and three; on a phone they wrap.
   - ARIA tabs with automatic activation: arrows, Home and End move between caps. Each tab is named for a screen reader with its keys spoken and its label, such as "slash then Enter: Filter, open". While focus is inside the tour, the tool's own keys also switch screens: `h`, `g`, `/`, `:`, `?`, and `Space` then `:`. Outside the tour, `/` opens the site search as everywhere else.
   - Each screen sits in a terminal window with a short caption and a link into the docs. All panels share one grid cell, so switching never moves the page.
   - From 64rem the whole screen shows; below that, the box the screen is about, at 10px to 15px, in a window at most 22 rows tall.
   - Without JavaScript, the caps are hidden and every screen shows in order under its own heading.
   - Captions that start with a key (`g`, `Enter`, `?`) draw it as a small key cap.
   After the tour, four more features (Cloud Logging, Tunnels, Read-only mode, Plugins) are set as nuvem sets its own help overlay: a dark plate in a 3px ink frame with "Also in nuvem" set into its top rule in Naples mono, and one ruled row per key: the key in Naples in a column as wide as the longest key, so every name starts at the same place, the name as a Coral link to its docs page, and one muted line of at most 60 characters.
4. **Service plate, on the night.** A Naples section title and, beside it, one muted sentence whose count of services with a detail view is read from the docs, then the services in four named families (`ServicePlate.astro`), read from the services table in the docs: Compute and serverless (Methyl), Data and storage (Naples), Messaging and networking (Coral), Identity and security (Eugenia, with a row of half-tone cells so it does not depend on color alone). Each family is a column in a 3px ink frame, headed by its swatch and name. Each tile links to its page and shows the name, large, and the id the config file uses, small in mono.
5. **How it works, on the night.** A Naples title with three off-white sentences set under it as the section's lead (the services set theirs beside the title), then the diagram (`HowItWorks.astro`) on a lit Naples plate inset from the page's edges, with the nuvem block hanging over its left edge and Google Cloud over its right (on a phone, over its top and bottom); everything on the plate is ink. Read lanes run from nuvem over the top straight into Google Cloud; nuvem leads into the confirmation, and the change lanes leave it for Google Cloud. Every claim in it is taken from the docs' services overview. The links to Before a change runs, the credentials questions and the security policy follow on the brown, in Coral.
6. **Close, into near-black.** The cell wordmark (Naples over a Coral impression) with the dictionary line under it (`nuvem /ˈnu.vẽj/ noun, Portuguese: cloud.`), and beside it the Install title, the install plate again with a strong-rule edge so it reads on near-black, the link to every install method and the step after installing. Near-black cloud tops rise below them into the footer.

**Install plate:** a plate up to 36rem wide: the command, with the Naples `$` prompt drawn by CSS, and the Copy button beside it. When the command is wider than its space it wraps after a `/` or at a space, the prompt hangs, and the whole command stays visible. The button is at least 44px square, Coral on the plate with a strong-rule outline; after a copy it reads Copied in Naples for two seconds, and a hidden status line announces the result. The button is in place from the first paint, so the command never rewraps when the script runs; without JavaScript a `<noscript>` style hides it. Its focus ring sits inside the button, on the plate.

**Motion:** the recording, the cap press (120ms) and the clouds' drift. Nothing enters, fades or reveals on scroll.

## Do's and Don'ts

### Do:
- **Do** import the tokens on any new page and reach for them by name. Do not copy hex values into new CSS.
- **Do** set text in the ink of every ground it can touch: dark ink on blue and yellow, off-white on brown and near-black.
- **Do** draw new sky elements with `sky-cells.mjs`, on whole pixels, and place them in reserved rows, kept-out columns or the page margin, never against reflowing text. Run `npm run check:sky` after any change to the home page.
- **Do** show the tool as real text from `demo/frames/`, through `readTerminalFrame()`, which fails the build on a pair below 4.5:1.
- **Do** keep every corner at 2px.
- **Do** check new text against the ratios in Colors. Body text needs 4.5:1 on its own ground.
- **Do** link from the home page into the docs through `docHref()`.

### Don't:
- **Don't** introduce another hue, a decorative gradient, glass, glow or a light theme. (The scroll cue is the one soft gradient, and it carries information; the strip is four flat bands.)
- **Don't** set small text in Pale Methyl Blue or Eugenia Red on the brown, or anything but dark ink on blue or yellow.
- **Don't** divide the home page into stacked full-width bands of color, or change the sky at a straight line.
- **Don't** add a second offset block. The recording has the only one.
- **Don't** bring the sky into the docs. The docs stay on the brown and take the strip and the marks.
- **Don't** use card grids of icon, title and text, pill shapes or large radii.
- **Don't** add motion that ignores `prefers-reduced-motion`, scroll-linked effects or intro animations.
- **Don't** load a web font for code, or a font that blocks first paint.

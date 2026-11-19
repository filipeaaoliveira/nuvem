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
typography:
  display:
    fontFamily: "Schibsted Grotesk Variable, -apple-system, BlinkMacSystemFont, Segoe UI, Roboto, Helvetica Neue, Arial, sans-serif"
    fontSize: "clamp(2rem, 1.4rem + 2.4vw, 2.625rem)"
    fontWeight: 700
    lineHeight: 1.2
    letterSpacing: "-0.02em"
  display-home:
    fontFamily: "Schibsted Grotesk Variable, sans-serif"
    fontSize: "clamp(2.125rem, 1.5rem + 2.4vw, 3.25rem)"
    fontWeight: 700
    lineHeight: 1.08
    letterSpacing: "-0.025em"
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
---

# Design System: nuvem

## Overview

**Creative North Star: "The Reference Atlas"**

The site reads like a classified reference volume. Each page is one entry. Commands and terminal output are set as framed, captioned plates. Everything else is plain text on one dark ground, divided by hairlines.

The readers have a terminal open beside the browser. They arrive by search or a deep link, find one option, copy a command and leave. The design serves that: a comfortable measure, a visible `/` search, and a clear mark for where you are.

There is one theme, dark. There is no light mode and no theme switch.

**Key Characteristics:**
- One ground colour for header, sidebar and column. Regions are divided by 1px hairlines, never by fills.
- Five named colours from one historical combination, each with one job.
- One grotesk for all reading text, system monospace for code.
- Code and terminal output sit on darker plates with a caption.
- Position is marked by a short hairline tick, not by a filled highlight.

All tokens live in `src/styles/tokens.css` as CSS custom properties with the `--nv-` prefix. The docs map them onto Starlight's variables in `src/styles/docs.css`. Code plates are configured in `astro.config.mjs`, `src/styles/code-theme.json` and `src/lib/code-plates.mjs`.

## Colors

The palette is combination 47 from Wada Sanzo's *A Dictionary of Color Combinations*: Dull Purplish Black, Naples Yellow, Coral Red, Pale Methyl Blue and Eugenia Red. The five colours are used as given. The supporting tones are the ground mixed toward the off-white text or toward the darker plate, so no new hue enters the page.

Contrast ratios below are measured against the ground (`--nv-bg`) unless stated.

### Ground
- **Dull Purplish Black** (`--nv-bg`): the page ground for header, sidebar, column and controls.

### Accents
- **Naples Yellow** (`--nv-naples`, 8.86:1): page titles, the wordmark, the `$` prompt, the current-position tick and label, the focus ring, search highlights.
- **Coral Red** (`--nv-coral`, 4.76:1): links, the pager's link titles, copy buttons on plates. Keywords and numbers inside code.
- **Pale Methyl Blue** (`--nv-methyl`, 4.28:1): rules and markers. The header's bottom rule, the table header rule, the blockquote edge, the active tab underline on plates, aside icons for notes. Large text or decoration only, never small text on the ground.
- **Eugenia Red** (`--nv-eugenia`, 3.22:1): the fill of caution and danger asides, and nothing else. Text on it is always `--nv-on-accent` (5.17:1).

### Supporting tones
- **Plate** (`--nv-plate`): the ground of code and terminal plates, notes and the search input. Off-white text on it is 15.04:1.
- **Raised plate** (`--nv-plate-raised`): inline code and keys. Off-white on it is 12.70:1.
- **Text** (`--nv-text`, 9.36:1): body text and headings below the page title.
- **Muted text** (`--nv-text-muted`, 6.20:1): sidebar and contents links, captions, the search placeholder, footer metadata.
- **Rule** (`--nv-rule`): hairlines between regions, under table rows, around plates. Decorative.
- **Strong rule** (`--nv-rule-strong`, 3.47:1; 5.58:1 on the plate): outlines on controls such as the search field, the copy button, keys and the menu toggle, so every control boundary meets the 3:1 non-text minimum.
- **On accent** (`--nv-on-accent`): text on any Naples, Coral, Methyl or Eugenia fill. It is the plate colour, and it reads at 5.17:1 or better on all four.

On the plate, the code colours measure: off-white 15.04:1, Coral 7.64:1, Methyl 6.87:1, comments (`#a39a90`, italic) 6.38:1, line numbers (`#958b81`) 5.29:1. Strings are Naples Yellow and punctuation is muted text.

**The One Job Rule.** Each colour does one job. Links are Coral. Titles, prompt and position are Naples. Rules are Methyl. Warnings are Eugenia. No element mixes two accents.

**The Methyl Floor Rule.** Pale Methyl Blue fails 4.5:1 on the ground. Use it for lines, markers and text at 18px or larger only. On the plate it passes and may colour code tokens.

**The Fill-Only Rule.** Eugenia Red is never text and never a line. It appears only as a full fill with dark text on top.

**The No New Hue Rule.** Never introduce another hue. A new tone is the ground mixed toward `--nv-text` or `--nv-plate`, written with `color-mix()` from existing tokens. Starlight's blue, purple, green, orange and red status colours are already folded into the palette in `docs.css`.

## Typography

**Reading face:** Schibsted Grotesk Variable (`--nv-font-sans`), self-hosted through `@fontsource-variable` with `font-display: swap`. Upright and italic axes are both loaded. The fallback is the system UI stack, so text paints at once.
**Code face:** the system monospace stack (`--nv-font-mono`). No web font is loaded for code.

**Character:** one family across a strict scale. Weight and size carry rank. Running text has kerning and ligatures on.

### Hierarchy
- **Page title** (700, `--nv-text-h1`, 32px to 42px fluid, line height 1.2, -0.02em): one per page, in Naples Yellow, balanced wrap.
- **Section heading** (650, `--nv-text-h2`, 26px): off-white, with a hairline above and 1rem of padding.
- **Subheading** (650, `--nv-text-h3`, 21px; `--nv-text-h4`, 18px): off-white, -0.01em.
- **Home title** (700, `--nv-text-display`, 34px to 52px, line height 1.08, -0.025em): the home page's one title, in Naples Yellow.
- **Lead** (400, `--nv-text-lead`, 17px to 18px, line height 1.55): the home page's pitch, under its title.
- **Body** (400, `--nv-text-body`, 17px, line height `--nv-leading-body` 1.65): within `--nv-measure` (68ch). The docs column is 38rem, about 70 characters.
- **Small body** (400, `--nv-text-body-sm`, 15px): table cells, the home page's list detail and key caps.
- **Small** (400, `--nv-text-small`, 14px): sidebar, table of contents, captions, aside titles.
- **Code** (`--nv-text-code`, 14px, line height 1.6): set against the 17px body. The home page's install command is larger, `--nv-text-command` (13px to 16px), so it reads as the page's action. Inline code is 0.875em of its context.
- **Emphasis** (`strong`, 650): bold steps to 650, not 700, so it sits below the title.

Headings use `--nv-leading-tight` (1.2). The scale ratio is about 1.25.

**The One Family Rule.** All reading text is Schibsted Grotesk. Do not add a second display face. Code is the only place for monospace.

## Layout

Spacing runs on a 4px step: `--nv-space-1` (4px) through `--nv-space-8` (64px), with steps at 4, 8, 12, 16, 24, 32, 48 and 64px.

- **Docs frame:** header, a 17rem left sidebar, a content area up to `--nv-wide` (52rem) and a right-hand "On this page" list. All share the ground. Hairlines (`--nv-rule`) separate them.
- **Measure:** prose (paragraphs, lists, headings, asides) stops at `--nv-measure` (38rem, about 70 characters). Tables and code plates may use the whole content area.
- **Contents list:** shown only when a page has at least one section heading; a page that is one block of text has none. A page without one keeps its column, and the header's search, at the same place as a page with one, so nothing shifts between pages.
- **Header:** the `nuvem` wordmark left in Naples Yellow (1.375rem, 700), search in the middle, a Docs link and the GitHub icon right. A 1px Methyl rule runs under it on every page. Inside the docs, Docs is Naples at 650 with no underline, marking the section. Below 50rem on docs pages the two links move into the menu; the home page has no menu, so there they stay in the header.
- **One left edge:** headings, plates, tables and asides all hang from the column's left edge.
- **Sections:** an `h2` opens with 2.25em of space above, a hairline and 1rem of padding.
- **Plates:** 1.5rem (`--nv-space-5`) above and below.
- **Sidebar:** each directory in `docs/` is a group, collapsed unless it holds the current page. Nested directories nest. A directory with a single page shows as a plain link.
- **Small screens:** the site holds at 360px. The menu and contents toggles become outlined controls that match the search field. An open toggle takes a Naples outline.

**The Hairline Rule.** Regions are divided by 1px lines, never by background fills or shadows.

## Elevation & Depth

The page is flat. Depth comes from tone: the plate is darker than the ground, and the raised plate sits between them. Nothing on the page casts a shadow at rest.

Starlight's floating menus keep two soft shadows: medium (`0 2px 8px rgb(0 0 0 / 0.25)`) and large (`0 8px 24px rgb(0 0 0 / 0.35)`). Plates, pager links and controls have none. The search overlay backdrop is the plate at 80% opacity.

**The Tone Not Shadow Rule.** To lift something, put it on a darker plate. Do not add a shadow.

## Shapes

Every corner is 2px (`--nv-radius`): plates, inline code, keys, asides, search, pager links, toggles. Borders are 1px. Keys add a 2px bottom border so they read as physical caps. There are no pills and no large radii.

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
Coral, underlined at 1px with a 0.2em offset. On hover the underline thickens to 2px. Link colour also applies to code inside a link.

### Position marker
The current page in the sidebar and the section in view in the contents list are Naples Yellow, and a short horizontal hairline tick sits just left of the label (10px in the sidebar, 8px in the contents list). Current sidebar entries are also weight 650. There is no background highlight. Other entries are muted and turn off-white on hover.

### Tables
Reference tables (keys, flags, config fields, commands) are the hardest content on the site.
- **Rules, not stripes.** Rows are divided by `--nv-rule`. The header row has no fill: off-white text, 650 weight, start-aligned, with a Methyl rule beneath.
- **Cells:** 15px text, 8px above and below, 16px between columns, aligned to the top. The table fills the content area.
- **First column:** a key, flag or field name. It never wraps.
- **Tokens never break:** inline code breaks only at its spaces, so `--configuration <NAME>` can wrap between the flag and its argument but never inside `--configuration`. A token longer than 24 characters may break after a `/`.
- **Too wide:** the table scrolls inside its own frame, with the same Methyl scroll cue as code. The frame takes keyboard focus so it can be scrolled without a mouse.
- **Below 40rem:** each row becomes a block. The first cell is its bold heading; every other cell is labelled with its column name in muted 14px text.

### Keys (kbd)
Physical-looking caps: raised-plate ground, strong-rule outline with a 2px bottom edge, 2px corners, monospace at 0.8125em with a 1.4 line height and 0.1em by 0.4em of padding. A cap is an inline box that does not wrap, so a line never breaks between a cap and the punctuation around it, and a cap never makes its line or table row taller than plain text.

The Markdown writes keys as inline code, so it reads well on GitHub. The site draws a code span as caps when it names keys (`Enter`, `Ctrl-x`, `Alt-←`, a single character, or alternatives such as `Esc/q`) in two places: the key column of a table (headed Key, Keys or Press), and the prose of the Keys page and the service pages. In that prose a single character becomes a cap only when it is a letter or one of `/ : ? , [ ]`, so values such as `0` and `-` stay code. Commands, flags, config keys and values always stay code. Alternatives are divided by a muted `/`.

### Asides
A labelled plate. Every aside names its kind in a 14px bold title, so colour is never the only signal.
- **Note and tip:** plate ground, hairline border, off-white text. The note icon is Methyl, the tip icon is Naples.
- **Caution and danger:** filled Eugenia Red with an Eugenia border. Title, icon, text, links and code all switch to `--nv-on-accent`. Inline code inside is outlined in dark, not tinted, so its text stays at 5.17:1. The focus ring inside these asides is dark too (Naples would be 2.76:1 on Eugenia).

### Blockquotes
A 1px Methyl edge on the start side, muted text.

### Search
An outlined field on the ground, strong-rule border, muted label. On hover, the border and label lift to muted and off-white. On wide screens a `/` key hint shows at the end of the field. Pressing `/` opens search anywhere except inside a text field; Ctrl+K and Cmd+K also work. The results dialog sits on the ground with a strong-rule border. The search input is a plate with a Naples focus ring; matches are Naples-filled with dark text.

### Pager
Previous and next links in a hairline box. The title is Coral, the direction is muted. On hover the border turns Coral.

### Focus and selection
- **Focus:** a 2px solid Naples outline (`--nv-focus`) with a 2px offset, on every focusable element (dark inside filled asides).
- **Skip link:** hidden until focused, then a Naples tab with dark text at the top left.
- **Selection:** Methyl mixed 45% into the plate, with off-white text.
- **Caret:** Naples. **Scrollbar:** strong rule on the ground.
- **Reduced motion:** animations and transitions are cut to near zero when the reader asks for it.

### Colophon
Every page ends with a hairline and two lines of muted 14px text: the licence and source link, then the trademark notice. Its links are Coral.

## Home page

The home page is a Starlight splash page, so the header, search, skip link and colophon are the docs' own. It opens with a centred hero: the title, the pitch and the install block, then the other ways to install and the first step, each on one line where the screen allows. The recording follows at the full content width, and everything after it hangs from the recording's left edge. The hero is kept compact so that at least 300px of the recording shows in the first viewport of a 1280 by 720 screen.

- **Title:** the one Naples title, larger than a docs title (`--nv-text-display`), centred and balanced, one line from 1024px up. The pitch below it is off-white in `--nv-text-lead`, centred within 58rem, two lines on a laptop screen.
- **Install plate:** a plate up to 40rem wide: the command, with the Naples `$` prompt drawn by CSS, and the Copy button beside it. When the command is wider than its space, as on a phone, it wraps after a `/` or at a space (`<wbr>` marks the slashes, so copying is unaffected), the prompt hangs, and the whole command stays visible; nothing scrolls. If even the wrapped command leaves no room for the button, the button drops under it at full width. The button is at least 44px square, Coral text with a strong-rule outline; after a copy its label reads Copied in Naples for two seconds, and a hidden status line announces the result. Without JavaScript the button is not shown and the command is plain selectable text.
- **Plate buttons** (Copy, Pause/Play): 14px, 650 weight, at least 32px tall (Copy 44px), Coral on the plate (7.64:1), strong-rule outline (5.58:1). Hover turns the outline Coral.
- **Recording plate:** the recording of `nuvem --demo` at the full content width, under a caption strip that names the command and holds the Pause/Play control, with a muted caption below saying what it shows, that the data is sample data, and linking to the video file so a phone can open it full size. Width and height are set, so nothing moves when it loads. It loads nothing before the page has loaded, starts muted and looping once it is on screen, and pauses off screen. It starts by itself only above 48rem, without `prefers-reduced-motion` and without the browser's data saver; otherwise the poster shows, the control reads Play, and the video is not downloaded until it is pressed. On a phone its terminal text would be too small to read, so the readable view of the tool there is the terminal frame below. This is the page's only motion.
- **Terminal frame:** a screen of nuvem as text, on a plate with a caption strip. It scales to fill the plate and stops at 11px. Below 48rem the plate shows only the box the frame is about (the palette), cut from the frame at build time and set at 10px or more, so the subject fits a 360px screen. Text that still does not fit scrolls sideways inside the plate with the Methyl scroll cue, and only then is the plate a tab stop. Bright black and the selected-row colour are lightened for the site (see `src/lib/terminal-frame.mjs`), so all its text clears 4.5:1.
- **Key list:** a short list of what someone deciding to install should know, about eight rows; the docs carry the rest, and a line after the list links to them and to the keys reference. Each row's title line starts with the key or flag that reaches it, as an off-white key cap (15px, 650) or code, on the same baseline as the Coral link to its docs page; one muted line of detail sits below. A key sequence is written with a muted "then" between the caps. A visually hidden "Key:", "Keys:" or "Flag:" precedes the caps, so a screen reader does not read a bare letter before each link. Rows are divided by hairlines and sit two to a row from 64rem up.
- **Sections** open with the docs' section heading: a hairline, 1rem of padding, 26px at 650. 64px separates them. From 64rem up, How it works and the closing Install sit side by side in two equal columns, so the closing install plate hangs from the right column's left edge.

## Do's and Don'ts

### Do:
- **Do** import the tokens on any new page. A custom Astro page such as a landing page imports `src/styles/tokens.css` and sets `background-color: var(--nv-bg)`, `color: var(--nv-text)` and `font-family: var(--nv-font-sans)` on `body`. The docs already load it through Starlight's `customCss`.
- **Do** reach for tokens by name: `var(--nv-coral)` for links, `var(--nv-naples)` for a title, `var(--nv-space-5)` for a gap. Do not copy hex values into new CSS.
- **Do** use Naples Yellow for the one title on a page and for where the reader is.
- **Do** use Coral Red for every link, underlined.
- **Do** keep Pale Methyl Blue to rules, markers and large text.
- **Do** set commands and output as captioned plates on `--nv-plate`.
- **Do** mark position with the hairline tick and a Naples label.
- **Do** keep every corner at 2px and every divider at 1px.
- **Do** check new text against the ratios in Colors. Body text needs 4.5:1 on its own ground.
- **Do** link from the home page into the docs through `docHref()`, so a broken link fails the build.
- **Do** keep the recording the home page's only motion.

### Don't:
- **Don't** introduce another hue, a decorative gradient, or a light theme. (The scroll cue is the one gradient, and it carries information.)
- **Don't** set small text in Pale Methyl Blue or Eugenia Red on the ground.
- **Don't** use Eugenia Red for anything but a filled caution or danger plate with dark text.
- **Don't** mix two accents in one element.
- **Don't** separate regions with fills or shadows. Use a hairline.
- **Don't** show position or state by colour alone. Pair it with a tick, a weight change or a text label.
- **Don't** load a web font for code, or a font that blocks first paint.
- **Don't** use card grids, pill shapes or large radii.

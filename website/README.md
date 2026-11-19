# nuvem website

The project site: a home page and the user documentation, built with
[Astro](https://astro.build) and [Starlight](https://starlight.astro.build).

## Commands

Run these from this directory. You need Node.js 22.23.4 or newer.
`.node-version` pins the exact release that the hosted build uses.

```sh
npm ci           # install the exact dependencies in package-lock.json
npm run dev      # serve the site at http://localhost:4321 and reload on change
npm run build    # write the static site to dist/
```

`npm run preview` serves the built `dist/` locally. Search works only in a
build, not in `npm run dev`.

Astro caches rendered pages and re-renders a page only when its Markdown
changes. If you edit a Markdown or code-block plugin in `src/lib/`, delete
the cache with `rm -rf .astro node_modules/.astro` before you build.

## Where the docs come from

The docs pages are the Markdown files in the repository's `docs/` directory.
The site reads them in place, and there are no copies under `website/`. When
you edit a file in `docs/`, the page changes.

- Title: the page's frontmatter `title`, or else its first `# ` heading.
  The heading is not repeated in the page body.
- Address: `docs/reference/keys.md` is served at `/docs/reference/keys/`, and
  `docs/getting-started/first-run.md` at `/docs/getting-started/first-run/`. A
  frontmatter `slug` overrides this.
- Sidebar: each top-level file is a link and each directory is a group. A
  directory inside a directory is a group inside a group. Groups are
  collapsed, except the one holding the page you are on. A directory with
  only one page shows as a plain link, not a group of one.
- Group labels: the `sidebar.label` or `title` of the directory's
  `index.md`, or else the directory name in sentence case
  (`getting-started` becomes "Getting started").
- Order: inside a group, pages follow their `sidebar.order` frontmatter,
  then their title, and `index.md` comes first. Top-level entries follow
  `INTENT_ORDER` in `src/lib/docs-source.mjs`. Anything not listed there goes
  last, alphabetically.
- Links: write links between docs as relative file links, such as
  `[keys](reference/keys.md#global-keys)`, so they also work on GitHub. The
  site rewrites them to page addresses. A relative link to any other file in
  the repository (`../ARCHITECTURE.md`, `decisions/`) points to that file on
  GitHub.
- Not published: `docs/decisions/` and `docs/performance-read-paths.md`
  are contributor documentation. The list is `EXCLUDED` in
  `src/lib/docs-source.mjs`.
- Changelog: the repository's `CHANGELOG.md` is read in place too, served
  at `/docs/changelog/` and listed last in the sidebar.

To add a page, add a Markdown file to `docs/`. To add a sidebar group, add a
directory with at least two pages.

## Link check

`npm run build` fails when a relative link in a published docs page is
broken. It lists every broken link with the file it is in. A link is broken
in these cases:

- It points to a docs page that does not exist.
- It has an anchor (`#section`) that the target page has no heading or `id`
  for, including anchors on the same page.
- It points to a repository file or directory that does not exist.

`npm run dev` prints the same list as a warning and keeps serving. The check
is `src/lib/doc-links.mjs`. It reads the Markdown directly, so it does not
depend on the page cache described above.

## The home page

`src/pages/index.astro` is a Starlight page with the splash template, so it
shares the docs' header, search and footer. It shows the real tool, read
from the repository's `demo/` directory in place:

- `demo/out/nuvem-demo.webm`, `nuvem-demo.mp4` and `nuvem-demo-poster.png`
  are the recording. The build copies them into `dist/_astro/` with a
  content hash and converts the poster to WebP.
- `demo/frames/palette.html` is a screen of the app as colored text. The
  build inlines it into the page. For screens narrower than 48rem, it also
  inlines a crop of the command palette box alone, found by the box's title.
  If a re-captured frame has no such box, the build says so.

To re-render either, run the scripts in `scripts/` (see `demo/README.md`) and
rebuild. Nothing under `website/` changes. If one of these files is missing,
the build stops and names the command to run (`src/lib/demo-assets.mjs`).

A frame is accepted only as the single `<pre>` of colored `<span>` runs that
`scripts/ansi2html.py` writes. Two terminal colors are shown lighter than in
the recordings so that every run of text reads at 4.5:1 or better, and the
build fails on any pair below that (`src/lib/terminal-frame.mjs`).

Links from the home page into the docs go through `docHref()` in
`src/lib/doc-links.mjs`, so a renamed page or heading fails the build, as a
broken link inside the docs does.

## Headers and the social card

`public/_headers` sets the response headers on Cloudflare Pages: long-lived
caching for the hashed files in `/_astro/`, and a Content-Security-Policy
that allows only the site's own files. The build replaces the placeholder in
its `script-src` with a hash of every inline script it emitted
(`src/lib/security-headers.mjs`), so a new inline script needs only a
rebuild. The site loads nothing from other origins. Keep it that way.

`public/og.png` is the 1200 by 630 card that link previews show.

## Layout

| Path | What it holds |
| --- | --- |
| `src/content.config.ts` | The loader that reads `docs/` |
| `src/lib/docs-source.mjs` | File-to-address mapping, titles and the sidebar |
| `src/lib/markdown-docs.mjs` | Removes the leading title, rewrites links, keeps code tokens whole, labels table cells, draws keys as key caps |
| `src/lib/doc-links.mjs` | Resolves docs links and fails the build on broken ones |
| `src/lib/code-plates.mjs` | Code block captions and the shell prompt |
| `src/lib/security-headers.mjs` | Fills the CSP script hashes in `dist/_headers` |
| `src/lib/demo-assets.mjs` | The demo files the home page uses; fails the build when one is missing |
| `src/lib/terminal-frame.mjs` | Reads a frame from `demo/frames/`, checks its markup and contrast |
| `src/routeData.ts` | Hides a one-entry contents list; no canonical on the 404 page |
| `src/styles/tokens.css` | Colors, type, spacing: shared by every page |
| `src/styles/docs.css` | The docs theme, mapped onto Starlight |
| `src/styles/code-theme.json` | Syntax colors for code blocks |
| `src/components/` | Starlight overrides: one theme, `/` opens search, a Docs link in the header and the mobile menu, the site footer |
| `src/components/landing/` | The home page's install plate, recording and terminal frame |
| `src/pages/index.astro` | The home page |
| `src/pages/404.astro` | The page for an address the site does not have |
| `DESIGN.md` | The design system: palette, type, components |

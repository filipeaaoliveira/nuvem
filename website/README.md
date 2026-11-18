# nuvem website

The project site: a home page and the user documentation, built with
[Astro](https://astro.build) and [Starlight](https://starlight.astro.build).

## Commands

Run these from this directory. You need Node.js 22.23.4 or newer;
`.node-version` pins the exact release the hosted build uses.

```sh
npm ci           # install the exact dependencies in package-lock.json
npm run dev      # serve the site at http://localhost:4321 and reload on change
npm run build    # write the static site to dist/
```

`npm run preview` serves the built `dist/` locally. Search only works in a
build, not in `npm run dev`.

Astro caches rendered pages and only re-renders a page when its Markdown
changes. After you edit a Markdown or code-block plugin in `src/lib/`, delete
the cache with `rm -rf .astro node_modules/.astro` before you build.

## Where the docs come from

The docs pages are the Markdown files in the repository's `docs/` directory.
The site reads them in place; there are no copies under `website/`. Edit a
file in `docs/` and the page changes.

- **Title:** the page's frontmatter `title`, or else its first `# ` heading.
  The heading is not repeated in the page body.
- **Address:** `docs/keys.md` is served at `/docs/keys/`, and
  `docs/guides/first-run.md` at `/docs/guides/first-run/`. A frontmatter
  `slug` overrides this.
- **Sidebar:** each top-level file is a link and each directory is a group;
  a directory inside a directory is a group inside a group. Groups are
  collapsed, except the one holding the page you are on. A directory with
  only one page shows as a plain link, not a group of one.
- **Group labels:** the `sidebar.label` or `title` of the directory's
  `index.md`, or else the directory name in sentence case
  (`getting-started` becomes "Getting started").
- **Order:** inside a group, pages follow their `sidebar.order` frontmatter,
  then their title; `index.md` comes first. Top-level entries follow
  `INTENT_ORDER` in `src/lib/docs-source.mjs`; anything not listed there goes
  last, alphabetically.
- **Links:** write links between docs as relative file links, such as
  `[keys](reference/keys.md#global-keys)`, so they also work on GitHub. The
  site rewrites them to page addresses. A relative link to any other file in
  the repository (`../ARCHITECTURE.md`, `decisions/`) points to that file on
  GitHub.
- **Not published:** `docs/decisions/` and `docs/performance-read-paths.md`
  are contributor documentation. The list is `EXCLUDED` in
  `src/lib/docs-source.mjs`.

To add a page, add a Markdown file to `docs/`. To add a sidebar group, add a
directory with at least two pages.

## Link check

`npm run build` fails when a relative link in a published docs page is
broken, and lists every broken link with the file it is in:

- a link to a docs page that does not exist;
- an anchor (`#section`) that the target page has no heading or `id` for,
  including anchors on the same page;
- a link to a repository file or directory that does not exist.

`npm run dev` prints the same list as a warning and keeps serving. The check
is `src/lib/doc-links.mjs`. It reads the Markdown directly, so it does not
depend on the page cache described above.

## Headers and the social card

`public/_headers` sets the response headers on Cloudflare Pages: long-lived
caching for the hashed files in `/_astro/`, and a Content-Security-Policy
that allows only the site's own files. The build replaces the placeholder in
its `script-src` with a hash of every inline script it emitted
(`src/lib/security-headers.mjs`), so a new inline script needs nothing more
than a rebuild. The site loads nothing from other origins; keep it that way.

`public/og.png` is the 1200 by 630 card that link previews show.

## Layout

| Path | What it holds |
| --- | --- |
| `src/content.config.ts` | The loader that reads `docs/` |
| `src/lib/docs-source.mjs` | File-to-address mapping, titles and the sidebar |
| `src/lib/markdown-docs.mjs` | Removes the leading title, rewrites links, keeps code tokens whole, labels table cells |
| `src/lib/doc-links.mjs` | Resolves docs links and fails the build on broken ones |
| `src/lib/code-plates.mjs` | Code block captions and the shell prompt |
| `src/lib/security-headers.mjs` | Fills the CSP script hashes in `dist/_headers` |
| `src/routeData.ts` | Hides a one-entry contents list; no canonical on the 404 page |
| `src/styles/tokens.css` | Colours, type, spacing: shared by every page |
| `src/styles/docs.css` | The docs theme, mapped onto Starlight |
| `src/styles/code-theme.json` | Syntax colours for code blocks |
| `src/components/` | Starlight overrides: one theme, `/` opens search |
| `src/pages/index.astro` | The home page |
| `DESIGN.md` | The design system: palette, type, components |

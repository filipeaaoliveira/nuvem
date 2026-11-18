# nuvem website

The project site: a home page and the user documentation, built with
[Astro](https://astro.build) and [Starlight](https://starlight.astro.build).

## Commands

Run these from this directory. You need Node.js 22.23.4 or newer.

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
- **Sidebar:** each top-level file is a link and each top-level directory is a
  group. A group is labelled by its `index.md` title, or else by the
  directory name. Pages inside a group are sorted by their `sidebar.order`
  frontmatter, then by title. Top-level entries follow the reading order in
  `INTENT_ORDER` in `src/lib/docs-source.mjs`; anything not listed there goes
  last, alphabetically.
- **Links:** write links between docs as relative file links, such as
  `[keys](keys.md#navigation)`, so they also work on GitHub. The site rewrites
  them to page addresses. A relative link to any other file in the repository
  points to that file on GitHub.
- **Not published:** `docs/decisions/` and `docs/performance-read-paths.md`
  are contributor documentation. The list is `EXCLUDED` in
  `src/lib/docs-source.mjs`.

To add a page, add a Markdown file to `docs/`. To add a sidebar group, add a
directory.

## Layout

| Path | What it holds |
| --- | --- |
| `src/content.config.ts` | The loader that reads `docs/` |
| `src/lib/docs-source.mjs` | File-to-address mapping, titles and the sidebar |
| `src/lib/markdown-docs.mjs` | Removes the leading title and rewrites links |
| `src/styles/tokens.css` | Colours, type, spacing: shared by every page |
| `src/styles/docs.css` | The docs theme, mapped onto Starlight |
| `src/styles/code-theme.json` | Syntax colours for code blocks |
| `src/components/` | Starlight overrides: one theme, `/` opens search |
| `src/pages/index.astro` | The home page |
| `DESIGN.md` | The design system: palette, type, components |

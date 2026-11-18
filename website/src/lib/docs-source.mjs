// Where the site's docs come from, and how a file in `docs/` maps to a URL.
//
// The site renders the repository's `docs/` directory in place. Nothing here
// names a docs page: pages are discovered from the directory, titled from
// frontmatter or their first `# ` heading, and grouped by subdirectory.
// The loader, the sidebar and the link rewriter all use this module so a
// file has exactly one URL.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, posix, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

/** Absolute path of the repository root. */
export const REPO_ROOT = resolve(here, '../../..');

/** Absolute path of the docs directory the site renders. */
export const DOCS_DIR = join(REPO_ROOT, 'docs');

/** Absolute path of the site (`website/`), and the docs directory relative to it. */
export const SITE_ROOT = resolve(here, '../..');
const DOCS_FROM_SITE = relative(SITE_ROOT, DOCS_DIR).split(sep).join('/');

/** Where links to repository files that are not published should point. */
export const REPO_URL = 'https://github.com/filipeaaoliveira/nuvem';
export const REPO_BLOB_URL = `${REPO_URL}/blob/main`;

/** URL prefix of every docs page; the landing page owns `/`. */
export const DOCS_ROUTE = 'docs';

/**
 * Contributor documentation: kept in the repository, linked to, not
 * published. Paths are relative to `docs/` and match a file or a directory.
 */
export const EXCLUDED = ['decisions', 'performance-read-paths.md'];

/** Glob patterns for the content loader. */
export const DOCS_GLOB = ['**/*.{md,mdx}', ...EXCLUDED.map((p) => (p.includes('.') ? `!${p}` : `!${p}/**`))];

const MARKDOWN = /\.mdx?$/i;

/** True when `relPath` (relative to `docs/`, POSIX separators) is published. */
export function isPublished(relPath) {
	if (!MARKDOWN.test(relPath)) return false;
	return !EXCLUDED.some((p) => relPath === p || relPath.startsWith(`${p}/`));
}

/**
 * Collection id for a file, from its path relative to `docs/`.
 * `guides/first-run.md` -> `docs/guides/first-run`; `guides/index.md` -> `docs/guides`.
 * A frontmatter `slug` wins, as it does in Starlight.
 */
export function idFor(relPath, slug) {
	if (slug) return posix.join(DOCS_ROUTE, String(slug).replace(/^\/+|\/+$/g, ''));
	const parts = relPath
		.replace(MARKDOWN, '')
		.split('/')
		.map((s) => s.trim().toLowerCase().replace(/\s+/g, '-'));
	if (parts.at(-1) === 'index') parts.pop();
	return [DOCS_ROUTE, ...parts].join('/');
}

/** Site URL for a file, with a trailing slash. */
export function urlFor(relPath, slug) {
	return `/${idFor(relPath, slug)}/`;
}

/** Splits a Markdown source into its frontmatter block (or '') and body. */
export function splitFrontmatter(source) {
	const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(source);
	return match ? { frontmatter: match[1], body: source.slice(match[0].length) } : { frontmatter: '', body: source };
}

/** Text of the first level-one ATX heading outside code fences, or undefined. */
export function firstHeading(markdownBody) {
	let fence = null;
	for (const line of markdownBody.split(/\r?\n/)) {
		const f = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
		if (f) {
			if (!fence) fence = f[1][0];
			else if (f[1][0] === fence) fence = null;
			continue;
		}
		if (fence) continue;
		const h = /^\s{0,3}#\s+(.+?)\s*#*\s*$/.exec(line);
		if (h) return stripInlineMarkdown(h[1]);
	}
	return undefined;
}

function stripInlineMarkdown(text) {
	return text
		.replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
		.replace(/[`*_]/g, '')
		.trim();
}

/** A top-level scalar frontmatter field of a file, or undefined. */
export function frontmatterField(absPath, key) {
	const { frontmatter } = splitFrontmatter(readFileSync(absPath, 'utf8'));
	const m = new RegExp(`^${key}:[ \\t]*(.+)$`, 'm').exec(frontmatter);
	return m ? m[1].trim().replace(/^(['"])(.*)\1$/, '$2') : undefined;
}

/** Title for a file: its frontmatter `title` when present, else its first heading, else its name. */
export function titleFor(absPath) {
	const title = frontmatterField(absPath, 'title');
	if (title) return title;
	const { body } = splitFrontmatter(readFileSync(absPath, 'utf8'));
	return firstHeading(body) ?? humanize(absPath.split(sep).at(-1).replace(MARKDOWN, ''));
}

/** `getting-started` -> `Getting started`. */
export function humanize(name) {
	const words = name.replace(/^\d+[-_.]\s*/, '').replace(/[-_]+/g, ' ').trim();
	return words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * The order a reader meets the docs in, by intent. A top-level file or
 * directory whose name is listed here takes that position; anything else
 * follows, alphabetically. Names that do not exist are ignored.
 */
export const INTENT_ORDER = [
	'getting-started',
	'installation',
	'install',
	'guides',
	'features',
	'demo-and-read-only',
	'configuration',
	'plugins',
	'reference',
	'keys',
	'cli',
	'services',
	'troubleshooting',
	'faq',
	'changelog',
];

function rank(name) {
	const i = INTENT_ORDER.indexOf(name.replace(MARKDOWN, '').toLowerCase());
	return i === -1 ? INTENT_ORDER.length : i;
}

/**
 * Starlight sidebar built from the docs directory: top-level files become
 * links, top-level directories become groups whose pages Starlight lists
 * itself (ordered by each page's `sidebar.order` frontmatter, then title).
 */
export function buildSidebar(dir = DOCS_DIR) {
	const entries = readdirSync(dir)
		.filter((name) => !name.startsWith('.'))
		.map((name) => ({ name, abs: join(dir, name), rel: relative(DOCS_DIR, join(dir, name)).split(sep).join('/') }))
		.filter(({ abs, rel }) => (statSync(abs).isDirectory() ? !EXCLUDED.includes(rel) : isPublished(rel)))
		.sort((a, b) => rank(a.name) - rank(b.name) || a.name.localeCompare(b.name));

	return entries.flatMap(({ name, abs, rel }) => {
		if (statSync(abs).isDirectory()) {
			const index = ['index.md', 'index.mdx'].map((f) => join(abs, f)).find((f) => exists(f));
			return [
				{
					label: index ? titleFor(index) : humanize(name),
					// Starlight matches autogenerated groups on each entry's file path,
					// which for these files is relative to the site root: `../docs/...`.
					items: [{ autogenerate: { directory: posix.join(DOCS_FROM_SITE, rel) } }],
				},
			];
		}
		if (/^index\.mdx?$/i.test(name)) return [];
		return [{ slug: idFor(rel, frontmatterField(abs, 'slug')) }];
	});
}

function exists(path) {
	try {
		return statSync(path).isFile();
	} catch {
		return false;
	}
}

/**
 * URL of the first page in sidebar order, so `/docs/` has somewhere to go
 * until the docs carry an index page of their own.
 */
export function firstPageUrl(sidebar = buildSidebar()) {
	for (const item of sidebar) {
		if ('slug' in item) return `/${item.slug}/`;
		const auto = item.items?.[0]?.autogenerate;
		if (auto) {
			const dir = resolve(SITE_ROOT, auto.directory);
			const first = readdirSync(dir)
				.filter((name) => MARKDOWN.test(name))
				.map((name) => ({ name, order: sidebarOrder(join(dir, name)) }))
				.sort((a, b) => a.order - b.order || a.name.localeCompare(b.name))[0]?.name;
			if (first) return urlFor(relative(DOCS_DIR, join(dir, first)).split(sep).join('/'));
		}
	}
	return '/';
}

/** A page's `sidebar.order` frontmatter, or Infinity. */
function sidebarOrder(absPath) {
	const { frontmatter } = splitFrontmatter(readFileSync(absPath, 'utf8'));
	const m = /^sidebar:\s*\n(?:[ \t]+.*\n)*?[ \t]+order:[ \t]*(-?\d+)/m.exec(`${frontmatter}\n`);
	return m ? Number(m[1]) : Infinity;
}

/** True when the docs carry their own top-level index page. */
export function hasDocsIndex() {
	return ['index.md', 'index.mdx'].some((f) => exists(join(DOCS_DIR, f)));
}

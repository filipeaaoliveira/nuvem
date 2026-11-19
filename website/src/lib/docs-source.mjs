// Where the site's docs come from, and how a file in `docs/` maps to a URL.
//
// The site renders the repository's `docs/` directory in place. Nothing here
// names a docs page: pages are discovered from the directory, titled from
// frontmatter or their first `# ` heading, and grouped by subdirectory.
// The loader, the sidebar, the link rewriter and the link check all use this
// module, so a file has exactly one URL.

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, posix, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse as parseYaml } from 'yaml';

const here = dirname(fileURLToPath(import.meta.url));

/**
 * Absolute path of the repository root. Pages that import this module are
 * bundled into `dist/` before they render, which moves `import.meta.url`, so
 * astro.config.mjs (which loads it in place) records the root in
 * NUVEM_REPO_ROOT for them.
 */
export const REPO_ROOT = process.env.NUVEM_REPO_ROOT || resolve(here, '../../..');

/** Absolute path of the docs directory the site renders. */
export const DOCS_DIR = join(REPO_ROOT, 'docs');

/** Where links to repository files that are not published should point. */
export const REPO_URL = 'https://github.com/filipeaaoliveira/nuvem';
export const REPO_BLOB_URL = `${REPO_URL}/blob/main`;
export const REPO_TREE_URL = `${REPO_URL}/tree/main`;

/** URL prefix of every docs page; the landing page owns `/`. */
export const DOCS_ROUTE = 'docs';

/**
 * Contributor documentation: kept in the repository, linked to, not
 * published. Paths are relative to `docs/` and match a file or a directory.
 */
export const EXCLUDED = ['decisions', 'performance-read-paths.md'];

/**
 * The repository's changelog. The site renders it in place as the last
 * sidebar entry, so there is no copy under `docs/` that can drift from it.
 */
export const CHANGELOG_FILE = join(REPO_ROOT, 'CHANGELOG.md');
export const CHANGELOG_ID = `${DOCS_ROUTE}/changelog`;

/** Glob patterns for the content loader. */
export const DOCS_GLOB = ['**/*.{md,mdx}', ...EXCLUDED.map((p) => (p.includes('.') ? `!${p}` : `!${p}/**`))];

const MARKDOWN = /\.mdx?$/i;
const INDEX = /^index\.mdx?$/i;

/** POSIX path of `abs` relative to `docs/`. */
export function docsRelative(abs) {
	return relative(DOCS_DIR, abs).split(sep).join('/');
}

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

/** Site URL for a published file (absolute path), with a trailing slash. */
export function urlForFile(absPath) {
	return `/${idFor(docsRelative(absPath), frontmatter(absPath).slug)}/`;
}

/** Splits a Markdown source into its frontmatter block (or '') and body. */
export function splitFrontmatter(source) {
	const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(source);
	return match ? { frontmatter: match[1], body: source.slice(match[0].length) } : { frontmatter: '', body: source };
}

/** Parsed frontmatter of a file, or `{}`. Invalid YAML fails with the file named. */
export function frontmatter(absPath) {
	const { frontmatter: raw } = splitFrontmatter(readFileSync(absPath, 'utf8'));
	if (!raw.trim()) return {};
	try {
		return parseYaml(raw) ?? {};
	} catch (error) {
		throw new Error(`Invalid frontmatter in ${relative(REPO_ROOT, absPath)}: ${error.message}`);
	}
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

/** Title for a file: its frontmatter `title` when present, else its first heading, else its name. */
export function titleFor(absPath) {
	const { title } = frontmatter(absPath);
	if (title) return String(title);
	const { body } = splitFrontmatter(readFileSync(absPath, 'utf8'));
	return firstHeading(body) ?? humanize(absPath.split(sep).at(-1).replace(MARKDOWN, ''));
}

/** Words that keep their own capitals in a label made from a file name. */
const LABEL_WORDS = { cli: 'CLI', faq: 'FAQ', gce: 'GCE', gke: 'GKE', iam: 'IAM', vpc: 'VPC', sql: 'SQL', api: 'API' };

/** `getting-started` -> `Getting started`; `cli` -> `CLI`. */
export function humanize(name) {
	const words = name
		.replace(/^\d+[-_.]\s*/, '')
		.split(/[-_\s]+/)
		.filter(Boolean)
		.map((w) => LABEL_WORDS[w.toLowerCase()] ?? w.toLowerCase());
	const label = words.join(' ');
	return label.charAt(0).toUpperCase() + label.slice(1);
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

function intentRank(name) {
	const i = INTENT_ORDER.indexOf(name.replace(MARKDOWN, '').toLowerCase());
	return i === -1 ? INTENT_ORDER.length : i;
}

/** A page's `sidebar.order` frontmatter, or Infinity. */
function sidebarOrder(absPath) {
	const order = frontmatter(absPath).sidebar?.order;
	return Number.isFinite(order) ? order : Infinity;
}

function isDirectory(path) {
	return existsSync(path) && statSync(path).isDirectory();
}

/** Published pages directly in `dir`, and its subdirectories that hold any. */
function listDir(dir) {
	const files = [];
	const dirs = [];
	for (const name of readdirSync(dir)) {
		if (name.startsWith('.')) continue;
		const abs = join(dir, name);
		const rel = docsRelative(abs);
		if (isDirectory(abs)) {
			if (!EXCLUDED.includes(rel) && countPages(abs) > 0) dirs.push({ name, abs });
		} else if (isPublished(rel)) {
			files.push({ name, abs });
		}
	}
	return { files, dirs };
}

function countPages(dir) {
	const { files, dirs } = listDir(dir);
	return files.length + dirs.reduce((n, d) => n + countPages(d.abs), 0);
}

function onlyPage(dir) {
	const { files, dirs } = listDir(dir);
	return files[0]?.abs ?? onlyPage(dirs[0].abs);
}

function indexOf(dir) {
	return ['index.md', 'index.mdx'].map((f) => join(dir, f)).find((f) => existsSync(f));
}

/** Label for a directory's group: its index page's sidebar label or title, else its name. */
function groupLabel(dir) {
	const index = indexOf(dir);
	if (index) {
		const fm = frontmatter(index);
		return String(fm.sidebar?.label ?? titleFor(index));
	}
	return humanize(dir.split(sep).at(-1));
}

function link(abs) {
	return { slug: idFor(docsRelative(abs), frontmatter(abs).slug) };
}

function items(dir, topLevel) {
	const { files, dirs } = listDir(dir);
	const entries = [
		...files.map(({ name, abs }) => ({
			name,
			abs,
			isDir: false,
			order: INDEX.test(name) ? -Infinity : sidebarOrder(abs),
			title: titleFor(abs),
		})),
		...dirs.map(({ name, abs }) => {
			const index = indexOf(abs);
			return { name, abs, isDir: true, order: index ? sidebarOrder(index) : Infinity, title: groupLabel(abs) };
		}),
	];
	if (topLevel && existsSync(CHANGELOG_FILE)) {
		entries.push({ name: 'changelog', abs: CHANGELOG_FILE, isDir: false, order: Infinity, title: 'Changelog' });
	}
	entries.sort((a, b) =>
		topLevel
			? intentRank(a.name) - intentRank(b.name) || a.name.localeCompare(b.name)
			: a.order - b.order || Number(a.isDir) - Number(b.isDir) || a.title.localeCompare(b.title),
	);
	return entries.flatMap(({ name, abs, isDir }) => {
		if (abs === CHANGELOG_FILE) return [{ slug: CHANGELOG_ID }];
		if (!isDir) return topLevel && INDEX.test(name) ? [] : [link(abs)];
		// A directory with a single page is a link, not a group of one.
		if (countPages(abs) === 1) return [link(onlyPage(abs))];
		// Starlight opens the group that holds the current page.
		return [{ label: groupLabel(abs), collapsed: true, items: items(abs, false) }];
	});
}

/**
 * Starlight sidebar built from the docs directory. Top-level files are links
 * and top-level directories are collapsed groups, in reading order by intent.
 * Inside a group, pages follow their `sidebar.order` frontmatter, then title;
 * an `index.md` comes first and names the group. Nested directories become
 * nested groups.
 */
export function buildSidebar(dir = DOCS_DIR) {
	return items(dir, true);
}

/**
 * URL of the first page in sidebar order, so `/docs/` has somewhere to go
 * until the docs carry an index page of their own.
 */
export function firstPageUrl(sidebar = buildSidebar()) {
	const first = (list) => {
		for (const item of list) {
			if ('slug' in item) return item.slug;
			const nested = first(item.items ?? []);
			if (nested) return nested;
		}
		return undefined;
	};
	const slug = first(sidebar);
	return slug ? `/${slug}/` : '/';
}

/** True when the docs carry their own top-level index page. */
export function hasDocsIndex() {
	return ['index.md', 'index.mdx'].some((f) => existsSync(join(DOCS_DIR, f)));
}

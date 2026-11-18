// Links in `docs/`: where each one goes on the site, and a check that every
// relative link points at something that exists.
//
// Docs link to each other as relative files (`../reference/keys.md#global`)
// so the Markdown also works on GitHub. On the site:
// - a link to a published page becomes that page's URL; its anchor must be a
//   heading or `id` on the target page;
// - a link to anything else in the repository (contributor docs, source
//   files) becomes a GitHub URL; the file or directory must exist.
// The build fails on any link that breaks these rules, naming the file.

import { existsSync, readFileSync, statSync } from 'node:fs';
import { dirname, relative, resolve, sep } from 'node:path';
import { glob } from 'node:fs/promises';
import { markdownToHtml } from 'satteri';
import { satteriHeadingIdsPlugin } from '@astrojs/markdown-satteri';
import {
	DOCS_DIR,
	REPO_BLOB_URL,
	REPO_ROOT,
	REPO_TREE_URL,
	docsRelative,
	isPublished,
	splitFrontmatter,
	urlForFile,
} from './docs-source.mjs';

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;

/** True when `path` is `dir` or inside it. */
export function inside(path, dir) {
	const rel = relative(dir, path);
	return rel === '' || (!rel.startsWith('..') && !rel.startsWith(sep) && !/^[a-z]:/i.test(rel));
}

/**
 * Where a link written in `fromFile` goes. Returns `{ href }` for links the
 * site can serve, plus what to check: `page` and `anchor` for a docs page,
 * `repoPath` for a repository file, or `anchor` alone for the same page.
 * External, absolute and empty links come back unchanged.
 */
export function resolveLink(url, fromFile) {
	if (!url || url.startsWith('/') || SCHEME.test(url)) return { href: url };
	const hashAt = url.indexOf('#');
	const target = hashAt === -1 ? url : url.slice(0, hashAt);
	const anchor = hashAt === -1 ? '' : decodeURIComponent(url.slice(hashAt + 1));
	if (!target) return { href: url, page: fromFile, anchor };

	const abs = resolve(dirname(fromFile), decodeURI(target));
	const hash = anchor ? `#${encodeURIComponent(anchor)}` : '';
	if (inside(abs, DOCS_DIR) && isPublished(docsRelative(abs))) {
		const href = existsSync(abs) ? urlForFile(abs) + hash : url;
		return { href, page: abs, anchor };
	}
	if (inside(abs, REPO_ROOT)) {
		const rel = relative(REPO_ROOT, abs).split(sep).join('/');
		const isDir = existsSync(abs) && statSync(abs).isDirectory();
		return { href: `${isDir ? REPO_TREE_URL : REPO_BLOB_URL}/${rel}${hash}`, repoPath: abs };
	}
	return { href: url, outside: abs };
}

/** Heading ids and explicit `id` attributes of a docs page, as the site renders them. */
const anchorCache = new Map();
async function anchorsOf(file) {
	if (!anchorCache.has(file)) {
		const { body } = splitFrontmatter(readFileSync(file, 'utf8'));
		const { html } = await markdownToHtml(body, { hastPlugins: [satteriHeadingIdsPlugin()] });
		anchorCache.set(file, new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((m) => decodeEntities(m[1]))));
	}
	return anchorCache.get(file);
}

function decodeEntities(text) {
	return text.replace(/&amp;/g, '&').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&lt;/g, '<').replace(/&gt;/g, '>');
}

/**
 * Site URL for a link to a docs page written relative to `docs/`, such as
 * `getting-started/install.md#build-from-source`, for pages outside the docs
 * (the home page) that link into them. Throws when the page or its anchor
 * does not exist, so a broken link fails the build.
 */
export async function docHref(url) {
	const link = resolveLink(url, resolve(DOCS_DIR, 'index.md'));
	if (!link.page || !existsSync(link.page)) throw new Error(`docs link "${url}" does not point at a published docs page`);
	if (link.anchor && !(await anchorsOf(link.page)).has(link.anchor)) {
		throw new Error(`docs link "${url}": docs/${docsRelative(link.page)} has no #${link.anchor}`);
	}
	return link.href;
}

/** Link and definition URLs in a Markdown body. */
async function linksIn(file) {
	const urls = [];
	const collect = (node) => {
		urls.push(node.url);
	};
	const { body } = splitFrontmatter(readFileSync(file, 'utf8'));
	await markdownToHtml(body, { mdastPlugins: [{ name: 'collect-links', link: collect, definition: collect }] });
	return urls;
}

/**
 * Checks every relative link in the published docs. Returns one message per
 * broken link, each naming the source file and the target.
 */
export async function checkDocLinks() {
	anchorCache.clear();
	const problems = [];
	const files = [];
	for await (const rel of glob('**/*.{md,mdx}', { cwd: DOCS_DIR })) {
		const posixRel = rel.split(sep).join('/');
		if (isPublished(posixRel)) files.push(resolve(DOCS_DIR, rel));
	}
	for (const file of files.sort()) {
		const source = relative(REPO_ROOT, file).split(sep).join('/');
		for (const url of await linksIn(file)) {
			const link = resolveLink(url, file);
			const shown = (abs) => relative(REPO_ROOT, abs).split(sep).join('/');
			if (link.outside) {
				problems.push(`${source}: "${url}" points outside the repository`);
			} else if (link.repoPath && !existsSync(link.repoPath)) {
				problems.push(`${source}: "${url}" points at ${shown(link.repoPath)}, which does not exist`);
			} else if (link.page) {
				if (!existsSync(link.page)) {
					problems.push(`${source}: "${url}" points at ${shown(link.page)}, which does not exist`);
				} else if (link.anchor && !(await anchorsOf(link.page)).has(link.anchor)) {
					problems.push(`${source}: "${url}" points at #${link.anchor}, which ${shown(link.page)} does not have`);
				}
			}
		}
	}
	return problems;
}

/** Astro integration: fails the build, or warns in dev, when a docs link is broken. */
export function docLinksIntegration() {
	const run = async (logger, fail) => {
		const problems = await checkDocLinks();
		if (problems.length === 0) return;
		const message = `Broken links in docs/ (${problems.length}):\n  ${problems.join('\n  ')}`;
		if (fail) throw new Error(message);
		logger.warn(message);
	};
	return {
		name: 'nuvem-doc-links',
		hooks: {
			'astro:build:start': ({ logger }) => run(logger, true),
			'astro:server:start': ({ logger }) => run(logger, false),
		},
	};
}

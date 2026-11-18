// Markdown (Sätteri mdast) plugin for files rendered from the repository's `docs/`.
//
// The same Markdown has to read well on GitHub and on the site, so:
// - The leading `# Title` is dropped: Starlight prints the page title itself.
// - Relative links to other published docs become site URLs.
// - Relative links to anything else in the repository (contributor docs,
//   source files) point at the file on GitHub.

import { existsSync, readFileSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, relative, resolve, sep } from 'node:path';
import { DOCS_DIR, REPO_BLOB_URL, REPO_ROOT, REPO_URL, firstHeading, frontmatterField, isPublished, splitFrontmatter, urlFor } from './docs-source.mjs';

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;

/** Plugin factory: Sätteri calls it once per file and skips files it returns nothing for. */
export default function docsMarkdown({ fileURL }) {
	if (!fileURL) return undefined;
	const path = fileURLToPath(fileURL);
	if (!inside(path, DOCS_DIR)) return undefined;
	const fromDir = dirname(path);
	let titleSeen = false;

	const relink = (node, ctx) => {
		const url = rewrite(node.url, fromDir);
		if (url !== node.url) ctx.setProperty(node, 'url', url);
	};

	return {
		name: 'nuvem-repo-docs',
		heading(node, ctx) {
			if (titleSeen || node.depth !== 1) return;
			titleSeen = true;
			const parent = ctx.parent(node);
			if (parent?.type === 'root' && ctx.indexOf(node) <= 1) ctx.removeNode(node);
		},
		link: relink,
		definition: relink,
	};
}

function rewrite(url, fromDir) {
	if (!url || url.startsWith('#') || url.startsWith('/') || SCHEME.test(url)) return url;
	const [target, ...rest] = url.split('#');
	const hash = rest.length ? `#${rest.join('#')}` : '';
	const abs = resolve(fromDir, decodeURI(target));

	if (inside(abs, DOCS_DIR)) {
		const rel = relative(DOCS_DIR, abs).split(sep).join('/');
		if (isPublished(rel) && existsSync(abs)) {
			// The page's own `# Title` is not rendered, so an anchor to it means the page top.
			const title = firstHeading(splitFrontmatter(readFileSync(abs, 'utf8')).body);
			const toTitle = title && hash === `#${headingSlug(title)}`;
			return urlFor(rel, frontmatterField(abs, 'slug')) + (toTitle ? '' : hash);
		}
	}
	if (inside(abs, REPO_ROOT) || abs === REPO_ROOT) {
		const rel = relative(REPO_ROOT, abs).split(sep).join('/');
		const isDir = existsSync(abs) && statSync(abs).isDirectory();
		return `${isDir ? `${REPO_URL}/tree/main` : REPO_BLOB_URL}/${rel}${hash}`;
	}
	return url;
}

/** GitHub-style heading anchor. */
function headingSlug(text) {
	return text
		.toLowerCase()
		.replace(/[^\p{L}\p{N}\s_-]/gu, '')
		.replace(/\s/g, '-');
}

function inside(path, dir) {
	const rel = relative(dir, path);
	return rel !== '' && !rel.startsWith('..') && !rel.startsWith(sep) && !/^[a-z]:/i.test(rel);
}


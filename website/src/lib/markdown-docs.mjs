// Markdown (Sätteri mdast) plugin for files rendered from the repository's
// `docs/`, and for its `CHANGELOG.md`.
//
// The same Markdown has to read well on GitHub and on the site, so:
// - The leading `# Title` is dropped: Starlight prints the page title itself.
// - Relative links become site URLs or GitHub URLs (see doc-links.mjs, which
//   also fails the build on a broken one).
// - Inline code breaks only at spaces, never inside a token such as
//   `--configuration`; a long path may still break after a `/`.
// - Table cells carry their column's header as `data-label`, so a narrow
//   screen can show each row as a labelled block, and each table's scroll
//   region is named after the heading above it.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolveLink, inside } from './doc-links.mjs';
import { CHANGELOG_FILE, DOCS_DIR, firstHeading, splitFrontmatter, titleFor } from './docs-source.mjs';

/** Plugin factory: Sätteri calls it once per file and skips files it returns nothing for. */
export default function docsMarkdown({ fileURL }) {
	if (!fileURL) return undefined;
	const path = fileURLToPath(fileURL);
	if (!inside(path, DOCS_DIR) && path !== CHANGELOG_FILE) return undefined;
	let titleSeen = false;
	// Each table's scroll region is named after the heading above it.
	let section = titleFor(path);
	const tablesIn = new Map();

	const relink = (node, ctx) => {
		const { href, page, anchor } = resolveLink(node.url, path);
		let url = href;
		// The target's own `# Title` is not rendered, so an anchor to it means the page top.
		if (page && page !== path && anchor && anchor === titleAnchor(page)) url = href.replace(/#.*$/, '');
		if (url !== node.url) ctx.setProperty(node, 'url', url);
	};

	return {
		name: 'nuvem-repo-docs',
		heading(node, ctx) {
			if (node.depth > 1 || titleSeen) section = ctx.textContent(node).trim();
			if (titleSeen || node.depth !== 1) return;
			titleSeen = true;
			const parent = ctx.parent(node);
			if (parent?.type === 'root' && ctx.indexOf(node) <= 1) ctx.removeNode(node);
		},
		link: relink,
		definition: relink,
		inlineCode(node, ctx) {
			// Headings keep plain code so their ids match the link check's.
			if (ctx.parent(node)?.type === 'heading') return;
			ctx.replaceNode(node, { type: 'html', value: codeHtml(node.value) });
		},
		table(node, ctx) {
			const [head, ...rows] = node.children ?? [];
			if (!head) return;
			// A scroll container for wide tables, focusable so keyboards can scroll
			// it, and named after its section so a screen reader's list of regions
			// tells the tables apart. A second table in one section gets a number.
			const seen = (tablesIn.get(section) ?? 0) + 1;
			tablesIn.set(section, seen);
			const label = seen === 1 ? section : `${section}, table ${seen}`;
			ctx.wrapNode(node, {
				type: 'paragraph',
				data: { hName: 'div', hProperties: { className: ['nv-table'], tabIndex: 0, role: 'region', ariaLabel: label } },
				children: [],
			});
			const labels = head.children.map((cell) => ctx.textContent(cell).trim());
			for (const row of rows) {
				row.children.forEach((cell, i) => {
					if (labels[i]) ctx.setProperty(cell, 'data', { hProperties: { dataLabel: labels[i] } });
				});
			}
		},
	};
}

const titleAnchors = new Map();
function titleAnchor(file) {
	if (!titleAnchors.has(file)) {
		const title = firstHeading(splitFrontmatter(readFileSync(file, 'utf8')).body);
		titleAnchors.set(file, title ? headingSlug(title) : undefined);
	}
	return titleAnchors.get(file);
}

/** GitHub-style heading anchor. */
function headingSlug(text) {
	return text
		.toLowerCase()
		.replace(/[^\p{L}\p{N}\s_-]/gu, '')
		.replace(/\s/g, '-');
}

function escapeHtml(text) {
	return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/**
 * `<code>` whose words never break inside: each run of non-space characters
 * is an unbreakable token, and a long path offers a break after each `/`.
 */
function codeHtml(value) {
	const parts = value.split(/(\s+)/).map((part, i) => {
		if (i % 2 === 1) return escapeHtml(part);
		if (!part) return '';
		const token = part.length > 24 ? escapeHtml(part).replace(/\//g, '/<wbr>') : escapeHtml(part);
		return `<span class="nv-token">${token}</span>`;
	});
	return `<code>${parts.join('')}</code>`;
}

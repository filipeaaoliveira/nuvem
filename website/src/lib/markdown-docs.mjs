// Markdown (Sätteri mdast) plugin for files rendered from the repository's
// `docs/`, and for its `CHANGELOG.md`.
//
// The same Markdown has to read well on GitHub and on the site, so:
// - The leading `# Title` is dropped: Starlight prints the page title itself.
// - Relative links become site URLs or GitHub URLs (see doc-links.mjs, which
//   also fails the build on a broken one).
// - Inline code breaks only at spaces, never inside a token such as
//   `--configuration`; a long token may still break after a `/`, `=` or `:`.
// - Table cells carry their column's header as `data-label`, so a narrow
//   screen can show each row as a labeled block, and each table's scroll
//   region is named after the heading above it.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
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
	// Cells in a table's key column, whose key-like code is drawn as key caps.
	const keyCells = new Set();
	const keyPage = KEY_PAGES.some((page) => inside(path, join(DOCS_DIR, page)));

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
			let cell = ctx.parent(node);
			while (cell && cell.type !== 'tableCell') cell = ctx.parent(cell);
			const inKeyColumn = Boolean(cell && keyCells.has(cell));
			const keys = keyCaps(node.value, inKeyColumn || keyPage, !inKeyColumn);
			ctx.replaceNode(node, { type: 'html', value: keys ?? codeHtml(node.value) });
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
			const keyColumn = labels.findIndex((label) => KEY_COLUMNS.includes(label));
			for (const row of rows) {
				row.children.forEach((cell, i) => {
					if (labels[i]) ctx.setProperty(cell, 'data', { hProperties: { dataLabel: labels[i] } });
					if (i === keyColumn) keyCells.add(cell);
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

/** Table columns that list keys, by their header. */
const KEY_COLUMNS = ['Key', 'Keys', 'Press'];

/**
 * Pages whose prose names keys in code, such as "press `r`": the key
 * reference and the service pages. Paths are relative to `docs/`.
 */
const KEY_PAGES = ['reference/keys.md', 'reference/services'];

/** Keys with a name, as the docs write them. `Home` is a workspace, not a key. */
const NAMED_KEYS = ['Esc', 'Enter', 'Tab', 'Space', 'Backspace', 'Delete', 'PgUp', 'PgDn', 'Shift', 'Ctrl', 'Alt', '↑', '↓', '←', '→'];

/**
 * One key as the docs write it: a named key, a single character, or either
 * after `Ctrl-`, `Alt-` or `Shift-`. In prose (`loose` false) a single
 * character counts only when it is a letter or a mark nuvem binds, so a
 * value such as `0` or `-` stays code.
 */
function isKey(token, loose) {
	const combo = /^(?:Ctrl|Alt|Shift)-(.+)$/u.exec(token);
	if (combo) return NAMED_KEYS.includes(combo[1]) || [...combo[1]].length === 1;
	if (NAMED_KEYS.includes(token)) return true;
	if ([...token].length !== 1) return false;
	return loose ? !/\s/u.test(token) : /^[A-Za-z/:?,[\]]$/u.test(token);
}

/**
 * Key caps for a code span that names keys, or undefined to leave it code.
 * Only spans in a table's key column or anywhere on a key page are `read`;
 * `strict` is false only in a key column. A span may hold alternatives,
 * `Enter/Esc/q`, or, in a key column, a range of digits, `0-9`.
 */
function keyCaps(value, read, strict) {
	if (!read) return undefined;
	const kbd = (key) => `<kbd>${escapeHtml(key)}</kbd>`;
	if (isKey(value, !strict)) return kbd(value);
	const range = /^(\d)-(\d)$/.exec(value);
	const or = (mark) => `<span class="nv-key-or">${mark}</span>`;
	if (range && !strict) return `${kbd(range[1])}${or('–')}${kbd(range[2])}`;
	const options = value.split('/');
	if (options.length > 1 && options.every((key) => key && isKey(key, !strict))) return options.map(kbd).join(or('/'));
	return undefined;
}

function escapeHtml(text) {
	return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/**
 * `<code>` whose words never break inside: each run of non-space characters
 * is an unbreakable token, and a long one offers a break after each `/`,
 * `=` and `:`, as in `--local-host-port=localhost:<local>`.
 */
function codeHtml(value) {
	const parts = value.split(/(\s+)/).map((part, i) => {
		if (i % 2 === 1) return escapeHtml(part);
		if (!part) return '';
		const token = part.length > 24 ? escapeHtml(part).replace(/[/=:]/g, '$&<wbr>') : escapeHtml(part);
		return `<span class="nv-token">${token}</span>`;
	});
	return `<code>${parts.join('')}</code>`;
}

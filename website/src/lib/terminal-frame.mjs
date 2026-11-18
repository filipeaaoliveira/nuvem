// Reads a frame from `demo/frames/` (one screen of nuvem as a coloured
// `<pre>`, written by `scripts/ansi2html.py`) so a page can inline it as text.
//
// - Only the markup ansi2html writes is accepted: one `<pre>` holding text
//   and flat `<span style="...">` runs. Anything else fails the build, so a
//   frame can never carry a script or a link into the page.
// - Two terminal colours are shown lighter than in the recordings. nuvem's
//   muted text and borders use the terminal's "bright black", and its
//   selected row uses the "blue" slot, which the recordings set to the page
//   ground. Muted text on that row would read at 2.89:1. On the site, bright
//   black becomes a lighter grey of the same family and the selected row a
//   raised plate, so every run of text reads at 4.5:1 or better.
// - Every text and background pair in the result is checked against 4.5:1.

import { readFileSync } from 'node:fs';
import { relative } from 'node:path';
import { REPO_ROOT } from './docs-source.mjs';

/** Terminal colour (as the recordings draw it) -> colour on the site. */
export const SITE_COLOURS = {
	'#8c8178': '#afa69e', // bright black: borders and muted text
	'#423b36': '#2c2622', // blue: the selected row (--nv-plate-raised)
};

const ALLOWED_PROPERTIES = new Set(['color', 'background', 'font-weight', 'font-style', 'text-decoration']);
const MIN_CONTRAST = 4.5;

function luminance(hex) {
	const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
	const lin = (v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
	return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG contrast ratio of two `#rrggbb` colours. */
export function contrast(a, b) {
	const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
	return (hi + 0.05) / (lo + 0.05);
}

function parseStyle(style, where) {
	const out = {};
	for (const rule of style.split(';').filter(Boolean)) {
		const [prop, value] = rule.split(':').map((s) => s.trim());
		if (!ALLOWED_PROPERTIES.has(prop)) throw new Error(`${where}: unexpected style "${rule}"`);
		if ((prop === 'color' || prop === 'background') && !/^#[0-9a-f]{6}$/i.test(value)) {
			throw new Error(`${where}: colour "${value}" is not #rrggbb`);
		}
		out[prop] = prop === 'color' || prop === 'background' ? (SITE_COLOURS[value.toLowerCase()] ?? value.toLowerCase()) : value;
	}
	return out;
}

const styleText = (props) =>
	Object.entries(props)
		.map(([k, v]) => `${k}:${v}`)
		.join(';');

/**
 * Reads the frame at `absPath`. Returns the inner HTML (with the site's
 * colours), the frame's own text and background colours, and its size in
 * columns and rows.
 */
export function readTerminalFrame(absPath) {
	const where = relative(REPO_ROOT, absPath);
	const source = readFileSync(absPath, 'utf8').trim();
	const pre = /^<pre class="terminal-frame" style="([^"]*)">([\s\S]*)<\/pre>$/.exec(source);
	if (!pre) throw new Error(`${where}: expected a single <pre class="terminal-frame"> element`);
	const frame = parseStyle(pre[1], where);
	if (!frame.color || !frame.background) throw new Error(`${where}: the <pre> must set color and background`);

	const problems = [];
	let open = false;
	const html = pre[2].replace(/<[^>]*>/g, (tag) => {
		if (tag === '</span>') {
			if (!open) throw new Error(`${where}: </span> without <span>`);
			open = false;
			return tag;
		}
		const span = /^<span style="([^"]*)">$/.exec(tag);
		if (!span) throw new Error(`${where}: unexpected tag ${tag}`);
		if (open) throw new Error(`${where}: nested <span>`);
		open = true;
		const props = parseStyle(span[1], where);
		const fg = props.color ?? frame.color;
		const bg = props.background ?? frame.background;
		const ratio = contrast(fg, bg);
		if (ratio < MIN_CONTRAST) problems.push(`${fg} on ${bg} is ${ratio.toFixed(2)}:1`);
		return `<span style="${styleText(props)}">`;
	});
	if (open) throw new Error(`${where}: unclosed <span>`);
	const base = contrast(frame.color, frame.background);
	if (base < MIN_CONTRAST) problems.push(`${frame.color} on ${frame.background} is ${base.toFixed(2)}:1`);
	if (problems.length > 0) {
		throw new Error(`${where}: text below ${MIN_CONTRAST}:1 after the site's colours:\n  ${[...new Set(problems)].join('\n  ')}`);
	}

	return { ...measure(html), html, color: frame.color, background: frame.background };
}

const ENTITIES = { '&lt;': '<', '&gt;': '>', '&amp;': '&', '&quot;': '"', '&#39;': "'" };
const decode = (text) => text.replace(/&(?:lt|gt|amp|quot|#39);/g, (e) => ENTITIES[e]);
const encode = (text) => text.replace(/[<>&]/g, (c) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;' })[c]);

/** Size of a frame's text in columns and rows. */
function measure(html) {
	const lines = decode(html.replace(/<[^>]*>/g, '')).split('\n');
	return { columns: Math.max(...lines.map((line) => [...line].length)), rows: lines.length };
}

/** The frame as rows of cells, each a character and the style it is drawn in. */
function toCells(html) {
	const rows = [[]];
	let style = '';
	for (const [, tag, text] of html.matchAll(/(<[^>]*>)|([^<]+)/g)) {
		if (tag) {
			style = tag === '</span>' ? '' : /style="([^"]*)"/.exec(tag)[1];
			continue;
		}
		for (const ch of decode(text)) {
			if (ch === '\n') rows.push([]);
			else rows.at(-1).push({ ch, style });
		}
	}
	return rows;
}

function fromCells(rows) {
	return rows
		.map((row) => {
			let out = '';
			let open = null;
			for (const { ch, style } of row) {
				if (style !== open) {
					if (open) out += '</span>';
					if (style) out += `<span style="${style}">`;
					open = style || null;
				}
				out += encode(ch);
			}
			return open ? `${out}</span>` : out;
		})
		.join('\n');
}

/**
 * `width` cells of `row` from `left`. When that would cut text that goes on
 * inside the box (before column `right`), the line ends at the last gap
 * between words instead, preferring a run of two or more spaces (the gap
 * between a palette command and its aliases), so no word is cut in half.
 */
function trimToWord(row, left, width, right) {
	const cut = row.slice(left, left + width);
	const rest = row.slice(left + width, right).map((c) => c.ch).join('');
	const last = cut.at(-1)?.ch ?? ' ';
	if (rest.trim() === '' || /[\s─━═]/.test(last + rest[0])) return cut;
	const line = cut.map((c) => c.ch).join('');
	const gaps = [...line.matchAll(/(?<=\S) {2,}(?=\S)/g)];
	const end = gaps.length > 0 ? gaps.at(-1).index : line.lastIndexOf(' ');
	if (end <= 0) return cut;
	return [...cut.slice(0, end), ...cut.slice(end).map(() => ({ ch: ' ', style: '' }))];
}

/**
 * A narrow view of a frame for small screens: the box whose title starts
 * with `title`, from its top-left corner, `width` columns wide. Fails the build when no such box exists, so a
 * re-captured frame cannot silently lose its subject on phones.
 */
export function cropToBox(frame, title, width) {
	const rows = toCells(frame.html);
	const text = rows.map((row) => row.map((c) => c.ch).join(''));
	const top = text.findIndex((line) => line.includes(`┌${title}`));
	if (top === -1) throw new Error(`the frame has no box titled "${title}" to show on narrow screens`);
	const left = [...text[top].slice(0, text[top].indexOf(`┌${title}`))].length;
	let bottom = text.findIndex((line, i) => i > top && [...line][left] === '└');
	if (bottom === -1) bottom = rows.length - 1;
	const topRow = [...text[top]];
	const right = topRow.indexOf('┐', left) === -1 ? topRow.length : topRow.indexOf('┐', left);
	const cropped = rows.slice(top, bottom + 1).map((row) => trimToWord(row, left, width, right));
	const html = fromCells(cropped);
	return { ...frame, ...measure(html), html };
}

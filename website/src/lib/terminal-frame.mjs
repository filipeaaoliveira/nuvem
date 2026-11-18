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

	const lines = html.replace(/<[^>]*>/g, '').replace(/&[a-z]+;|&#\d+;/g, 'x').split('\n');
	return {
		html,
		color: frame.color,
		background: frame.background,
		columns: Math.max(...lines.map((line) => [...line].length)),
		rows: lines.length,
	};
}

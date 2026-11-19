#!/usr/bin/env node
// Checks the home page's sky in a real browser, against the built site in
// dist/ (run `npm run build` first). It fails when:
//
// - a line of text on the sky is not wholly on one ground, with 12px of that
//   ground around it: a ground edge crosses it or comes too close;
// - a decoration (a cloud, a star, the moon) comes within 16px of text, a
//   service tile, the diagram, a terminal window or the install command;
// - a seam between two grounds leaves a line of another color: any straight
//   run of off-color pixels longer than 48px, at four pixel ratios;
// - two grounds of the sky meet along a straight line longer than 64px where
//   it can be seen: every change of ground must be an edge of cells.
//
// It drives Chrome or Chromium over the DevTools protocol, with no
// dependency beyond Node: set CHROME_PATH if the browser is not found.
// Usage: node scripts/check-sky.mjs [--dist <dir>] [width,width,...]

import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { extname, join, normalize, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const args = process.argv.slice(2);
const distAt = args.indexOf('--dist');
const DIST = distAt === -1 ? fileURLToPath(new URL('../dist/', import.meta.url)) : `${resolve(args.splice(distAt, 2)[1])}${sep}`;
const WIDTHS = (args[0] ?? '360,400,480,540,600,700,768,830,900,1000,1024,1100,1200,1280,1366,1440,1600,1920,2560')
	.split(',')
	.map(Number);
const TEXT_CLEARANCE = 12;
const DECORATION_CLEARANCE = 16;
const SEAM_WIDTHS = [360, 480, 600, 768, 1280, 1440];
const SEAM_RUN = 48;
const FLAT_RUN = 64;
const SEAM_SCALES = [1, 1.25, 1.5, 2];
// The grounds of the sky: Pale Methyl Blue, Naples Yellow, the brown night, near-black.
const GROUNDS = ['#54afb4', '#fbe6a0', '#423b36', '#1b1816'];

if (!existsSync(join(DIST, 'index.html'))) {
	console.error(`${join(DIST, 'index.html')} is missing. Run \`npm run build\` first.`);
	process.exit(2);
}

// A static server for dist/.
const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png', '.webp': 'image/webp', '.woff2': 'font/woff2', '.json': 'application/json', '.mp4': 'video/mp4', '.webm': 'video/webm' };
const server = createServer((req, res) => {
	let path = normalize(join(DIST, decodeURIComponent(new URL(req.url, 'http://x').pathname)));
	if (!path.startsWith(DIST)) return res.writeHead(403).end();
	if (existsSync(path) && statSync(path).isDirectory()) path = join(path, 'index.html');
	if (!existsSync(path)) return res.writeHead(404).end();
	res.writeHead(200, { 'Content-Type': TYPES[extname(path)] ?? 'application/octet-stream' });
	res.end(readFileSync(path));
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const BASE = `http://127.0.0.1:${server.address().port}/`;

// Chrome, headless, with the DevTools protocol on a WebSocket.
const CHROME =
	process.env.CHROME_PATH ??
	[
		'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
		'/Applications/Chromium.app/Contents/MacOS/Chromium',
		'/usr/bin/google-chrome',
		'/usr/bin/google-chrome-stable',
		'/usr/bin/chromium',
		'/usr/bin/chromium-browser',
	].find((p) => existsSync(p));
if (!CHROME) {
	console.error('Chrome or Chromium was not found. Set CHROME_PATH to its executable.');
	process.exit(2);
}
const profile = mkdtempSync(join(tmpdir(), 'nuvem-sky-'));
const chrome = spawn(CHROME, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check', '--hide-scrollbars', '--force-color-profile=srgb', 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
const wsUrl = await new Promise((resolve, reject) => {
	let err = '';
	chrome.stderr.on('data', (d) => {
		err += d;
		const m = /DevTools listening on (ws:\/\/\S+)/.exec(err);
		if (m) resolve(m[1]);
	});
	chrome.on('exit', () => reject(new Error(`Chrome exited before it was ready:\n${err}`)));
});

const ws = new WebSocket(wsUrl);
await new Promise((resolve) => ws.addEventListener('open', resolve, { once: true }));
let nextId = 0;
const pending = new Map();
const waiters = [];
ws.addEventListener('message', ({ data }) => {
	const msg = JSON.parse(data);
	if (msg.id && pending.has(msg.id)) {
		const { resolve, reject } = pending.get(msg.id);
		pending.delete(msg.id);
		msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
	} else if (msg.method) {
		for (const w of [...waiters]) if (w.method === msg.method) (waiters.splice(waiters.indexOf(w), 1), w.resolve(msg.params));
	}
});
let session;
const send = (method, params = {}) =>
	new Promise((resolve, reject) => {
		const id = ++nextId;
		pending.set(id, { resolve, reject });
		ws.send(JSON.stringify({ id, method, params, ...(session && !method.startsWith('Target.') ? { sessionId: session } : {}) }));
	});
const once = (method) => new Promise((resolve) => waiters.push({ method, resolve }));
const evaluate = async (fn, arg) => {
	const { result, exceptionDetails } = await send('Runtime.evaluate', { expression: `(${fn})(${JSON.stringify(arg ?? null)})`, awaitPromise: true, returnByValue: true });
	if (exceptionDetails) throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
	return result.value;
};

const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
({ sessionId: session } = await send('Target.attachToTarget', { targetId, flatten: true }));
await send('Page.enable');
await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });

async function open(width, scale = 1) {
	await send('Emulation.setDeviceMetricsOverride', { width, height: 900, deviceScaleFactor: scale, mobile: false });
	const loaded = once('Page.loadEventFired');
	await send('Page.navigate', { url: BASE });
	await loaded;
	await evaluate(() => document.fonts.ready.then(() => new Promise((r) => setTimeout(r, 150))));
}

// Opaque blocks that hide the sky behind them are painted in this color, so
// a seam they cover is not counted.
const MASK = '#ff00ff';

/**
 * A screenshot of the whole page with only the grounds left: text,
 * decorations and pictures hidden, and the opaque blocks (windows, plates,
 * tiles, the diagram) painted in the mask color.
 */
async function groundsOnly() {
	await evaluate((mask) => {
		const style = document.createElement('style');
		style.textContent = `
			.nv-landing *:not(.dusk):not(.night):not(.sky-edge), .hero *, header.header *, .colophon * { color: transparent !important; background-color: transparent !important; background-image: none !important; border-color: transparent !important; box-shadow: none !important; outline: none !important; text-decoration-color: transparent !important; }
			.nv-landing .sky-edge svg { visibility: visible !important; }
			.nv-landing img, .nv-landing svg, .nv-landing video, .nv-landing pre, .nv-landing [data-deco], .hero h1, header.header img, header.header svg { visibility: hidden !important; }
			.nv-landing :is(.recording, .window, .install, .help, .plate ul, .flow):not(#_) { visibility: visible !important; background-color: ${mask} !important; }
			.nv-landing :is(.recording, .window, .install, .help, .plate ul, .flow) * { visibility: hidden !important; }
			main:has(.nv-landing) .hero::after { display: none !important; }`;
		document.head.append(style);
		return new Promise((r) => setTimeout(r, 100));
	}, MASK);
	const { cssContentSize } = await send('Page.getLayoutMetrics');
	const { data } = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true, clip: { x: 0, y: 0, width: cssContentSize.width, height: cssContentSize.height, scale: 1 } });
	return data;
}

const failures = [];

for (const width of WIDTHS) {
	await open(width);
	const layout = await evaluate(
		({ textClearance, decorationClearance }) => {
			const box = (r) => ({ l: r.left, t: r.top + scrollY, r: r.right, b: r.bottom + scrollY });
			const shown = (e) => {
				for (let x = e; x; x = x.parentElement) {
					const cs = getComputedStyle(x);
					if (cs.display === 'none' || cs.visibility === 'hidden') return false;
				}
				return true;
			};
			// Decorations, and the hero's day clouds (drawn in its ::after).
			const decorations = [...document.querySelectorAll('[data-deco]')]
				.filter(shown)
				.map((e) => ({ name: e.getAttribute('class') ?? e.tagName, ...box(e.getBoundingClientRect()) }))
				.filter((d) => d.r - d.l > 0);
			const hero = document.querySelector('main .hero');
			const after = hero && getComputedStyle(hero, '::after');
			if (after && after.content !== 'none') {
				const h = hero.getBoundingClientRect();
				const right = h.right - parseFloat(after.right);
				decorations.push({ name: 'day clouds', l: right - parseFloat(after.width), r: right, t: h.top + scrollY, b: h.bottom + scrollY });
			}
			// Everything a decoration must keep clear of.
			const obstacles = [];
			const panel = document.querySelector('main .content-panel');
			const walker = document.createTreeWalker(panel, NodeFilter.SHOW_TEXT);
			const lines = [];
			for (let n; (n = walker.nextNode()); ) {
				if (!n.textContent.trim()) continue;
				const el = n.parentElement;
				if (el.closest('.sr-only, svg, noscript') || !shown(el)) continue;
				const range = document.createRange();
				range.selectNodeContents(n);
				for (const r of range.getClientRects()) {
					if (r.width < 1) continue;
					const line = { what: n.textContent.trim().slice(0, 32), ...box(r) };
					obstacles.push(line);
					// Lines on an opaque plate have the plate as their ground.
					if (!el.closest('.install, .recording, .window, .plate, .flow, .help, [role=tab], .colophon')) lines.push(line);
				}
			}
			for (const e of panel.querySelectorAll('.plate a, .flow, .recording, .window, .install, .help, [role=tab], kbd, .wordmark')) {
				if (shown(e)) obstacles.push({ what: e.getAttribute('class') ?? e.tagName, ...box(e.getBoundingClientRect()) });
			}
			const near = [];
			for (const d of decorations)
				for (const o of obstacles)
					if (d.l < o.r + decorationClearance && d.r > o.l - decorationClearance && d.t < o.b + decorationClearance && d.b > o.t - decorationClearance)
						near.push(`${d.name} within ${decorationClearance}px of "${o.what}"`);
			return { decorations: decorations.length, near: [...new Set(near)], lines: lines.map((l) => ({ ...l, l: l.l - textClearance, r: l.r + textClearance, t: l.t - textClearance, b: l.b + textClearance })) };
		},
		{ textClearance: TEXT_CLEARANCE, decorationClearance: DECORATION_CLEARANCE },
	);
	const png = await groundsOnly();
	const crossed = await evaluate(
		async ({ png, lines, grounds, flatRun, groundRgb: gs, mask }) => {
			const grounds2 = gs;
			const img = new Image();
			img.src = `data:image/png;base64,${png}`;
			await img.decode();
			const canvas = new OffscreenCanvas(img.width, img.height);
			const g = canvas.getContext('2d', { willReadFrequently: true });
			g.drawImage(img, 0, 0);
			const out = [];
			for (const l of lines) {
				const x0 = Math.max(0, Math.floor(l.l));
				const y0 = Math.max(0, Math.floor(l.t));
				const x1 = Math.min(img.width, Math.ceil(l.r));
				const y1 = Math.min(img.height, Math.ceil(l.b));
				if (x1 <= x0 || y1 <= y0) continue;
				const d = g.getImageData(x0, y0, x1 - x0, y1 - y0).data;
				const maskBlend = (px) =>
					grounds2.some((gr) => {
						const c = [0, 1, 2].reduce((a, b) => (Math.abs(mask[b] - gr[b]) > Math.abs(mask[a] - gr[a]) ? b : a));
						const t = (px[c] - gr[c]) / (mask[c] - gr[c]);
						return t > 0 && t < 1 && px.every((v, k) => Math.abs(v - (gr[k] + t * (mask[k] - gr[k]))) <= 6);
					});
				const colors = new Set();
				for (let i = 0; i < d.length; i += 4) {
					const k = (d[i] << 16) | (d[i + 1] << 8) | d[i + 2];
					// The mask (an opaque block beside the line) and its blended
					// edge are not a ground.
					const px = [d[i], d[i + 1], d[i + 2]];
					if (!(px[0] === 255 && px[1] === 0 && px[2] === 255) && !maskBlend(px)) colors.add(k);
				}
				if (colors.size > 1) out.push(`"${l.what}" at y=${Math.round(l.t)} touches ${colors.size} grounds (${[...colors].map((c) => '#' + c.toString(16).padStart(6, '0')).join(' ')})`);
			}
			// Straight changes of ground: two rows where one ground sits on
			// another along more than flatRun pixels.
			const key = (d, i) => {
				const k = (d[i] << 16) | (d[i + 1] << 8) | d[i + 2];
				return grounds.includes(k) ? k : -1;
			};
			let above = g.getImageData(0, 0, img.width, 1).data;
			for (let y = 1; y < img.height - 3; y++) {
				const below = g.getImageData(0, y, img.width, 1).data;
				let run = 0;
				let pair = '';
				for (let x = 0; x <= img.width; x++) {
					const a = x < img.width ? key(above, 4 * x) : -1;
					const b = x < img.width ? key(below, 4 * x) : -1;
					const p = a !== -1 && b !== -1 && a !== b ? `${a}/${b}` : '';
					if (p && p === pair) run++;
					else {
						if (run > flatRun) out.push(`a straight change of ground ${run}px long at y=${y}, x=${x - run}`);
						run = p ? 1 : 0;
						pair = p;
					}
				}
				above = below;
			}
			return out;
		},
		{ png, lines: layout.lines, grounds: GROUNDS.map((h) => parseInt(h.slice(1), 16)), flatRun: FLAT_RUN, groundRgb: GROUNDS.map((h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16))), mask: [255, 0, 255] },
	);
	const ok = layout.near.length === 0 && crossed.length === 0;
	console.log(`${String(width).padStart(4)}px  ${layout.lines.length} lines on the sky, ${layout.decorations} decorations  ${ok ? 'ok' : 'FAIL'}`);
	for (const f of [...crossed, ...layout.near]) console.log(`        ${f}`);
	if (!ok) failures.push(width);
}

// Seams: with only the grounds left, every pixel must be one of the
// grounds' own colors (or the mask). A straight run of blended pixels is a
// line at a seam.
const groundRgb = [...GROUNDS, MASK].map((h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16)));
for (const width of SEAM_WIDTHS) {
	for (const scale of SEAM_SCALES) {
		await open(width, scale);
		const png = await groundsOnly();
		const stray = await evaluate(
			async ({ png, grounds, scale, runLimit, mask }) => {
				const img = new Image();
				img.src = `data:image/png;base64,${png}`;
				await img.decode();
				const canvas = new OffscreenCanvas(img.width, img.height);
				const g = canvas.getContext('2d', { willReadFrequently: true });
				g.drawImage(img, 0, 0);
				// The edge of a masked block blends the mask with the ground
				// beside it at a fractional ratio: that is not a seam.
				const maskBlend = (px) =>
					grounds.slice(0, -1).some((gr) => {
						const c = [0, 1, 2].reduce((a, b) => (Math.abs(mask[b] - gr[b]) > Math.abs(mask[a] - gr[a]) ? b : a));
						const t = (px[c] - gr[c]) / (mask[c] - gr[c]);
						return t > 0 && t < 1 && px.every((v, k) => Math.abs(v - (gr[k] + t * (mask[k] - gr[k]))) <= 6);
					});
				const rows = [];
				const samples = [];
				// The last rows are the page's own fractional end, not a seam.
				for (let y = 0; y < img.height - 3; y++) {
					const d = g.getImageData(0, y, img.width, 1).data;
					let run = 0;
					let longest = 0;
					let at = 0;
					for (let i = 0; i < d.length; i += 4) {
						const px = [d[i], d[i + 1], d[i + 2]];
						const ok = grounds.some((gr) => px.every((v, c) => Math.abs(v - gr[c]) <= 2)) || maskBlend(px);
						run = ok ? 0 : run + 1;
						if (run > longest) (longest = run), (at = i / 4 - run + 1);
					}
					// A seam shows as a straight run of off-color pixels.
					if (longest > runLimit * scale) {
						rows.push(y);
						const i = 4 * (at + Math.floor(longest / 2));
						if (samples.length < 3) samples.push(`css y=${(y / scale).toFixed(1)}, x=${(at / scale).toFixed(0)}, ${(longest / scale).toFixed(0)}px of #${[d[i], d[i + 1], d[i + 2]].map((v) => v.toString(16).padStart(2, '0')).join('')}`);
					}
				}
				return { rows, samples };
			},
			{ png, grounds: groundRgb, scale, runLimit: SEAM_RUN, mask: [255, 0, 255] },
		);
		const ok = stray.rows.length === 0;
		console.log(`seams ${width}px at ${scale}x  ${ok ? 'ok' : `FAIL: ${stray.rows.length} rows with a line, first ${stray.samples.join('; ')}`}`);
		if (!ok) failures.push(`seams ${width}@${scale}`);
	}
}

ws.close();
const exited = new Promise((resolve) => chrome.once('exit', resolve));
chrome.kill();
await exited;
server.close();
rmSync(profile, { recursive: true, force: true, maxRetries: 5 });
if (failures.length) {
	console.error(`\nThe sky check failed at: ${failures.join(', ')}`);
	process.exit(1);
}
console.log('\nThe sky check passed.');

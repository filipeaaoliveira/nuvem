#!/usr/bin/env node
// Checks the home page's sky in a real browser, against the built site in
// dist/ (run `npm run build` first). It fails when:
//
// - a line of text on the sky is not wholly on one ground, with 12px of that
//   ground around it: a ground edge crosses it or comes too close;
// - a decoration (a cloud, a star, the moon) comes within 16px of text, a
//   service tile, the diagram, a terminal window or the install command;
// - a seam between two grounds leaves a hairline of another color.
//
// It drives Chrome or Chromium over the DevTools protocol, with no
// dependency beyond Node: set CHROME_PATH if the browser is not found.
// Usage: node scripts/check-sky.mjs [width,width,...]

import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const DIST = fileURLToPath(new URL('../dist/', import.meta.url));
const WIDTHS = (process.argv[2] ?? '360,400,480,540,600,700,768,830,900,1000,1024,1100,1200,1280,1366,1440,1600,1920,2560')
	.split(',')
	.map(Number);
const TEXT_CLEARANCE = 12;
const DECORATION_CLEARANCE = 16;
const SEAM_WIDTHS = [600, 768, 1280, 1440];
const SEAM_SCALES = [1, 1.25, 1.5, 2];
// The grounds of the sky: Pale Methyl Blue, Naples Yellow, the brown night, near-black.
const GROUNDS = ['#54afb4', '#fbe6a0', '#423b36', '#1b1816'];

if (!existsSync(join(DIST, 'index.html'))) {
	console.error('dist/index.html is missing. Run `npm run build` first.');
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

/** A screenshot of the whole page with everything but the grounds hidden. */
async function groundsOnly() {
	await evaluate(() => {
		const style = document.createElement('style');
		style.textContent = `
			.nv-landing *:not(.dusk):not(.night):not(.sky-edge), .hero *, header.header *, .colophon * { color: transparent !important; background-color: transparent !important; border-color: transparent !important; box-shadow: none !important; outline: none !important; text-decoration-color: transparent !important; }
			.nv-landing img, .nv-landing svg, .nv-landing video, .nv-landing pre, .nv-landing .flow, .nv-landing .plate, .nv-landing [data-deco], .hero h1, header.header img, header.header svg { visibility: hidden !important; }
			main:has(.nv-landing) .hero::after { display: none !important; }`;
		document.head.append(style);
		return new Promise((r) => setTimeout(r, 100));
	});
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
		async ({ png, lines }) => {
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
				const colors = new Set();
				for (let i = 0; i < d.length; i += 4) colors.add((d[i] << 16) | (d[i + 1] << 8) | d[i + 2]);
				if (colors.size > 1) out.push(`"${l.what}" at y=${Math.round(l.t)} touches ${colors.size} grounds`);
			}
			return out;
		},
		{ png, lines: layout.lines },
	);
	const ok = layout.near.length === 0 && crossed.length === 0;
	console.log(`${String(width).padStart(4)}px  ${layout.lines.length} lines on the sky, ${layout.decorations} decorations  ${ok ? 'ok' : 'FAIL'}`);
	for (const f of [...crossed, ...layout.near]) console.log(`        ${f}`);
	if (!ok) failures.push(width);
}

// Seams: with everything but the grounds hidden, every pixel must be one of
// the grounds' own colors. A blended pixel is a hairline at a seam.
const groundRgb = GROUNDS.map((h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16)));
for (const width of SEAM_WIDTHS) {
	for (const scale of SEAM_SCALES) {
		await open(width, scale);
		const png = await groundsOnly();
		const stray = await evaluate(
			async ({ png, grounds, scale }) => {
				const img = new Image();
				img.src = `data:image/png;base64,${png}`;
				await img.decode();
				const canvas = new OffscreenCanvas(img.width, img.height);
				const g = canvas.getContext('2d', { willReadFrequently: true });
				g.drawImage(img, 0, 0);
				const rows = [];
				const samples = [];
				// The last rows are the page's own fractional end, not a seam.
				for (let y = 0; y < img.height - 3; y++) {
					const d = g.getImageData(0, y, img.width, 1).data;
					let off = 0;
					for (let i = 0; i < d.length; i += 4) {
						const ok = grounds.some(([r, gg, b]) => Math.abs(d[i] - r) <= 2 && Math.abs(d[i + 1] - gg) <= 2 && Math.abs(d[i + 2] - b) <= 2);
						if (!ok) off++;
					}
					// A seam shows as a hairline across the whole row. (The soft
					// edges of single cells at a fractional scale do not.)
					if (off > img.width * 0.9) {
						rows.push(y);
						const i = 4 * Math.floor(img.width / 2);
						if (samples.length < 3) samples.push(`css y=${(y / scale).toFixed(1)}: #${[d[i], d[i + 1], d[i + 2]].map((v) => v.toString(16).padStart(2, '0')).join('')}`);
					}
				}
				return { rows, samples };
			},
			{ png, grounds: groundRgb, scale },
		);
		const ok = stray.rows.length === 0;
		console.log(`seams ${width}px at ${scale}x  ${ok ? 'ok' : `FAIL: ${stray.rows.length} off-color rows, first ${stray.samples.join('; ')}`}`);
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

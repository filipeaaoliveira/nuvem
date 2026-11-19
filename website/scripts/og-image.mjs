#!/usr/bin/env node
// Draws public/og.png, the 1200 by 630 card that link previews show: the
// home page's day sky, the cell wordmark, a cloud, Naples cloud tops rising
// at the bottom, and the pitch. Everything but the pitch comes from
// src/lib/sky-cells.mjs, so the card matches the site.
//
// It renders an HTML page with Chrome or Chromium headless (set CHROME_PATH
// if it is not found) and compresses the result with sharp.
// Usage: npm run og

import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import sharp from 'sharp';
import { COLORS, cloudSvg, edgeSvg, skyline, wordmarkSvg } from '../src/lib/sky-cells.mjs';

const OUT = fileURLToPath(new URL('../public/og.png', import.meta.url));
const require = createRequire(import.meta.url);
const font = pathToFileURL(require.resolve('@fontsource-variable/schibsted-grotesk/files/schibsted-grotesk-latin-wght-normal.woff2')).href;

const W = 1200;
const H = 630;
const CELL = 12;
const dusk = skyline({ w: W / CELL, h: 11, seed: 4, r: [2.4, 5.2], solid: 3, towers: [[70, 4.5]] });

const html = `<!doctype html>
<meta charset="utf-8">
<style>
	@font-face { font-family: 'Schibsted Grotesk'; src: url('${font}') format('woff2'); font-weight: 400 900; }
	html, body { margin: 0; width: ${W}px; height: ${H}px; overflow: hidden; background: ${COLORS.methyl}; }
	.mark { position: absolute; left: 72px; top: 84px; }
	.cloud { position: absolute; right: -60px; top: 56px; }
	.cloud-small { position: absolute; right: 360px; top: 40px; }
	h1 { position: absolute; left: 72px; top: 236px; width: 760px; margin: 0; color: ${COLORS.ink};
		font: 800 78px/0.98 'Schibsted Grotesk', sans-serif; letter-spacing: -0.035em; }
	.dusk { position: absolute; left: 0; bottom: 0; line-height: 0; }
	svg { display: block; }
</style>
<div class="mark">${wordmarkSvg(14, COLORS.ink)}</div>
<div class="cloud">${cloudSvg('large', 8, COLORS.text, COLORS.coral)}</div>
<div class="cloud-small">${cloudSvg('small', 5, COLORS.text, COLORS.coral)}</div>
<h1>A terminal UI for Google&nbsp;Cloud</h1>
<div class="dusk">${edgeSvg(dusk, CELL, COLORS.naples)}</div>`;

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

const dir = mkdtempSync(join(tmpdir(), 'nuvem-og-'));
const page = join(dir, 'og.html');
const shot = join(dir, 'og.png');
writeFileSync(page, html);
const run = spawnSync(
	CHROME,
	['--headless=new', `--user-data-dir=${join(dir, 'profile')}`, '--hide-scrollbars', '--force-device-scale-factor=1', '--force-color-profile=srgb', `--window-size=${W},${H}`, '--virtual-time-budget=2000', `--screenshot=${shot}`, pathToFileURL(page).href],
	{ encoding: 'utf8' },
);
if (!existsSync(shot)) {
	console.error(`Chrome did not write the screenshot:\n${run.stderr}`);
	process.exit(1);
}
const info = await sharp(shot).resize(W, H, { fit: 'cover', position: 'top' }).png({ palette: true, colors: 32, compressionLevel: 9 }).toFile(OUT);
rmSync(dir, { recursive: true, force: true });
console.log(`Wrote public/og.png (${info.width} by ${info.height}, ${Math.round(info.size / 1024)} KB).`);

// Fills the Content-Security-Policy in dist/_headers with the SHA-256 hash of
// every inline <script> the build emitted. The policy then allows exactly
// those scripts and files from the site itself, and nothing else.

import { createHash } from 'node:crypto';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const PLACEHOLDER = 'INLINE_SCRIPT_HASHES';

async function htmlFiles(dir) {
	const out = [];
	for (const entry of await readdir(dir, { withFileTypes: true })) {
		const path = join(dir, entry.name);
		if (entry.isDirectory()) out.push(...(await htmlFiles(path)));
		else if (entry.name.endsWith('.html')) out.push(path);
	}
	return out;
}

export function securityHeadersIntegration() {
	return {
		name: 'nuvem-security-headers',
		hooks: {
			'astro:build:done': async ({ dir, logger }) => {
				const root = fileURLToPath(dir);
				const hashes = new Set();
				for (const file of await htmlFiles(root)) {
					const html = await readFile(file, 'utf8');
					for (const [, attrs, body] of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/g)) {
						if (/\ssrc=/.test(attrs) || /type="application\/(ld\+)?json"/.test(attrs)) continue;
						hashes.add(`'sha256-${createHash('sha256').update(body).digest('base64')}'`);
					}
				}
				const headersPath = join(root, '_headers');
				const headers = await readFile(headersPath, 'utf8');
				if (!headers.includes(PLACEHOLDER)) throw new Error(`${PLACEHOLDER} is missing from public/_headers`);
				await writeFile(headersPath, headers.replaceAll(PLACEHOLDER, [...hashes].sort().join(' ')));
				logger.info(`Content-Security-Policy allows ${hashes.size} inline scripts by hash.`);
			},
		},
	};
}

// Serves the site's redirects (the `redirects` in astro.config.mjs) as real
// HTTP redirects on Cloudflare, through dist/_redirects.
//
// Astro writes a page with a meta refresh for each redirect. That page still
// works with `npm run preview` and on any static host, so it stays; this also
// gives it a `lang`, which Astro leaves out.

import { appendFile, readFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

export function redirectsIntegration() {
	let redirects = {};
	return {
		name: 'nuvem-redirects',
		hooks: {
			'astro:config:done': ({ config }) => {
				redirects = config.redirects ?? {};
			},
			'astro:build:done': async ({ dir, logger }) => {
				const root = fileURLToPath(dir);
				const rules = [];
				for (const [from, target] of Object.entries(redirects)) {
					const to = typeof target === 'string' ? target : target.destination;
					const status = typeof target === 'string' ? 301 : target.status;
					const bare = from.replace(/\/+$/, '');
					// Both spellings of the address, with and without the trailing slash.
					rules.push(`${bare} ${to} ${status}`, `${bare}/ ${to} ${status}`);

					const page = join(root, bare, 'index.html');
					if (existsSync(page)) {
						const html = await readFile(page, 'utf8');
						if (!/<html\b/i.test(html)) await writeFile(page, html.replace(/^<!doctype html>/i, '$&<html lang="en">'));
					}
				}
				if (rules.length === 0) return;
				await appendFile(join(root, '_redirects'), `${rules.join('\n')}\n`);
				logger.info(`_redirects has ${rules.length} rules.`);
			},
		},
	};
}

// The services nuvem browses, as the docs list them: read from the table at
// the top of `docs/reference/services/overview.md` (Service, Id, Aliases,
// Category). The home page draws its service plate from this, so the plate
// changes when the docs do and never holds a hand-typed copy of the list.
//
// The build fails when the table cannot be read, when a row links to a page
// that does not exist, or when a page in `docs/reference/services/` is
// missing from the table.

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { DOCS_DIR } from './docs-source.mjs';

const SERVICES_DIR = join(DOCS_DIR, 'reference', 'services');
const OVERVIEW = 'overview.md';
const HEADER = /^\|\s*Service\s*\|\s*Id\s*\|\s*Aliases\s*\|\s*Category\s*\|\s*$/;

const cells = (line) =>
	line
		.trim()
		.replace(/^\||\|$/g, '')
		.split('|')
		.map((c) => c.trim());

/**
 * Every service, in the order of the docs table:
 * `{ name, file, id, category }`, where `file` is relative to `docs/`.
 */
export function readServices() {
	const lines = readFileSync(join(SERVICES_DIR, OVERVIEW), 'utf8').split(/\r?\n/);
	const start = lines.findIndex((line) => HEADER.test(line));
	if (start === -1) {
		throw new Error(`docs/reference/services/${OVERVIEW}: no "| Service | Id | Aliases | Category |" table for the home page's service plate`);
	}
	const services = [];
	for (const line of lines.slice(start + 2)) {
		if (!line.trim().startsWith('|')) break;
		const [service, id, , category] = cells(line);
		const link = /^\[([^\]]+)\]\(([^)#]+)\)$/.exec(service);
		if (!link) throw new Error(`docs/reference/services/${OVERVIEW}: "${service}" is not a link to a service page`);
		const [, name, page] = link;
		if (!existsSync(join(SERVICES_DIR, page))) throw new Error(`docs/reference/services/${OVERVIEW}: ${page} does not exist`);
		services.push({ name, file: `reference/services/${page}`, id: id.replace(/`/g, ''), category });
	}
	const listed = new Set(services.map((s) => s.file));
	const missing = readdirSync(SERVICES_DIR)
		.filter((f) => f.endsWith('.md') && f !== OVERVIEW && !listed.has(`reference/services/${f}`))
		.map((f) => `docs/reference/services/${f}`);
	if (missing.length > 0) {
		throw new Error(`docs/reference/services/${OVERVIEW}: the services table does not list ${missing.join(', ')}`);
	}
	return services;
}

const SUPPORT_HEADER = /^\|\s*Service\s*\|\s*`Enter` opens\s*\|/;

/**
 * How many services open to more detail: the rows of the "What each service
 * supports" table in the same page whose `Enter` opens column is not
 * "nothing". The build fails when that table cannot be read.
 */
export function countDetailServices() {
	const lines = readFileSync(join(SERVICES_DIR, OVERVIEW), 'utf8').split(/\r?\n/);
	const start = lines.findIndex((line) => SUPPORT_HEADER.test(line));
	if (start === -1) {
		throw new Error(`docs/reference/services/${OVERVIEW}: no "| Service | \`Enter\` opens |" table to count the services with a detail view`);
	}
	let count = 0;
	for (const line of lines.slice(start + 2)) {
		if (!line.trim().startsWith('|')) break;
		if (cells(line)[1] !== 'nothing') count++;
	}
	return count;
}

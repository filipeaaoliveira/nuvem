// The home page's sky, drawn in terminal cells: the edges where one ground
// gives way to the next, the clouds, the stars and the moon. Everything is
// a grid of square cells in flat palette colors, built at build time as
// SVG, so nothing here runs in the browser.
//
// Cells are drawn on whole pixels: each function takes the cell size in
// pixels and returns an SVG whose width and height are exact multiples of
// it, to be shown at exactly that size (as an element, or as a background
// with a matching background-size). Edges are deterministic: the same seed
// gives the same skyline on every build.

/** The palette as hex, for SVG; the same values as the tokens in tokens.css. */
export const COLORS = {
	ground: '#423b36',
	naples: '#fbe6a0',
	coral: '#f58e84',
	methyl: '#54afb4',
	eugenia: '#e2625e',
	ink: '#1b1816',
	text: '#f1ece6',
	muted: '#cbc1b6', // --nv-text-muted
};

/** A small seeded generator (mulberry32), so a skyline never changes between builds. */
function random(seed) {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6d2b79f5) >>> 0;
		let t = a;
		t = Math.imul(t ^ (t >>> 15), t | 1);
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

/**
 * A grid of `w` by `h` cells, each filled when its center lies inside one
 * of the shapes: `{ rect: [x0, y0, x1, y1] }` or `{ disc: [cx, cy, r] }`.
 */
function rasterize(w, h, shapes) {
	const inside = (s, x, y) =>
		s.rect ? s.rect[0] <= x && x < s.rect[2] && s.rect[1] <= y && y < s.rect[3] : (x - s.disc[0]) ** 2 + (y - s.disc[1]) ** 2 <= s.disc[2] ** 2;
	return Array.from({ length: h }, (_, j) => Array.from({ length: w }, (_, i) => shapes.some((s) => inside(s, i + 0.5, j + 0.5))));
}

/**
 * Solid cells as one path on a grid of `c` pixels: each row's runs merged,
 * then runs that repeat in the rows below merged into one rectangle.
 */
function solidPath(grid, c) {
	const open = new Map();
	const rects = [];
	grid.forEach((row, j) => {
		const seen = new Set();
		let i = 0;
		while (i < row.length) {
			if (!row[i]) {
				i++;
				continue;
			}
			let k = i;
			while (k < row.length && row[k]) k++;
			const key = `${i},${k}`;
			seen.add(key);
			const run = open.get(key);
			if (run && run.end === j) run.end = j + 1;
			else {
				if (run) rects.push(run);
				open.set(key, { i, k, start: j, end: j + 1 });
			}
			i = k;
		}
		for (const [key, run] of open) if (!seen.has(key) && run.end <= j) (rects.push(run), open.delete(key));
	});
	rects.push(...open.values());
	return rects.map(({ i, k, start, end }) => `M${i * c} ${start * c}h${(k - i) * c}v${(end - start) * c}h${(i - k) * c}z`).join('');
}

let patterns = 0;

/**
 * A fill that turns a solid shape into separate cells: a square one pixel
 * smaller than the cell (the motif's hand-set gap), or, for `half`, two
 * diagonal quarters of the cell (a half-tone, like ░). Shapes drawn on the
 * cell grid line up with it exactly.
 */
function cellFill(c, color, half = false) {
	const id = `cells${++patterns}`;
	const q = Math.floor(c / 2);
	const mark = half
		? `<path d="M0 0h${q}v${q}h${-q}zM${q} ${q}h${q}v${q}h${-q}z" fill="${color}"/>`
		: `<path d="M0 0h${c - 1}v${c - 1}h${1 - c}z" fill="${color}"/>`;
	return { def: `<pattern id="${id}" width="${c}" height="${c}" patternUnits="userSpaceOnUse">${mark}</pattern>`, fill: `url(#${id})` };
}

/** `paths`: [d, fill] pairs; a fill may be a cellFill() for gapped or half-tone cells. */
function svg(w, h, c, paths) {
	const live = paths.filter(([d]) => d);
	const defs = live.map(([, f]) => (typeof f === 'object' ? f.def : '')).join('');
	const body = live.map(([d, f]) => `<path d="${d}" fill="${typeof f === 'object' ? f.fill : f}"/>`).join('');
	return `<svg xmlns="http://www.w3.org/2000/svg" width="${w * c}" height="${h * c}" viewBox="0 0 ${w * c} ${h * c}" shape-rendering="crispEdges" aria-hidden="true" focusable="false">${defs ? `<defs>${defs}</defs>` : ''}${body}</svg>`;
}

/**
 * Cloud tops rising from a filled bottom: the edge of the ground below.
 * `w` and `h` in cells; `r` the range of a lobe's radius; `towers` adds a
 * few taller heads at given columns; no flat stretch of the top is longer
 * than `maxFlat` cells.
 */
export function skyline({ w, h, seed, r: [rmin, rmax], solid = 2, towers = [], maxFlat = 3 }) {
	const rnd = random(seed);
	const base = h - solid;
	const shapes = [{ rect: [0, base, w, h] }];
	let x = -2;
	while (x < w + 2) {
		const r = rmin + rnd() * (rmax - rmin);
		const cy = base + r * (0.15 + rnd() * 0.4);
		shapes.push({ disc: [x + r, cy, r] });
		if (rnd() < 0.45) {
			const r2 = r * (0.45 + rnd() * 0.25);
			shapes.push({ disc: [x + r + (rnd() - 0.5) * r, cy - r * 0.75, r2] });
		}
		x += r * (1.1 + rnd() * 0.6);
	}
	for (const [tx, tr] of towers) {
		shapes.push({ disc: [tx, base + tr * 0.2, tr] });
		shapes.push({ disc: [tx + tr * 0.5, base - tr * 0.55, tr * 0.6] });
	}
	return breakFlats(rasterize(w, h, shapes), maxFlat);
}

/**
 * No change of ground in a drawing runs straight for more than `maxFlat`
 * cells: wherever filled cells meet empty ones along a longer row boundary
 * (the top of a cloud, the underside of a lobe, the drawing's own top row),
 * two cells every `maxFlat` step across it, so the edge never reads as a
 * straight line. Repeats until no long run is left.
 */
function breakFlats(grid, maxFlat) {
	const h = grid.length;
	const w = grid[0].length;
	// Above the drawing is the other ground; below it, its own.
	const cell = (j, i) => (j < 0 ? false : j >= h ? true : grid[j][i]);
	for (let pass = 0; pass < 8; pass++) {
		let changed = false;
		for (let j = -1; j < h; j++) {
			let run = 0;
			let dir = 0;
			for (let i = 0; i <= w; i++) {
				const d = i < w && cell(j, i) !== cell(j + 1, i) ? (cell(j + 1, i) ? 1 : -1) : 0;
				if (d !== 0 && d === dir) {
					run++;
					continue;
				}
				if (run > maxFlat) {
					changed = true;
					for (let k = i - run + maxFlat - 1; k < i; k += maxFlat + 1) {
						for (const c of [k, k + 1].filter((c) => c < i)) {
							// Filled below: raise the top by a cell, or lower it at the
							// drawing's edge. Filled above: fill the cell below, or
							// trim the cell above at the bottom edge.
							if (dir === 1) {
								if (j >= 0) grid[j][c] = true;
								else grid[j + 1][c] = false;
							} else if (j + 1 < h) grid[j + 1][c] = true;
							else grid[j][c] = false;
						}
					}
				}
				run = d !== 0 ? 1 : 0;
				dir = d;
			}
		}
		if (!changed) break;
	}
	return grid;
}

/** Half-tone cells, like ░, as paths: two diagonal quarters of each cell. */
function checkerPath(grid, c) {
	const q = Math.floor(c / 2);
	let d = '';
	grid.forEach((row, j) =>
		row.forEach((on, i) => {
			if (on) d += `M${i * c} ${j * c}h${q}v${q}h${-q}zM${i * c + q} ${j * c + q}h${q}v${q}h${-q}z`;
		}),
	);
	return d;
}

/** A path repeated `copies` times side by side, `width` pixels apart. */
const repeated = (d, width, copies) => Array.from({ length: copies }, (_, k) => (k ? `<path transform="translate(${k * width} 0)" d="${d}"` : `<path d="${d}"`));

/**
 * An edge drawing as an inline SVG of plain paths, `copies` times side by
 * side, with crisp edges: no pattern and no image, so it stays sharp at any
 * pixel ratio and any position.
 */
function edgeMarkup(w, h, c, copies, parts) {
	const width = w * c;
	const body = parts
		.filter(([d]) => d)
		.flatMap(([d, fill]) => repeated(d, width, copies).map((p) => `${p} fill="${fill}"/>`))
		.join('');
	return `<svg xmlns="http://www.w3.org/2000/svg" width="${width * copies}" height="${h * c}" viewBox="0 0 ${width * copies} ${h * c}" shape-rendering="crispEdges" aria-hidden="true" focusable="false">${body}</svg>`;
}

/** A skyline as an SVG of solid cells, `copies` times side by side. */
export function edgeSvg(grid, c, color, copies = 1) {
	return edgeMarkup(grid[0].length, grid.length, c, copies, [[solidPath(grid, c), color]]);
}

/**
 * The underside of a ground hanging down into the next: the skyline turned
 * over, with a row of half-tone cells under each column's lowest cell and a
 * sparse row under that, so the ground thins out as it ends.
 */
export function fringeSvg(grid, c, color, copies = 1) {
	const hung = [...grid].reverse();
	const h = hung.length;
	const w = hung[0].length;
	const dense = Array.from({ length: h + 2 }, () => Array(w).fill(false));
	const sparse = Array.from({ length: h + 2 }, () => Array(w).fill(false));
	for (let i = 0; i < w; i++) {
		let low = -1;
		for (let j = 0; j < h; j++) if (hung[j][i]) low = j;
		if (low < 0) continue;
		dense[low + 1][i] = true;
		if (i % 2 === 0) sparse[low + 2][i] = true;
	}
	const q = Math.floor(c / 2);
	let dots = '';
	sparse.forEach((row, j) => row.forEach((on, i) => on && (dots += `M${i * c + Math.floor(q / 2)} ${j * c + Math.floor(q / 2)}h${q}v${q}h${-q}z`)));
	return edgeMarkup(w, h + 2, c, copies, [
		[solidPath(hung, c), color],
		[checkerPath(dense, c), color],
		[dots, color],
	]);
}

/** The clouds, as drawn by hand: `#` a full cell, `+` a half-tone cell. */
export const CLOUDS = {
	large: [
		'..................######..............',
		'.............#..##########............',
		'..........###################.........',
		'........#########################.....',
		'.....#############################....',
		'...################################...',
		'..##################################..',
		'.####################################.',
		'######################################',
		'.####################################.',
		'..++++++++++++++++++++++++++++++++++..',
		'.....++++++++++++++++++++++++++++.....',
	],
	small: [
		'......#####.........',
		'....#########.###...',
		'..################..',
		'.##################.',
		'####################',
		'.++++++++++++++++++.',
		'...++++++++++++++...',
	],
	wisp: ['....######....', '.############.', '##############', '.++++++++++++.'],
};

/** A cloud of `c`-pixel cells: body cells with a gap, a half-tone underside. */
export function cloudSvg(shape, c, body, shade) {
	const rows = CLOUDS[shape];
	const w = Math.max(...rows.map((r) => r.length));
	const grid = (ch) => rows.map((r) => [...r.padEnd(w, '.')].map((x) => x === ch));
	return svg(w, rows.length, c, [
		[solidPath(grid('#'), c), cellFill(c, body)],
		[solidPath(grid('+'), c), cellFill(c, shade, true)],
	]);
}

/** Pixel size of a cloud drawn with `c`-pixel cells. */
export function cloudSize(shape, c) {
	const rows = CLOUDS[shape];
	return { width: Math.max(...rows.map((r) => r.length)) * c, height: rows.length * c };
}

/** The letters of the wordmark, all on a five-cell x-height. */
export const LETTERS = {
	n: ['###.', '#..#', '#..#', '#..#', '#..#'],
	u: ['#..#', '#..#', '#..#', '#..#', '.###'],
	v: ['#...#', '#...#', '.#.#.', '.#.#.', '..#..'],
	e: ['.##.', '#..#', '####', '#...', '.###'],
	m: ['####.', '#.#.#', '#.#.#', '#.#.#', '#.#.#'],
};

/** "nuvem" in cells of `c` pixels, with a one-cell space between letters. */
export function wordmarkSvg(c, color) {
	const rows = Array.from({ length: 5 }, () => []);
	for (const [k, ch] of [...'nuvem'].entries()) {
		LETTERS[ch].forEach((r, y) => rows[y].push(...[...r].map((x) => x === '#'), ...(k < 4 ? [false] : [])));
	}
	return svg(rows[0].length, 5, c, [[solidPath(rows, c), cellFill(c, color)]]);
}

const MOON = ['..###', '.##..', '##...', '##...', '##...', '.##..', '..###'];

/** A crescent moon in cells. */
export function moonSvg(c, color) {
	return svg(5, 7, c, [[solidPath(MOON.map((r) => [...r].map((x) => x === '#')), c), cellFill(c, color)]]);
}

/** A star: one cell, or a plus of five. */
export function starSvg(kind, c, color) {
	const grid = kind === 'plus' ? ['.#.', '###', '.#.'] : ['#'];
	return svg(grid[0].length, grid.length, c, [[solidPath(grid.map((r) => [...r].map((x) => x === '#')), c), color]]);
}

/**
 * An SVG string as a CSS url(), for a background. Only the characters a
 * data URL cannot carry are escaped, which keeps the page small.
 */
export function cssUrl(svgText) {
	const body = svgText.replace(/"/g, "'").replace(/[%#<>{}]/g, (ch) => `%${ch.charCodeAt(0).toString(16).toUpperCase()}`);
	return `url("data:image/svg+xml,${body}")`;
}

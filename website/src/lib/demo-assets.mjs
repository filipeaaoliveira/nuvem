// The recordings and frames the home page shows. They live in the
// repository's `demo/` directory, produced by `scripts/render-demo.sh` and
// `scripts/capture-frame.sh`, and the site reads them in place: there are no
// copies under `website/`.
//
// The page imports the videos and the poster through Vite, which copies them
// into `dist/_astro/` with a content hash. A missing file would otherwise
// surface as an unresolved import deep in the build, so the integration
// below checks for every file first and names the command that makes it.

import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { REPO_ROOT } from './docs-source.mjs';

/** Repository-relative paths of every demo file the site uses. */
export const DEMO_FILES = {
	webm: 'demo/out/nuvem-demo.webm',
	mp4: 'demo/out/nuvem-demo.mp4',
	poster: 'demo/out/nuvem-demo-poster.png',
	frame: 'demo/frames/palette.html',
};

const MAKES = {
	'demo/out': 'scripts/render-demo.sh',
	'demo/frames': 'scripts/capture-frame.sh --all',
};

/** Absolute path of a demo file. */
export function demoPath(key) {
	return join(REPO_ROOT, DEMO_FILES[key]);
}

/** One message per missing demo file, each naming the script that makes it. */
export function missingDemoFiles() {
	return Object.values(DEMO_FILES)
		.filter((rel) => !existsSync(join(REPO_ROOT, rel)))
		.map((rel) => {
			const dir = Object.keys(MAKES).find((d) => rel.startsWith(`${d}/`));
			return `${rel} is missing. Run ${MAKES[dir]} from the repository root to make it.`;
		});
}

/**
 * Astro integration: fails the build when a demo file is missing, and lets
 * the dev server read files outside `website/`.
 */
export function demoAssetsIntegration() {
	return {
		name: 'nuvem-demo-assets',
		hooks: {
			'astro:config:setup': ({ command, updateConfig, logger }) => {
				const missing = missingDemoFiles();
				if (missing.length > 0) {
					const message = `The home page needs files from demo/ (${missing.length} missing):\n  ${missing.join('\n  ')}`;
					if (command === 'build') throw new Error(message);
					logger.warn(message);
				}
				updateConfig({ vite: { server: { fs: { allow: [REPO_ROOT] } } } });
			},
		},
	};
}


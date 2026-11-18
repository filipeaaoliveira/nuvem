import { defineCollection } from 'astro:content';
import { glob, type Loader } from 'astro/loaders';
import { docsSchema } from '@astrojs/starlight/schema';
import { resolve } from 'node:path';
import { DOCS_DIR, DOCS_GLOB, REPO_ROOT, idFor, titleFor } from './lib/docs-source.mjs';

/**
 * Loads the repository's `docs/` in place. A page without a frontmatter
 * `title` takes its first `# ` heading, so plain Markdown that reads well on
 * GitHub needs no site-specific header.
 */
function repoDocsLoader(): Loader {
	const inner = glob({
		pattern: DOCS_GLOB,
		base: DOCS_DIR,
		generateId: ({ entry, data }) => idFor(entry, data.slug as string | undefined),
	});
	return {
		name: 'repo-docs-loader',
		load: (context) =>
			inner.load({
				...context,
				parseData: (props) => {
					const data = { ...props.data };
					if (!data.title && props.filePath) {
						data.title = titleFor(resolve(REPO_ROOT, 'website', props.filePath));
					}
					return context.parseData({ ...props, data });
				},
			}),
	};
}

export const collections = {
	docs: defineCollection({ loader: repoDocsLoader(), schema: docsSchema() }),
};

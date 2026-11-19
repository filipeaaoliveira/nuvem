import { defineCollection } from 'astro:content';
import { glob, type Loader, type LoaderContext } from 'astro/loaders';
import { docsSchema } from '@astrojs/starlight/schema';
import { basename, dirname, resolve } from 'node:path';
import { CHANGELOG_FILE, CHANGELOG_ID, DOCS_DIR, DOCS_GLOB, REPO_ROOT, idFor, titleFor } from './lib/docs-source.mjs';

/**
 * Loads the repository's `docs/` and its `CHANGELOG.md` in place. A page without a frontmatter
 * `title` takes its first `# ` heading, so plain Markdown that reads well on
 * GitHub needs no site-specific header.
 */
function repoDocsLoader(): Loader {
	const docs = glob({
		pattern: DOCS_GLOB,
		base: DOCS_DIR,
		generateId: ({ entry, data }) => idFor(entry, data.slug as string | undefined),
	});
	// The repository's CHANGELOG.md, rendered in place as the last docs page.
	const changelog = glob({
		pattern: basename(CHANGELOG_FILE),
		base: dirname(CHANGELOG_FILE),
		generateId: () => CHANGELOG_ID,
	});
	return {
		name: 'repo-docs-loader',
		load: async (context) => {
			const parseData: LoaderContext['parseData'] = (props) => {
				const data = { ...props.data };
				if (!data.title && props.filePath) {
					data.title = titleFor(resolve(REPO_ROOT, 'website', props.filePath));
				}
				return context.parseData({ ...props, data });
			};
			// Each glob loader drops the store entries it did not load itself, so
			// each sees only its own ids.
			const own = (keep: (id: string) => boolean) =>
				new Proxy(context.store, {
					get(store, key) {
						if (key === 'keys') return () => [...store.keys()].filter(keep);
						const value = Reflect.get(store, key);
						return typeof value === 'function' ? value.bind(store) : value;
					},
				});
			await docs.load({ ...context, parseData, store: own((id) => id !== CHANGELOG_ID) });
			// Watch the one file, not the whole repository root it sits in.
			const watcher = context.watcher && new Proxy(context.watcher, {
				get(target, key) {
					if (key === 'add') return () => target.add(CHANGELOG_FILE);
					const value = Reflect.get(target, key);
					return typeof value === 'function' ? value.bind(target) : value;
				},
			});
			await changelog.load({ ...context, parseData, watcher, store: own((id) => id === CHANGELOG_ID) });
		},
	};
}

export const collections = {
	docs: defineCollection({ loader: repoDocsLoader(), schema: docsSchema() }),
};

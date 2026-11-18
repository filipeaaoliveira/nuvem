// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { REPO_URL, buildSidebar, firstPageUrl, hasDocsIndex } from './src/lib/docs-source.mjs';
import { satteri } from '@astrojs/markdown-satteri';
import docsMarkdown from './src/lib/markdown-docs.mjs';
import { pluginCodePlates } from './src/lib/code-plates.mjs';
import { docLinksIntegration } from './src/lib/doc-links.mjs';
import codeTheme from './src/styles/code-theme.json' with { type: 'json' };

const sidebar = buildSidebar();

export default defineConfig({
	site: 'https://nuvem.fyi',
	// Until the docs have an index page, /docs/ opens the first page.
	redirects: hasDocsIndex() ? {} : { '/docs': firstPageUrl(sidebar) },
	markdown: {
		processor: satteri({ mdastPlugins: [docsMarkdown] }),
	},
	integrations: [
		docLinksIntegration(),
		starlight({
			title: 'nuvem',
			description: 'A terminal UI for Google Cloud.',
			social: [{ icon: 'github', label: 'Source on GitHub', href: REPO_URL }],
			editLink: { baseUrl: `${REPO_URL}/edit/main/website/` },
			customCss: ['./src/styles/tokens.css', './src/styles/docs.css'],
			// Starlight's own Markdown features (asides, heading anchors) also
			// apply to the repository's docs, which live outside src/.
			markdown: { processedDirs: ['../docs'] },
			sidebar,
			routeMiddleware: './src/routeData.ts',
			components: {
				ThemeProvider: './src/components/ThemeProvider.astro',
				ThemeSelect: './src/components/ThemeSelect.astro',
				Search: './src/components/Search.astro',
			},
			expressiveCode: {
				themes: [codeTheme],
				plugins: [pluginCodePlates()],
				useStarlightDarkModeSwitch: false,
				useStarlightUiThemeColors: false,
				styleOverrides: {
					borderRadius: '2px',
					borderWidth: '1px',
					borderColor: 'var(--nv-rule)',
					codeFontFamily: 'var(--nv-font-mono)',
					codeFontSize: 'var(--nv-text-code)',
					codeLineHeight: '1.6',
					uiFontFamily: 'var(--nv-font-sans)',
					uiFontSize: 'var(--nv-text-small)',
					codeBackground: 'var(--nv-plate)',
					frames: {
						editorBackground: 'var(--nv-plate)',
						terminalBackground: 'var(--nv-plate)',
						editorTabBarBackground: 'var(--nv-plate)',
						editorActiveTabBackground: 'var(--nv-plate)',
						editorActiveTabForeground: 'var(--nv-text-muted)',
						editorActiveTabIndicatorTopColor: 'transparent',
						editorActiveTabIndicatorBottomColor: 'var(--nv-methyl)',
						editorTabBarBorderBottomColor: 'var(--nv-rule)',
						terminalTitlebarBackground: 'var(--nv-plate)',
						terminalTitlebarForeground: 'var(--nv-text-muted)',
						terminalTitlebarBorderBottomColor: 'var(--nv-rule)',
						terminalTitlebarDotsForeground: 'var(--nv-rule-strong)',
						terminalTitlebarDotsOpacity: '0',
						inlineButtonForeground: 'var(--nv-coral)',
						inlineButtonBackground: 'var(--nv-plate)',
						inlineButtonBackgroundIdleOpacity: '1',
						inlineButtonBackgroundHoverOrFocusOpacity: '1',
						inlineButtonBorder: 'var(--nv-rule-strong)',
						inlineButtonBorderOpacity: '1',
						tooltipSuccessBackground: 'var(--nv-naples)',
						tooltipSuccessForeground: 'var(--nv-plate)',
						shadowColor: 'transparent',
						frameBoxShadowCssValue: 'none',
					},
				},
			},
		}),
	],
});

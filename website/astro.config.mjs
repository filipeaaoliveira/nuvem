// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { REPO_ROOT, REPO_URL, buildSidebar, firstPageUrl, hasDocsIndex } from './src/lib/docs-source.mjs';
import { satteri } from '@astrojs/markdown-satteri';
import docsMarkdown from './src/lib/markdown-docs.mjs';
import { pluginCodePlates } from './src/lib/code-plates.mjs';
import { docLinksIntegration } from './src/lib/doc-links.mjs';
import { securityHeadersIntegration } from './src/lib/security-headers.mjs';
import { redirectsIntegration } from './src/lib/redirects.mjs';
import { demoAssetsIntegration } from './src/lib/demo-assets.mjs';
import codeTheme from './src/styles/code-theme.json' with { type: 'json' };

// Pages bundled for rendering read the repository root from here; see docs-source.mjs.
process.env.NUVEM_REPO_ROOT = REPO_ROOT;

const sidebar = buildSidebar();

export default defineConfig({
	site: 'https://nuvem.fyi',
	// Until the docs have an index page, /docs/ opens the first page. On
	// Cloudflare this is an HTTP redirect (src/lib/redirects.mjs).
	redirects: hasDocsIndex() ? {} : { '/docs': firstPageUrl(sidebar) },
	markdown: {
		processor: satteri({ mdastPlugins: [docsMarkdown] }),
	},
	integrations: [
		docLinksIntegration(),
		demoAssetsIntegration(),
		starlight({
			title: 'nuvem',
			description: 'A terminal UI for Google Cloud.',
			// The site title stays as text for screen readers, so the image is decorative.
			logo: { src: './src/assets/wordmark.svg', alt: '', replacesTitle: true },
			social: [{ icon: 'github', label: 'Source on GitHub', href: REPO_URL }],
			editLink: { baseUrl: `${REPO_URL}/edit/main/website/` },
			head: [
				{ tag: 'meta', attrs: { property: 'og:image', content: 'https://nuvem.fyi/og.png' } },
				{ tag: 'meta', attrs: { property: 'og:image:width', content: '1200' } },
				{ tag: 'meta', attrs: { property: 'og:image:height', content: '630' } },
				{ tag: 'meta', attrs: { property: 'og:image:alt', content: 'nuvem: a terminal UI for Google Cloud.' } },
			],
			customCss: ['./src/styles/tokens.css', './src/styles/docs.css', './src/styles/identity.css'],
			// Starlight's own Markdown features (asides, heading anchors) also
			// apply to the repository's docs, which live outside src/.
			markdown: { processedDirs: ['../docs'] },
			sidebar,
			routeMiddleware: './src/routeData.ts',
			// src/pages/404.astro replaces Starlight's stock not-found page.
			disable404Route: true,
			components: {
				Head: './src/components/Head.astro',
				ThemeProvider: './src/components/ThemeProvider.astro',
				ThemeSelect: './src/components/ThemeSelect.astro',
				Search: './src/components/Search.astro',
				Header: './src/components/Header.astro',
				Footer: './src/components/Footer.astro',
				MobileMenuFooter: './src/components/MobileMenuFooter.astro',
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
		securityHeadersIntegration(),
		redirectsIntegration(),
	],
});

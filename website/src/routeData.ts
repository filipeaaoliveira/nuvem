import { defineRouteMiddleware } from '@astrojs/starlight/route-data';

interface TocItem {
	children: TocItem[];
}

const count = (items: TocItem[]): number => items.reduce((n, item) => n + 1 + count(item.children), 0);

export const onRequest = defineRouteMiddleware((context) => {
	const route = context.locals.starlightRoute;

	// "On this page" with only the page title in it says nothing: drop it.
	if (route.toc && count(route.toc.items) < 2) route.toc = undefined;

	// The not-found page is served at every missing address, so it has no canonical URL.
	if (route.id === '404' || context.url.pathname.startsWith('/404')) {
		route.head = route.head.filter((entry) => !(entry.tag === 'link' && entry.attrs?.rel === 'canonical'));
	}
});

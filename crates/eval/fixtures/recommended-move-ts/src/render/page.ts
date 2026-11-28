import type { RenderPageOptions } from '../shared/types';

export function renderPage(options: RenderPageOptions): string {
  return `${pageTitle(options)}\n${pageRule(options)}\n${pageBody(options)}`;
}

export function pageTitle(options: RenderPageOptions): string {
  return options.title.toUpperCase();
}

export function pageRule(options: RenderPageOptions): string {
  return '-'.repeat(options.width);
}

export function pageBody(options: RenderPageOptions): string {
  return options.title.padEnd(options.width, '.');
}

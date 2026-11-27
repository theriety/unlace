import { renderPage } from './page';

export function layoutPage(title: string): string {
  return renderPage({ title, width: 80 });
}

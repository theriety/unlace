import { renderPage } from './page';

export function previewPage(title: string): string {
  return renderPage({ title, width: 40 });
}

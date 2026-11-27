import { formatNote, formatTitle } from './format';

export function pageTitle(page: number): string {
  return formatTitle(`Page ${page}`);
}

export function pageNote(page: number, total: number): string {
  return formatNote(`page ${page} of ${total}`);
}

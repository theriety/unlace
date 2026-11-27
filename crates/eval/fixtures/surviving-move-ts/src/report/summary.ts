import { formatNote, formatTitle } from './format';

export function summaryTitle(name: string): string {
  return formatTitle(`Summary of ${name}`);
}

export function summaryNote(count: number): string {
  return formatNote(`${count} entries`);
}

export function summaryFooter(name: string): string {
  return formatNote(`end of ${name}`);
}

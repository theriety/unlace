import { formatRow } from '../report/format';

export function renderRow(cells: string[]): string {
  return formatRow(cells);
}

export function renderRows(rows: string[][]): string {
  return rows.map((cells) => formatRow(cells)).join('\n');
}

export function renderTotals(totals: number[]): string {
  return formatRow(totals.map((total) => total.toFixed(2)));
}

export function renderAverages(sums: number[], count: number): string {
  return formatRow(sums.map((sum) => (sum / count).toFixed(2)));
}

export function renderEmptyRow(width: number): string {
  return formatRow(Array.from({ length: width }, () => ''));
}

export function renderSeparator(width: number): string {
  return formatRow(Array.from({ length: width }, () => '---'));
}

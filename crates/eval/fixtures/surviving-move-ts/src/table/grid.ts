import { formatRow } from '../report/format';
import { renderRows } from './row';

export function renderGrid(header: string[], rows: string[][]): string {
  return `${formatRow(header)}\n${renderRows(rows)}`;
}

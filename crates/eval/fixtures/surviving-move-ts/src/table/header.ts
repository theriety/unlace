import { renderRow } from './row';

export function renderHeader(columns: string[]): string {
  return renderRow(columns.map((column) => column.toUpperCase()));
}

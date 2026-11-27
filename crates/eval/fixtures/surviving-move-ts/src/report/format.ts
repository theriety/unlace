export function formatRow(cells: string[]): string {
  return cells.map((cell) => cell.trim()).join(' | ');
}

export function formatTitle(title: string): string {
  return `# ${title.trim()}`;
}

export function formatNote(note: string): string {
  return `> ${note.trim()}`;
}

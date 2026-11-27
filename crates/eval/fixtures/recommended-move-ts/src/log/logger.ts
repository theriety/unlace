import type { LogEntry } from '../shared/types';

export function formatEntry(entry: LogEntry): string {
  return `[${entry.level}] ${entry.message}`;
}

export function warn(message: string): LogEntry {
  return { level: 'warn', message };
}

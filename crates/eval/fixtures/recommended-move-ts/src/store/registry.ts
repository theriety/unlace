import type { EntityId } from '../shared/types';

export function register(ids: EntityId[], id: EntityId): EntityId[] {
  return [...ids, id];
}

export function known(ids: EntityId[], id: EntityId): boolean {
  return ids.includes(id);
}

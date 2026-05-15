import type {
  JsonSnapshot,
  ModelMessageSnapshot,
  ModelRequestSnapshotValue,
  ModelResponseSnapshotValue,
} from '../types/trace'

export function requestMessagesFromSnapshot(snapshot: JsonSnapshot | null): ModelMessageSnapshot[] {
  const value = snapshot?.value as ModelRequestSnapshotValue | null | undefined
  return Array.isArray(value?.messages) ? value.messages : []
}

export function responseMessageFromSnapshot(snapshot: JsonSnapshot | null): ModelMessageSnapshot[] {
  const value = snapshot?.value as ModelResponseSnapshotValue | null | undefined
  return value?.message ? [value.message] : []
}

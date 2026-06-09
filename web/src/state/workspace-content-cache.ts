export type WorkspaceContentCacheEntry = {
  value: string;
  bytes: number;
  lastAccessed: number;
};

export type WorkspaceContentCache = {
  maxBytes: number;
  totalBytes: number;
  entries: Record<string, WorkspaceContentCacheEntry>;
};

const DEFAULT_MAX_BYTES = 30 * 1024 * 1024;

export function emptyWorkspaceContentCache(maxBytes = DEFAULT_MAX_BYTES): WorkspaceContentCache {
  return { maxBytes, totalBytes: 0, entries: {} };
}

export function setWorkspaceContentCacheEntry(
  cache: WorkspaceContentCache,
  key: string,
  value: string,
  now = Date.now(),
): WorkspaceContentCache {
  const bytes = byteLength(value);
  const previous = cache.entries[key];
  const entries = {
    ...cache.entries,
    [key]: { value, bytes, lastAccessed: now },
  };
  const totalBytes = cache.totalBytes - (previous?.bytes ?? 0) + bytes;
  return trimCache({ ...cache, entries, totalBytes }, key);
}

export function getWorkspaceContentCacheValue(
  cache: WorkspaceContentCache,
  key: string,
  now = Date.now(),
): { value: string; cache: WorkspaceContentCache } | null {
  const entry = cache.entries[key];
  if (!entry) {
    return null;
  }
  return {
    value: entry.value,
    cache: {
      ...cache,
      entries: {
        ...cache.entries,
        [key]: { ...entry, lastAccessed: now },
      },
    },
  };
}

export function workspaceContentCacheValues(cache: WorkspaceContentCache): Record<string, string> {
  return Object.fromEntries(
    Object.entries(cache.entries).map(([key, entry]) => [key, entry.value]),
  );
}

function trimCache(cache: WorkspaceContentCache, protectedKey: string): WorkspaceContentCache {
  if (cache.totalBytes <= cache.maxBytes) {
    return cache;
  }
  const entries = { ...cache.entries };
  let totalBytes = cache.totalBytes;
  const evictionCandidates = Object.entries(entries)
    .filter(([key]) => key !== protectedKey)
    .sort((left, right) => left[1].lastAccessed - right[1].lastAccessed);

  for (const [key, entry] of evictionCandidates) {
    if (totalBytes <= cache.maxBytes) {
      break;
    }
    delete entries[key];
    totalBytes -= entry.bytes;
  }

  return { ...cache, entries, totalBytes };
}

function byteLength(value: string) {
  return new TextEncoder().encode(value).length;
}

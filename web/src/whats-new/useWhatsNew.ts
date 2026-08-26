import { useEffect, useState } from "react";
import { CURRENT_VERSION, recentEntries, type ChangelogEntry } from "./changelog";

export const WHATS_NEW_SEEN_KEY = "aria-whats-new-seen";

const STORAGE_UNAVAILABLE = Symbol("storage-unavailable");

function readSeenVersion(): string | null | typeof STORAGE_UNAVAILABLE {
  try {
    return window.localStorage.getItem(WHATS_NEW_SEEN_KEY);
  } catch {
    return STORAGE_UNAVAILABLE;
  }
}

function writeSeenVersion(version: string): void {
  try {
    window.localStorage.setItem(WHATS_NEW_SEEN_KEY, version);
  } catch {
    /* localStorage 不可用：静默降级 */
  }
}

export function useWhatsNew(): {
  open: boolean;
  entries: ChangelogEntry[];
  close: () => void;
} {
  const entries = recentEntries(CURRENT_VERSION);
  const currentEntry = entries[0] ?? null;
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!currentEntry) {
      return;
    }
    const seen = readSeenVersion();
    if (seen === STORAGE_UNAVAILABLE) {
      setOpen(false);
      return;
    }
    setOpen(seen !== CURRENT_VERSION);
  }, [currentEntry]);

  const close = () => {
    writeSeenVersion(CURRENT_VERSION);
    setOpen(false);
  };

  return { open, entries, close };
}

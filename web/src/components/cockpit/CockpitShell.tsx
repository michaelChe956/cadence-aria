import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { JSX } from "react";
import { useWorkspaceSessionObservers } from "../../hooks/useWorkspaceSessionObservers";
import { readCockpitSettings } from "../../state/cockpit-settings";
import type { CockpitInboxItem } from "../../state/workspace-cockpit-projection";
import { useWorkspaceStore } from "../../state/workspace-ws-store";

const GO_TO_INBOX_EVENT = "aria:cockpit:go-to-inbox";
const SYSTEM_NOTIFICATION_DELAY_MS = 30_000;

type NotificationGuidance = "permission-denied" | "permission-default" | null;

type CockpitShellContextValue = {
  inbox: readonly CockpitInboxItem[];
  pulseItemIds: ReadonlySet<string>;
  notificationGuidance: NotificationGuidance;
};

const CockpitShellContext = createContext<CockpitShellContextValue | null>(null);

function sessionIdFor(itemId: string): string {
  const separator = itemId.indexOf(":");
  return separator === -1 ? itemId : itemId.slice(0, separator);
}

function faviconHref(count: number): string {
  const badge = count > 0
    ? `<circle cx="24" cy="8" r="7" fill="#B42318"/><text x="24" y="11" text-anchor="middle" font-family="sans-serif" font-size="8" font-weight="700" fill="white">${count > 99 ? "99+" : count}</text>`
    : "";
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="8" fill="#FFF4EC"/><path d="M8 23 16 7l8 16h-4l-1.5-3.2h-5L12 23H8Zm7-7h2l-1-2.5L15 16Z" fill="#8E2D60"/>${badge}</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

export function useCockpitShellInbox(): readonly CockpitInboxItem[] {
  return useContext(CockpitShellContext)?.inbox ?? [];
}

export function useCockpitInboxPulse(itemId: string): boolean {
  return useContext(CockpitShellContext)?.pulseItemIds.has(itemId) ?? false;
}

export function useCockpitNotificationGuidance(): NotificationGuidance {
  return useContext(CockpitShellContext)?.notificationGuidance ?? null;
}
export function CockpitShell({ children }: { children: ReactNode }): JSX.Element {
  const [settings] = useState(() => readCockpitSettings());
  const currentSessionId = useWorkspaceStore((state) => state.sessionId);
  const currentSessionState = useWorkspaceStore();
  const [pulseItemIds, setPulseItemIds] = useState<ReadonlySet<string>>(() => new Set());
  const [toast, setToast] = useState<CockpitInboxItem | null>(null);
  const [notificationGuidance, setNotificationGuidance] = useState<NotificationGuidance>(null);
  const knownItemIdsRef = useRef(new Set<string>());
  const inboxBecameNonEmptyAtRef = useRef<number | null>(null);
  const previousFaviconHrefRef = useRef<string | null>(null);
  const initialTitleRef = useRef(document.title);
  const { inbox } = useWorkspaceSessionObservers({
    currentSessionId,
    currentSessionState,
    watchLimit: settings.watchLimit,
    refreshIntervalMs: settings.observerRefreshIntervalMs,
  });
  const itemIds = useMemo(() => new Set(inbox.map((item) => item.id)), [inbox]);
  const itemIdKey = useMemo(() => Array.from(itemIds).sort().join("|"), [itemIds]);

  useEffect(() => {
    if (currentSessionId === null) return;
    setPulseItemIds((previous) => {
      const next = new Set(
        Array.from(previous).filter((itemId) => sessionIdFor(itemId) !== currentSessionId),
      );
      return next.size === previous.size ? previous : next;
    });
  }, [currentSessionId]);

  useEffect(() => {
    const newlyOpened = inbox.filter((item) => !knownItemIdsRef.current.has(item.id));
    knownItemIdsRef.current = itemIds;
    setPulseItemIds((previous) => {
      const next = new Set(Array.from(previous).filter((itemId) => itemIds.has(itemId)));
      for (const item of newlyOpened) {
        next.add(item.id);
      }
      return next;
    });
    if (newlyOpened.length > 0) {
      setToast(newlyOpened[0]);
    }
  }, [inbox, itemIds]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 5_000);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    const count = inbox.length;
    if (settings.titleEmojiEnabled && count > 0) {
      document.title = `🔴待处理×${count} · aria`;
    } else {
      document.title = initialTitleRef.current;
    }
    return () => {
      document.title = initialTitleRef.current;
    };
  }, [inbox.length, settings.titleEmojiEnabled]);

  useEffect(() => {
    const icon = document.querySelector<HTMLLinkElement>('link[rel~="icon"]');
    if (!icon) return;
    if (previousFaviconHrefRef.current === null) {
      previousFaviconHrefRef.current = icon.href;
    }
    icon.href = inbox.length > 0 ? faviconHref(inbox.length) : previousFaviconHrefRef.current;
    return () => {
      if (previousFaviconHrefRef.current !== null) {
        icon.href = previousFaviconHrefRef.current;
      }
    };
  }, [inbox.length]);

  useEffect(() => {
    if (inbox.length === 0) {
      inboxBecameNonEmptyAtRef.current = null;
      return;
    }
    if (!settings.systemNotificationsEnabled || typeof Notification === "undefined") {
      return;
    }
    const now = Date.now();
    if (inboxBecameNonEmptyAtRef.current === null) {
      inboxBecameNonEmptyAtRef.current = now;
    }
    const remainingDelayMs = Math.max(
      0,
      SYSTEM_NOTIFICATION_DELAY_MS - (now - inboxBecameNonEmptyAtRef.current),
    );
    const timer = window.setTimeout(() => {
      if (Notification.permission === "granted") {
        new Notification("aria：需要处理", {
          body: `待处理 ${inbox.length} 项`,
          tag: "aria-cockpit-inbox",
        });
        setNotificationGuidance(null);
      } else if (Notification.permission === "denied") {
        setNotificationGuidance("permission-denied");
      } else {
        setNotificationGuidance("permission-default");
      }
    }, remainingDelayMs);
    return () => window.clearTimeout(timer);
  }, [inbox, settings.systemNotificationsEnabled]);

  const goToInbox = useCallback(() => {
    const firstItem = inbox[0];
    if (!firstItem) return;
    const sessionId = sessionIdFor(firstItem.id);
    setPulseItemIds((previous) =>
      new Set(
        Array.from(previous).filter((itemId) => sessionIdFor(itemId) !== sessionId),
      ),
    );
    window.dispatchEvent(
      new CustomEvent(GO_TO_INBOX_EVENT, { detail: { sessionId } }),
    );
  }, [inbox]);
  const contextValue = useMemo<CockpitShellContextValue>(
    () => ({ inbox, pulseItemIds, notificationGuidance }),
    [inbox, notificationGuidance, pulseItemIds],
  );

  return (
    <CockpitShellContext.Provider value={contextValue}>
      <div data-testid="cockpit-shell" className="min-h-screen bg-[var(--aria-bg)] text-[var(--aria-ink)]">
        {inbox.length > 0 ? (
          <aside
            role="alert"
            className="sticky top-0 z-[90] flex min-h-11 items-center justify-between gap-3 bg-[var(--aria-danger)] px-3 py-1 text-sm font-semibold text-white shadow-md"
          >
            <span>待处理 {inbox.length} 项</span>
            <button
              type="button"
              onClick={goToInbox}
              className="min-h-11 rounded-md bg-white px-3 text-sm font-semibold text-[var(--aria-danger)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--aria-danger)]"
            >
              去处理
            </button>
          </aside>
        ) : null}
        {toast ? (
          <div
            role="status"
            className="fixed right-4 top-16 z-[100] max-w-sm rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-4 py-3 text-sm font-semibold text-[var(--aria-ink)] shadow-lg"
          >
            需要处理：{toast.title}
          </div>
        ) : null}
        {children}
      </div>
    </CockpitShellContext.Provider>
  );
}

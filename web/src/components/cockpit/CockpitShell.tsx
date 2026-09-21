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
import { createPortal } from "react-dom";
import { Settings } from "lucide-react";
import {
  useWorkspaceSessionObservers,
  type WorkspaceSessionObserverResult,
} from "../../hooks/useWorkspaceSessionObservers";
import {
  readCockpitSettings,
  writeCockpitSettings,
  type CockpitSettings,
} from "../../state/cockpit-settings";
import type { CockpitInboxItem } from "../../state/workspace-cockpit-projection";
import { CockpitEscalation } from "./CockpitEscalation";
import { CockpitSettingsDialog } from "./CockpitSettingsDialog";
import { useWorkspaceStore } from "../../state/workspace-ws-store";
const SYSTEM_NOTIFICATION_DELAY_MS = 30_000;

type NotificationGuidance = "permission-denied" | "permission-default" | null;
type CockpitShellContextValue = {
  inbox: readonly CockpitInboxItem[];
  records: WorkspaceSessionObserverResult["records"];
  pulseItemIds: ReadonlySet<string>;
  notificationGuidance: NotificationGuidance;
  settings: CockpitSettings;
  watchSession(sessionId: string): void;
  /** 页面顶栏登记设置入口宿主节点；未登记时 shell 兜底浮层渲染入口。 */
  registerSettingsSlot(node: HTMLElement | null): void;
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

export function useCockpitObservedRecords(): WorkspaceSessionObserverResult["records"] {
  return useContext(CockpitShellContext)?.records ?? [];
}
export function useCockpitSessionWatch(): (sessionId: string) => void {
  return useContext(CockpitShellContext)?.watchSession ?? (() => undefined);
}

export function useCockpitInboxPulse(itemId: string): boolean {
  return useContext(CockpitShellContext)?.pulseItemIds.has(itemId) ?? false;
}

export function useCockpitNotificationGuidance(): NotificationGuidance {
  return useContext(CockpitShellContext)?.notificationGuidance ?? null;
}

export function useCockpitSettings(): CockpitSettings {
  return useContext(CockpitShellContext)?.settings ?? readCockpitSettings();
}

/**
 * 页面顶栏把「驾驶舱设置」入口挂到自己的布局里（如 cockpit 页头右侧），
 * 入口不再以 fixed 浮层压住 spec 抽屉等内容；登记后 shell 用 portal 注入宿主。
 */
export function useCockpitSettingsSlotRef(): (node: HTMLElement | null) => void {
  return useContext(CockpitShellContext)?.registerSettingsSlot ?? (() => undefined);
}
export function CockpitShell({
  children,
  onGoToInbox,
}: {
  children: ReactNode;
  onGoToInbox?: (sessionId: string) => void;
}): JSX.Element {
  const [settings, setSettings] = useState<CockpitSettings>(() => readCockpitSettings());
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsSlot, setSettingsSlot] = useState<HTMLElement | null>(null);
  const currentSessionId = useWorkspaceStore((state) => state.sessionId);
  const currentSessionState = useWorkspaceStore();
  const [pulseItemIds, setPulseItemIds] = useState<ReadonlySet<string>>(() => new Set());
  const [toast, setToast] = useState<CockpitInboxItem | null>(null);
  const [notificationGuidance, setNotificationGuidance] = useState<NotificationGuidance>(null);
  const knownItemIdsRef = useRef(new Set<string>());
  const inboxBecameNonEmptyAtRef = useRef<number | null>(null);
  const notificationSentRef = useRef(false);
  const previousFaviconHrefRef = useRef<string | null>(null);
  const initialTitleRef = useRef(document.title);
  const { inbox, countedInbox, records, watchSession } = useWorkspaceSessionObservers({
    currentSessionId,
    currentSessionState,
    watchLimit: settings.watchLimit,
    refreshIntervalMs: settings.observerRefreshIntervalMs,
  });
  const itemIds = useMemo(() => new Set(countedInbox.map((item) => item.id)), [countedInbox]);

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
    const newlyOpened = countedInbox.filter((item) => !knownItemIdsRef.current.has(item.id));
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
  }, [countedInbox, itemIds]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 5_000);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    const count = countedInbox.length;
    if (settings.titleEmojiEnabled && count > 0) {
      document.title = `🔴待处理×${count} · aria`;
    } else {
      document.title = initialTitleRef.current;
    }
    return () => {
      document.title = initialTitleRef.current;
    };
  }, [countedInbox.length, settings.titleEmojiEnabled]);

  useEffect(() => {
    const icon = document.querySelector<HTMLLinkElement>('link[rel~="icon"]');
    if (!icon) return;
    if (previousFaviconHrefRef.current === null) {
      previousFaviconHrefRef.current = icon.href;
    }
    icon.href = countedInbox.length > 0 ? faviconHref(countedInbox.length) : previousFaviconHrefRef.current;
    return () => {
      if (previousFaviconHrefRef.current !== null) {
        icon.href = previousFaviconHrefRef.current;
      }
    };
  }, [countedInbox.length]);

  useEffect(() => {
    if (countedInbox.length === 0) {
      inboxBecameNonEmptyAtRef.current = null;
      notificationSentRef.current = false;
      return;
    }
    if (
      notificationSentRef.current ||
      !settings.systemNotificationsEnabled ||
      typeof Notification === "undefined"
    ) {
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
          body: `待处理 ${countedInbox.length} 项`,
          tag: "aria-cockpit-inbox",
        });
        notificationSentRef.current = true;
        setNotificationGuidance(null);
      } else if (Notification.permission === "denied") {
        setNotificationGuidance("permission-denied");
      } else {
        setNotificationGuidance("permission-default");
      }
    }, remainingDelayMs);
    return () => window.clearTimeout(timer);
  }, [countedInbox, settings.systemNotificationsEnabled]);

  const goToInbox = useCallback(() => {
    const firstItem = countedInbox[0];
    if (!firstItem) return;
    const sessionId = sessionIdFor(firstItem.id);
    setPulseItemIds((previous) =>
      new Set(
        Array.from(previous).filter((itemId) => sessionIdFor(itemId) !== sessionId),
      ),
    );
    onGoToInbox?.(sessionId);
  }, [countedInbox, onGoToInbox]);
  const updateSettings = useCallback((nextSettings: CockpitSettings) => {
    writeCockpitSettings(nextSettings);
    setSettings(nextSettings);
  }, []);
  const contextValue = useMemo<CockpitShellContextValue>(
    () => ({
      inbox,
      records,
      pulseItemIds,
      notificationGuidance,
      settings,
      watchSession,
      registerSettingsSlot: setSettingsSlot,
    }),
    [inbox, notificationGuidance, pulseItemIds, records, settings, watchSession],
  );
  // 入口默认由 shell 兜底浮层渲染；页面登记顶栏宿主后改由 portal 注入宿主，
  // 避免 fixed 浮层压住 spec 抽屉（Artifact 审核/计划审批面板）。
  const settingsTrigger = (
    <button
      type="button"
      data-testid="cockpit-settings-trigger"
      aria-label="驾驶舱设置"
      onClick={() => setSettingsOpen(true)}
      className="inline-flex h-11 w-11 cursor-pointer items-center justify-center rounded-xl border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] shadow-md transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2"
    >
      <Settings aria-hidden="true" className="h-4 w-4" />
    </button>
  );

  return (
    <CockpitShellContext.Provider value={contextValue}>
      <div data-testid="cockpit-shell" className="min-h-screen bg-[var(--aria-bg)] text-[var(--aria-ink)]">
        {countedInbox.length > 0 ? (
          <aside
            role="alert"
            className="sticky top-0 z-[90] flex min-h-11 items-center justify-between gap-3 bg-[var(--aria-danger)] px-3 py-1 text-sm font-semibold text-white shadow-md"
          >
            <span>待处理 {countedInbox.length} 项</span>
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
            className="fixed right-4 top-28 z-[100] max-w-sm rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-4 py-3 text-sm font-semibold text-[var(--aria-ink)] shadow-lg"
          >
            需要处理：{toast.title}
          </div>
        ) : null}
        {settingsSlot === null ? (
          // 无页面宿主时的兜底入口：落在页面自身顶栏（约 44px）之下，且 z 序低于抽屉/
          // 浮层（lifecycle spec 抽屉 z-50、升级提醒 z-100、弹窗 z-110）——既不压页面
          // 顶栏控件，也不会挡住 spec 抽屉（UI-A）。
          <div data-testid="cockpit-settings-fallback" className="fixed right-4 top-16 z-40">
            {settingsTrigger}
          </div>
        ) : (
          createPortal(settingsTrigger, settingsSlot)
        )}
        <CockpitSettingsDialog
          open={settingsOpen}
          onClose={() => setSettingsOpen(false)}
          settings={settings}
          onChange={updateSettings}
        />
        <CockpitEscalation items={countedInbox} settings={settings} />
        {children}
      </div>
    </CockpitShellContext.Provider>
  );
}

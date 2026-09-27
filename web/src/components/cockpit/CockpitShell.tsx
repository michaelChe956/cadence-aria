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
import type { CodingAttempt } from "../../api/types";
import {
  readCockpitSettings,
  writeCockpitSettings,
  type CockpitSettings,
} from "../../state/cockpit-settings";
import type { CockpitInboxItem } from "../../state/workspace-cockpit-projection";
import { CockpitEscalation } from "./CockpitEscalation";
import { CockpitSettingsDialog } from "./CockpitSettingsDialog";
import { isRecentCompletionVisible } from "../../state/recent-completion";
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
  /**
   * P0 1.3（REQ-WIGA-05）Task 11：会话所属 issue 的当前活跃 coding attempt
   * 发现（lifecycle 目录轮询副产物）——驾驶舱按需拉 attempt snapshot，不常
   * 驻轮询、不要求打开 Coding Workspace。
   */
  codingAttemptForSession(sessionId: string): CodingAttempt | null;
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

export function useCockpitCodingAttemptForSession(): (
  sessionId: string,
) => CodingAttempt | null {
  return useContext(CockpitShellContext)?.codingAttemptForSession ?? (() => null);
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
  const [infoToast, setInfoToast] = useState<CockpitInboxItem | null>(null);
  // P3（REQ-WIGA-07）：候选驱动提示队列——每身份至多提醒一次；队列里
  // 等待显示的候选（gate toast 优先，info 逐条 5s）。
  const [infoQueue, setInfoQueue] = useState<readonly CockpitInboxItem[]>([]);
  const remindedInfoIdentitiesRef = useRef<Set<string>>(new Set());
  const knownInfoKeysRef = useRef<Set<string> | null>(null);
  const [notificationGuidance, setNotificationGuidance] = useState<NotificationGuidance>(null);
  const knownItemIdsRef = useRef(new Set<string>());
  const inboxBecameNonEmptyAtRef = useRef<number | null>(null);
  const notificationSentRef = useRef(false);
  const previousFaviconHrefRef = useRef<string | null>(null);
  const initialTitleRef = useRef(document.title);
  // P3（REQ-WIGA-07）：显式三分消费——展示收件箱 = displayItems，计数 =
  // actionableCount（info 恒不计数），提示 = notificationCandidates（每
  // 身份一次）；`inbox` 语义即 displayItems，countedInbox 保留为 actionable
  // 列表（去处理/系统通知）。
  const {
    displayItems,
    actionableCount,
    notificationCandidates,
    countedInbox,
    records,
    watchSession,
    codingAttemptForSession,
  } = useWorkspaceSessionObservers({
    currentSessionId,
    currentSessionState,
    watchLimit: settings.watchLimit,
    refreshIntervalMs: settings.observerRefreshIntervalMs,
  });
  const inbox = displayItems;
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

  // P1 WIGA Task 9（REQ-WIGA-07）legacy 静默基线：无 completionIdentity 的
  // plan/coding info（旧服务端 watched 投影）保持首非空批次只登记、新 key
  // 提醒一次；近期完成事实（有 completionIdentity）改走候选队列，不在此
  // 重复提示。
  useEffect(() => {
    const infoItems = inbox.filter(
      (item) => item.kind === "info" && item.completionIdentity === undefined,
    );
    if (knownInfoKeysRef.current === null) {
      if (infoItems.length === 0) {
        return;
      }
      knownInfoKeysRef.current = new Set(infoItems.map((item) => item.id));
      return;
    }
    const known = knownInfoKeysRef.current;
    const fresh = infoItems.filter((item) => !known.has(item.id));
    for (const item of infoItems) {
      known.add(item.id);
    }
    if (fresh.length > 0) {
      setInfoQueue((previous) => [...previous, fresh[0]]);
    }
  }, [inbox]);

  // P3（REQ-WIGA-07）：候选入队——每身份（completionIdentity）至多一次，
  // 按候选到达顺序排队；不取易跨 project 碰撞的 item.id。
  useEffect(() => {
    const queued = notificationCandidates.filter(
      (item) =>
        item.completionIdentity !== undefined &&
        !remindedInfoIdentitiesRef.current.has(item.completionIdentity),
    );
    if (queued.length === 0) {
      return;
    }
    setInfoQueue((previous) => [...previous, ...queued]);
  }, [notificationCandidates]);

  // P3（REQ-WIGA-07）：候选出队——gate toast 优先（info 留队待显）；队头
  // 已过 TTL 的直接丢弃不提醒；逐条显示 5s 后取下一条。
  useEffect(() => {
    if (infoToast !== null || toast !== null || infoQueue.length === 0) {
      return;
    }
    const nowMs = Date.now();
    // TTL 只约束近期完成事实；legacy info（无 completionIdentity）保持
    // P1 语义，不因 occurred_at 旧而丢提示。
    const nextVisibleIndex = infoQueue.findIndex(
      (item) =>
        item.completionIdentity === undefined ||
        isRecentCompletionVisible(item.createdAt ?? "", nowMs),
    );
    const expiredCount = nextVisibleIndex === -1 ? infoQueue.length : nextVisibleIndex;
    if (expiredCount > 0) {
      setInfoQueue((previous) => previous.slice(expiredCount));
      return;
    }
    const [head, ...rest] = infoQueue;
    setInfoQueue(rest);
    if (head.completionIdentity !== undefined) {
      remindedInfoIdentitiesRef.current.add(head.completionIdentity);
    }
    setInfoToast(head);
  }, [infoQueue, infoToast, toast]);

  useEffect(() => {
    if (!infoToast) return;
    const timer = window.setTimeout(() => setInfoToast(null), 5_000);
    return () => window.clearTimeout(timer);
  }, [infoToast]);

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 5_000);
    return () => window.clearTimeout(timer);
  }, [toast]);

  // P3（REQ-WIGA-07）：标题/favicon 计数语义等价替换——只随 actionableCount
  // （可操作 gate/choice/stopped/error/sc_failed）变化，info 恒不计数。
  useEffect(() => {
    if (settings.titleEmojiEnabled && actionableCount > 0) {
      document.title = `🔴待处理×${actionableCount} · aria`;
    } else {
      document.title = initialTitleRef.current;
    }
    return () => {
      document.title = initialTitleRef.current;
    };
  }, [actionableCount, settings.titleEmojiEnabled]);

  useEffect(() => {
    const icon = document.querySelector<HTMLLinkElement>('link[rel~="icon"]');
    if (!icon) return;
    if (previousFaviconHrefRef.current === null) {
      previousFaviconHrefRef.current = icon.href;
    }
    icon.href = actionableCount > 0 ? faviconHref(actionableCount) : previousFaviconHrefRef.current;
    return () => {
      if (previousFaviconHrefRef.current !== null) {
        icon.href = previousFaviconHrefRef.current;
      }
    };
  }, [actionableCount]);

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
      codingAttemptForSession,
      registerSettingsSlot: setSettingsSlot,
    }),
    [
      inbox,
      notificationGuidance,
      pulseItemIds,
      records,
      settings,
      watchSession,
      codingAttemptForSession,
    ],
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
      {/* v40 复验 #2：外壳独占视口高度（唯一 h-screen）。「待处理」告警条
          与页面在同一列内分配高度——此前告警条（52px）叠在 h-screen 页面
          之上，文档 scrollHeight=100vh+52px，主屏出现滚动条且页面底部
          （输入条/发送反馈按钮）被挤出视口。可增长页面（工作台/图片创作）
          改在内滚容器滚动，文档级不再滚动。 */}
      <div
        data-testid="cockpit-shell"
        className="flex h-screen flex-col overflow-hidden bg-[var(--aria-bg)] text-[var(--aria-ink)]"
      >
        {actionableCount > 0 ? (
          <aside
            role="alert"
            className="z-[90] flex min-h-11 shrink-0 items-center justify-between gap-3 bg-[var(--aria-danger)] px-3 py-1 text-sm font-semibold text-white shadow-md"
          >
            <span>待处理 {actionableCount} 项</span>
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
        {infoToast && !toast ? (
          <div
            role="status"
            className="fixed right-4 top-28 z-[100] max-w-sm rounded-md border border-emerald-300 bg-[var(--aria-panel)] px-4 py-3 text-sm font-semibold text-[var(--aria-ink)] shadow-lg"
          >
            进度信息：{infoToast.title}
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
        <div data-testid="cockpit-shell-scroll" className="min-h-0 flex-1 overflow-y-auto">
          {children}
        </div>
      </div>
    </CockpitShellContext.Provider>
  );
}

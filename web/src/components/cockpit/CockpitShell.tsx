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
import { useLocation, useRouter } from "@tanstack/react-router";
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
import type {
  CockpitInboxItem,
  CodingAttentionInboxProjection,
} from "../../state/workspace-cockpit-projection";
import type { CodingAttentionLocationItem } from "../../router";
import {
  inboxFocusTargetFromAttention,
  useWorkbenchInboxFocusOpener,
  type WorkbenchInboxFocusTarget,
} from "../../state/workbench-inbox-focus";
import { CockpitEscalation } from "./CockpitEscalation";
import { CockpitSettingsDialog } from "./CockpitSettingsDialog";
import { isRecentCompletionVisible } from "../../state/recent-completion";
import { useWorkspaceStore } from "../../state/workspace-ws-store";
const SYSTEM_NOTIFICATION_DELAY_MS = 30_000;

/**
 * 知识库详情页路由（沉浸模式判定）：
 * /workbench/projects/:projectId/logical-codebases/:lcId/knowledge-base
 * ——命中该路由时外壳只渲染页面自身，不渲染任何全局浮动件。
 */
const KNOWLEDGE_BASE_ROUTE_PATTERN =
  /^\/workbench\/projects\/[^/]+\/logical-codebases\/[^/]+\/knowledge-base\/?$/;

/** 当前路径是否命中知识库详情页路由（沉浸模式）。 */
export function isKnowledgeBaseRoute(pathname: string): boolean {
  return KNOWLEDGE_BASE_ROUTE_PATTERN.test(pathname);
}

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
  /**
   * P4-e（REQ-UI37-19）：A1 待处理 target 统一导航（shell prop 注入的镜像），
   * 收件箱「去处理」等 shell 子树消费；null = 宿主未接线，不出导航按钮。
   */
  openAttentionTarget: ((item: CodingAttentionLocationItem) => void) | null;
  /**
   * UI-3（待处理统一）：打开共享待处理抽屉并聚焦（/workbench
   * overlay=inbox + 定位字段）；shell 子树（收件箱行等）消费。
   * null = 无路由上下文且宿主未注入（单组件测试），消费方回落旧导航。
   */
  openInboxFocus: ((target: WorkbenchInboxFocusTarget) => void) | null;
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

// P4-e（REQ-UI37-19，契约 A1）：coding_action 事实投影 → 统一导航入参
//（router.tsx `useCodingAttentionTargetOpener` 消费），通知/收件箱「去处理」
// 共用，不各自拼地址。
export function codingAttentionLocationItem(
  info: CodingAttentionInboxProjection,
): CodingAttentionLocationItem {
  return {
    kind: info.attentionKind,
    project_id: info.projectId,
    issue_id: info.issueId,
    plan_id: info.planId ?? undefined,
    attempt_id: info.attemptId ?? undefined,
    logical_work_item_id: info.logicalWorkItemId ?? undefined,
    target: info.target,
    focus: info.focus ?? undefined,
  };
}

// P4-e（REQ-UI37-19）：coding_action 新 key 系统提醒——候选已由 observer 层
// 做 hydration 静默基线与同 key 去重，这里每身份至多发一次浏览器
// Notification（权限已授且设置开启才发）；UI-3（设计 §3 能力 6）起 onclick
// 只开共享待处理抽屉并聚焦（不执行确认/终止；coding_plan/workspace_session
// 目标回落统一导航）；权限拒绝/无 API 时平台消息（toast＋收件箱行）仍是退路。
function CodingAttentionNotifier({
  candidates,
  systemNotificationsEnabled,
  openTarget,
  openInboxFocus,
}: {
  candidates: readonly CockpitInboxItem[];
  systemNotificationsEnabled: boolean;
  openTarget: (item: CodingAttentionLocationItem) => void;
  openInboxFocus: ((target: WorkbenchInboxFocusTarget) => void) | null;
}) {
  const remindedIdentitiesRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    if (!systemNotificationsEnabled || typeof Notification === "undefined") {
      return;
    }
    for (const item of candidates) {
      if (item.kind !== "coding_action" || !item.attentionInfo) {
        continue;
      }
      const { identity } = item.attentionInfo;
      if (remindedIdentitiesRef.current.has(identity)) {
        continue;
      }
      remindedIdentitiesRef.current.add(identity);
      if (Notification.permission !== "granted") {
        continue;
      }
      const notification = new Notification(`aria：${item.title}`, {
        body: item.summary || item.attentionInfo.attentionKind,
        tag: `aria-coding-attention:${identity}`,
      });
      const attention = item.attentionInfo;
      const locationItem = codingAttentionLocationItem(attention);
      notification.onclick = () => {
        window.focus();
        const focusTarget =
          openInboxFocus !== null
            ? inboxFocusTargetFromAttention(attention)
            : null;
        if (focusTarget !== null && openInboxFocus !== null) {
          openInboxFocus(focusTarget);
          return;
        }
        openTarget(locationItem);
      };
    }
  }, [candidates, systemNotificationsEnabled, openTarget, openInboxFocus]);
  return null;
}

// P4-e（REQ-UI37-19）：告警条「去处理」的 coding 分流——全局按钮不能再把
// coding item.id 前缀（`coding_attention:…`）当 sessionId；UI-3 起与通知
// 同源：attempt 目标开共享待处理抽屉聚焦（openInboxFocus），其余回落
// 统一导航。
function CodingAttentionGoToButton({
  info,
  openTarget,
  openInboxFocus,
}: {
  info: CodingAttentionInboxProjection;
  openTarget: (item: CodingAttentionLocationItem) => void;
  openInboxFocus: ((target: WorkbenchInboxFocusTarget) => void) | null;
}) {
  const handleClick = () => {
    const focusTarget =
      openInboxFocus !== null ? inboxFocusTargetFromAttention(info) : null;
    if (focusTarget !== null && openInboxFocus !== null) {
      openInboxFocus(focusTarget);
      return;
    }
    openTarget(codingAttentionLocationItem(info));
  };
  return (
    <button
      type="button"
      data-testid="cockpit-attention-go-to"
      onClick={handleClick}
      className="min-h-11 rounded-md bg-white px-3 text-sm font-semibold text-[var(--aria-danger)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--aria-danger)]"
    >
      去处理
    </button>
  );
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
/**
 * P4-e（REQ-UI37-19）：shell 子树（如收件箱）读取 A1 待处理 target 统一
 * 导航；未接线（无 shell/宿主未注入）返回 null，消费方不出导航按钮。
 */
export function useCockpitAttentionTargetOpener(): ((
  item: CodingAttentionLocationItem,
) => void) | null {
  return useContext(CockpitShellContext)?.openAttentionTarget ?? null;
}

/**
 * UI-3（待处理统一）：shell 子树读取「打开共享待处理抽屉并聚焦」通道；
 * 未接线（无路由上下文且宿主未注入）返回 null，消费方回落旧导航。
 */
export function useCockpitInboxFocusOpener(): ((
  target: WorkbenchInboxFocusTarget,
) => void) | null {
  return useContext(CockpitShellContext)?.openInboxFocus ?? null;
}
type CockpitShellProps = {
  children: ReactNode;
  onGoToInbox?: (sessionId: string) => void;
  /**
   * P4-e（REQ-UI37-19）：coding_action 的统一导航（router 挂载点注入
   * `useCodingAttentionTargetOpener`）；缺席时 coding_action 的系统通知
   * onclick/告警条分流/收件箱「去处理」均不启用（平台消息不受影响）。
   * 组件自身不 import router 模块，避免 Shell↔router 循环加载。
   */
  onOpenAttentionTarget?: (item: CodingAttentionLocationItem) => void;
  /**
   * UI-3（待处理统一）：打开共享待处理抽屉并聚焦——显式注入优先（测试
   * 用）；缺省用内置实现（useWorkbenchInboxFocusOpener，仅依赖
   * @tanstack/react-router + 冻结 search 契约，不 import ./router，
   * 无路由上下文时为 null → 消费方回落旧导航）。
   */
  onOpenInboxFocus?: (target: WorkbenchInboxFocusTarget) => void;
};

/**
 * 外壳入口：路由感知（沉浸模式按当前路径判定）。
 * 无路由上下文（单组件测试）时按普通模式渲染——既有行为零变化。
 */
export function CockpitShell(props: CockpitShellProps): JSX.Element {
  const router = useRouter({ warn: false });
  if (!router) {
    return <CockpitShellContent {...props} pathname="" />;
  }
  return <CockpitShellRouted {...props} />;
}

/** 路由上下文内取当前路径（useLocation 订阅路由状态 ⇒ 路由切换即时生效）。 */
function CockpitShellRouted(props: CockpitShellProps): JSX.Element {
  const pathname = useLocation({ select: (location) => location.pathname });
  return <CockpitShellContent {...props} pathname={pathname} />;
}

function CockpitShellContent({
  children,
  onGoToInbox,
  onOpenAttentionTarget,
  onOpenInboxFocus,
  pathname,
}: CockpitShellProps & { pathname: string }): JSX.Element {
  // UI-3：共享待处理抽屉聚焦导航（通知/告警/收件箱行同源消费）。
  const routerOpenInboxFocus = useWorkbenchInboxFocusOpener();
  const openInboxFocus = onOpenInboxFocus ?? routerOpenInboxFocus;
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
  // P4-e（REQ-UI37-19）：告警条「去处理」的 coding 分流依据——首条待处理是
  // coding_action 且统一导航已接线（onOpenAttentionTarget 注入）时走统一
  // 导航，否则维持 sessionId 路径。
  const firstCountedItem = countedInbox[0];
  const firstCodingAttention =
    firstCountedItem?.kind === "coding_action" && firstCountedItem.attentionInfo
      ? firstCountedItem.attentionInfo
      : null;
  const hasCodingAttention = countedInbox.some((item) => item.kind === "coding_action");

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
      openAttentionTarget: onOpenAttentionTarget ?? null,
      openInboxFocus,
    }),
    [
      inbox,
      notificationGuidance,
      onOpenAttentionTarget,
      openInboxFocus,
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

  // 沉浸模式（知识库详情页路由）：只渲染页面自身——全局浮动件一律不渲染
  // （设置触发器/兜底浮层、设置对话框、升级提醒、待处理告警条与 toast）。
  // 外壳容器保留（仅承载视口高度与滚动链路），页面内部自行滚动。
  if (isKnowledgeBaseRoute(pathname)) {
    return (
      <CockpitShellContext.Provider value={contextValue}>
        <div
          data-testid="cockpit-shell"
          className="flex h-screen flex-col overflow-hidden bg-[var(--aria-bg)] text-[var(--aria-ink)]"
        >
          <div
            data-testid="cockpit-shell-scroll"
            className="min-h-0 flex-1 overflow-hidden"
          >
            {children}
          </div>
        </div>
      </CockpitShellContext.Provider>
    );
  }

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
            {firstCodingAttention && onOpenAttentionTarget ? (
              <CodingAttentionGoToButton
                info={firstCodingAttention}
                openTarget={onOpenAttentionTarget}
                openInboxFocus={openInboxFocus}
              />
            ) : (
              <button
                type="button"
                onClick={goToInbox}
                className="min-h-11 rounded-md bg-white px-3 text-sm font-semibold text-[var(--aria-danger)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--aria-danger)]"
              >
                去处理
              </button>
            )}
          </aside>
        ) : null}
        {toast ? (
          <div
            role="status"
            data-testid="cockpit-gate-toast"
            className="fixed right-4 top-28 z-[100] max-w-sm rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-4 py-3 text-sm font-semibold text-[var(--aria-ink)] shadow-lg"
          >
            需要处理：{toast.title}
            {/* UI-3：toast 只提醒+定位——coding_action 且可开抽屉时点击聚焦，
                不执行动作；其余保持纯展示。 */}
            {toast.kind === "coding_action" &&
            toast.attentionInfo &&
            openInboxFocus !== null ? (
              <button
                type="button"
                data-testid="cockpit-toast-open-inbox"
                onClick={() => {
                  const focusTarget = inboxFocusTargetFromAttention(
                    toast.attentionInfo!,
                  );
                  if (focusTarget !== null) {
                    openInboxFocus(focusTarget);
                  }
                }}
                className="ml-2 min-h-9 rounded-md border border-amber-300 bg-white px-2.5 py-1 text-xs font-semibold text-amber-900 transition-colors duration-200 hover:bg-amber-100"
              >
                去处理
              </button>
            ) : null}
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
        {hasCodingAttention && onOpenAttentionTarget ? (
          <CodingAttentionNotifier
            candidates={notificationCandidates}
            systemNotificationsEnabled={settings.systemNotificationsEnabled}
            openTarget={onOpenAttentionTarget}
            openInboxFocus={openInboxFocus}
          />
        ) : null}
        <div data-testid="cockpit-shell-scroll" className="min-h-0 flex-1 overflow-y-auto">
          {children}
        </div>
      </div>
    </CockpitShellContext.Provider>
  );
}

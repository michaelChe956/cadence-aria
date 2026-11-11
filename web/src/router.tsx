import {
  Navigate,
  Outlet,
  RouterProvider,
  createRootRoute,
  createRoute,
  createRouter,
  useNavigate,
  useLocation,
  useParams,
  useSearch,
  type RouterHistory,
} from "@tanstack/react-router";
import { useCallback, useEffect, useRef } from "react";
import { AppShell } from "./app-shell";
import { ProviderAvailabilityGuard } from "./components/providers/ProviderAvailabilityGuard";
import { CockpitShell } from "./components/cockpit/CockpitShell";
import { ChatWorkspacePage } from "./pages/ChatWorkspacePage";
import { CodingWorkspacePage } from "./pages/CodingWorkspacePage";
import { WorkbenchOverlayHost } from "./components/lifecycle/WorkbenchOverlayHost";
import type { CodingAttentionItemDto } from "./api/types";
import { ImageCreatePage } from "./pages/ImageCreatePage";
import { KnowledgeBasePage } from "./pages/KnowledgeBasePage";
import { LegacyCodingWorkspaceRedirect } from "./pages/LegacyCodingWorkspaceRedirect";
import { seedWorkbenchOverlayAddress } from "./state/workbench-overlay-store";
import {
  mergeWorkbenchSearch,
  parseWorkbenchSearch,
  searchParamString,
  type WorkbenchSearch,
} from "./workbench-search";

function RootRouteComponent() {
  const navigate = useNavigate();
  // P4-e（清单外机械补丁，IRC 已披露）：coding_action「去处理」/系统通知
  // onclick 的 A1 统一导航经 prop 注入（Shell 不 import router，避免循环加载）。
  const openAttentionTarget = useCodingAttentionTargetOpener();
  return (
    <CockpitShell
      onGoToInbox={(sessionId) =>
        void navigate({
          to: "/workbench/workspace/$sessionId",
          params: { sessionId },
        })
      }
      onOpenAttentionTarget={openAttentionTarget}
    >
      <Outlet />
    </CockpitShell>
  );
}

function GuardedWorkflowLayout() {
  return (
    <ProviderAvailabilityGuard>
      <Outlet />
    </ProviderAvailabilityGuard>
  );
}

const rootRoute = createRootRoute({ component: RootRouteComponent });

const guardedWorkflowRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: "guarded-workflow",
  component: GuardedWorkflowLayout,
});

const indexRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/",
  component: () => <Navigate to="/workbench" />,
});


function WorkbenchRouteComponent() {
  const search = useSearch({ strict: false }) as WorkbenchSearch;
  const navigate = useNavigate();
  // navigate 的身份随位置变化 ⇒ 直接把它放进 useCallback 依赖会让
  // syncDrawerFocus 每次导航都换身份 ⇒ 下游（工作台的抽屉焦点同步 effect）
  // 跟着重跑，并在 replace 里把刚发生的跨路由导航（如「查看知识库详细内容」
  // → 知识库详情页）**顶回 /workbench**。用 ref 固定 navigate，让两个回调
  // 身份稳定。
  const navigateRef = useRef(navigate);
  navigateRef.current = navigate;
  // 当前路径：抽屉焦点同步只在「当前就在工作台路由」时写地址。跨路由导航
  // 过程中工作台组件还会再渲染一帧（卸载前），那一帧若执行 replace
  // /workbench，会把刚发生的导航顶回去（真机复现：点「打开输入文件并补充」
  // 被顶回工作台）。
  const pathname = useLocation({ select: (location) => location.pathname });
  const pathnameRef = useRef(pathname);
  pathnameRef.current = pathname;
  // UI-1 冻结契约：/workbench 的任何 search 写入都必须合并现有 search
  // （mergeWorkbenchSearch）。此前 syncDrawerFocus 只写 {focus} 会覆盖掉
  // overlay 抽屉定位字段——本回调是必改点。
  const syncDrawerFocus = useCallback((entityKey: string | null) => {
    if (pathnameRef.current !== "/workbench") {
      return;
    }
    void navigateRef.current({
      to: "/workbench",
      search: (previous: Partial<WorkbenchSearch>) =>
        mergeWorkbenchSearch(previous, { focus: entityKey ?? undefined }),
      replace: true,
    });
  }, []);
  // overlay 抽屉（plan-overview/inbox）定位写入：同路由合并，可前进后退。
  const updateOverlaySearch = useCallback(
    (patch: Partial<WorkbenchSearch>) => {
      void navigate({
        to: "/workbench",
        search: (previous: Partial<WorkbenchSearch>) =>
          mergeWorkbenchSearch(previous, patch),
      });
    },
    [navigate],
  );
  return (
    <>
      <AppShell
        focusEntityKey={search.focus ?? null}
        onDrawerFocusChange={syncDrawerFocus}
        onOpenWorkspace={(sessionId) =>
          void navigate({
            to: "/workbench/workspace/$sessionId",
            params: { sessionId },
          })
        }
        onOpenCodingWorkspace={({ projectId, issueId, attemptId }) =>
          void navigate({
            to: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
            params: { projectId, issueId, attemptId },
          })
        }
      />
      <WorkbenchOverlayHost
        search={search}
        onSearchChange={updateOverlaySearch}
      />
    </>
  );
}

const workbenchRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/workbench",
  validateSearch: (search: Record<string, unknown>): WorkbenchSearch =>
    parseWorkbenchSearch(search),
  component: WorkbenchRouteComponent,
});

function WorkspaceRouteComponent() {
  const { sessionId } = useParams({ strict: false });
  const navigate = useNavigate();
  return (
    <ChatWorkspacePage
      sessionId={sessionId}
      onBack={() => void navigate({ to: "/workbench" })}
      onOpenSession={(parentSessionId) =>
        void navigate({
          to: "/workbench/workspace/$sessionId",
          params: { sessionId: parentSessionId },
        })
      }
      onOpenInfoCoding={({ projectId, issueId, attemptId }) =>
        void navigate({
          to: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
          params: { projectId, issueId, attemptId },
        })
      }
    />
  );
}

const workspaceRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/workbench/workspace/$sessionId",
  component: WorkspaceRouteComponent,
});

function CodingWorkspaceRouteComponent() {
  const { projectId, issueId, attemptId } = useParams({ strict: false });
  const search = useSearch({ strict: false }) as CodingWorkspaceFocusSearch;
  const navigate = useNavigate();
  // P1-c：planId 即 plan 上下文。原先它是「返回回总览」的唯一依据；现在只在
  // 无 A 类标记时才生效（见下方来源分流 B）。
  const planId = search.planId;
  // F-B（owner 复验残项：不再无脑落主页抽屉）：「返回」按来源分流——
  //   A 同页「全部 WI」弹窗下钻（from=plan-overview-modal + backAttemptId）
  //     → 回来源 attempt 页并自动重开总览弹窗（同页形态，URL 不跳 /workbench）；
  //   B 工作台总览抽屉来源（from=workbench-overlay，或缺省 from 的 planId
  //     上下文——既有 A1/深链入口）→ 仍回总览抽屉（UI-1 形态不回归）；
  //   C 无来源 → 回工作台主页，不带 overlay。
  const backAttemptId =
    search.from === "plan-overview-modal" ? search.backAttemptId : undefined;
  // UI-1：总览入口改开 /workbench 抽屉（overlay=plan-overview），「返回」的
  // 抽屉落点同样落到抽屉。跨路由进入不做 functional search 合并
  // （from 路由同名字段会串味），显式全新 search；导航前种入 plan 地址簿，
  // 抽屉开箱即解析。
  const openPlanOverview = (targetPlanId: string) => {
    seedWorkbenchOverlayAddress(targetPlanId, { projectId, issueId });
    void navigate({
      to: "/workbench",
      search: { overlay: "plan-overview", planId: targetPlanId },
    });
  };
  return (
    <CodingWorkspacePage
      address={{ projectId, issueId, attemptId }}
      focus={{
        attentionKind: search.focus,
        gateId: search.gateId,
        choiceId: search.choiceId,
      }}
      // A 类返回：回到来源 attempt 时只带「重开总览」请求——来源页本身的返回
      // 语义保持无来源（再按返回即回工作台主页），不继承本次下钻链。
      openOverviewRequested={search.openOverview === true}
      onOpenPlanOverview={openPlanOverview}
      onBack={() => {
        if (backAttemptId) {
          void navigate({
            to: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
            params: { projectId, issueId, attemptId: backAttemptId },
            search: { openOverview: true },
          });
          return;
        }
        if (planId) {
          openPlanOverview(planId);
          return;
        }
        void navigate({ to: "/workbench", search: {} });
      }}
    />
  );
}

const codingWorkspaceRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
  validateSearch: (
    search: Record<string, unknown>,
  ): CodingWorkspaceFocusSearch => ({
    planId: searchParamString(search.planId),
    workItemId: searchParamString(search.workItemId),
    focus: searchParamString(search.focus),
    gateId: searchParamString(search.gateId),
    choiceId: searchParamString(search.choiceId),
    from:
      search.from === "plan-overview-modal" ||
      search.from === "workbench-overlay"
        ? search.from
        : undefined,
    backAttemptId: searchParamString(search.backAttemptId),
    openOverview: search.openOverview === true ? true : undefined,
  }),
  component: CodingWorkspaceRouteComponent,
});


// UI-1（owner 拍板）：旧整页总览路由仅做 replace 重定向到规范化 workbench
// 地址并打开抽屉（移除整页渲染）；plan 地址簿先种入——刷新/分享旧链接同样
// 落到抽屉形态。
function CodingPlanOverviewRedirectRouteComponent() {
  const { projectId, issueId, planId } = useParams({ strict: false });
  const navigate = useNavigate();
  useEffect(() => {
    if (!projectId || !issueId || !planId) {
      return;
    }
    seedWorkbenchOverlayAddress(planId, { projectId, issueId });
    void navigate({
      to: "/workbench",
      search: { overlay: "plan-overview", planId },
      replace: true,
    });
  }, [navigate, planId, projectId, issueId]);
  return (
    <div
      data-testid="plan-overview-redirect"
      className="px-4 py-3 text-xs text-[var(--aria-ink-muted)]"
    >
      正在跳转到 Coding Plan 总览…
    </div>
  );
}

const codingPlanOverviewRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/workbench/projects/$projectId/issues/$issueId/coding-plans/$planId",
  component: CodingPlanOverviewRedirectRouteComponent,
});

// LC 知识库详情页（文件树 + 可写编辑器 + 预览 + 保存）。
// 挂在 guardedWorkflowRoute 子树下，与 codingWorkspaceRoute 同风格：路由参数
// 由 useParams 取出后透传给页面组件。
function KnowledgeBaseRouteComponent() {
  const { projectId, lcId } = useParams({ strict: false });
  return <KnowledgeBasePage projectId={projectId} lcId={lcId} />;
}

const knowledgeBaseRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/workbench/projects/$projectId/logical-codebases/$lcId/knowledge-base",
  component: KnowledgeBaseRouteComponent,
});

// ── P1-c（REQ-MTG-12）+ UI-1：A1 待办 target 统一导航 ──
// 契约 A1：attempt 走既有 coding 路由、plan 走 /workbench 总览抽屉
// （overlay=plan-overview）、plan 问题走 workspace/$sessionId；
// gateId/choiceId 焦点与 planId（返回总览上下文）经 search 传递。
// 块 4（P4-e）只回调本 helper，不重复实现导航。
export type CodingWorkspaceFocusSearch = {
  planId?: string;
  workItemId?: string;
  focus?: string;
  gateId?: string;
  choiceId?: string;
  /** F-B：「返回」来源标记——`"plan-overview-modal"` 表示本页由「全部 WI」
   * 同页弹窗下钻而来（A 类），配 backAttemptId 即回来源 attempt 页；
   * `"workbench-overlay"` 显式声明工作台总览抽屉来源（B 类）。两者都缺省时
   * 按 planId 判定 B/C：有 planId ⇒ 工作台总览抽屉（既有形态），无 ⇒ 主页。 */
  from?: CodingWorkspaceReturnSource;
  /** F-B：A 类下钻的来源 attempt id——「返回」回到该 attempt 并重开总览弹窗。 */
  backAttemptId?: string;
  /** F-B：`true` ⇒ 挂载/换址后自动打开本页总览弹窗（A 类返回时携带）。
   * 用布尔而非 "1"：TanStack 的 search 序列化会给「可 JSON 解析的字符串」
   * 加引号（`openOverview=%221%22`），布尔落成干净的 `openOverview=true`。 */
  openOverview?: boolean;
};

export type CodingWorkspaceReturnSource =
  | "plan-overview-modal"
  | "workbench-overlay";

export type CodingAttentionLocation = {
  to: string;
  params: Record<string, string>;
  search: Record<string, string | undefined>;
};

export type CodingAttentionLocationItem = Pick<
  CodingAttentionItemDto,
  | "kind"
  | "project_id"
  | "issue_id"
  | "plan_id"
  | "attempt_id"
  | "logical_work_item_id"
  | "target"
  | "focus"
>;

export function codingAttentionLocation(
  item: CodingAttentionLocationItem,
): CodingAttentionLocation | null {
  if (item.target.type === "coding_attempt" && item.target.attempt_id) {
    return {
      to: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
      params: {
        projectId: item.project_id,
        issueId: item.issue_id,
        attemptId: item.target.attempt_id,
      },
      search: {
        planId: item.plan_id ?? undefined,
        workItemId: item.logical_work_item_id ?? undefined,
        focus: item.kind === "final_confirm" ? "final_confirm" : undefined,
        gateId: item.focus?.gate_id,
        choiceId: item.focus?.choice_id,
      },
    };
  }
  if (item.target.type === "coding_plan" && item.target.plan_id) {
    // UI-1：plan 目标改开 /workbench 总览抽屉（与 Coding 页总览入口同一抽屉）。
    return {
      to: "/workbench",
      params: {},
      search: { overlay: "plan-overview", planId: item.target.plan_id },
    };
  }
  if (item.target.type === "workspace_session" && item.target.session_id) {
    return {
      to: "/workbench/workspace/$sessionId",
      params: { sessionId: item.target.session_id },
      search: {},
    };
  }
  return null;
}

type LooseNavigate = (location: CodingAttentionLocation) => void;

export function openCodingAttentionLocation(
  navigate: LooseNavigate,
  item: CodingAttentionLocationItem,
): boolean {
  const location = codingAttentionLocation(item);
  if (!location) {
    return false;
  }
  navigate(location);
  return true;
}

// P4-e（块 4）消费入口：通知/收件箱「去处理」经此统一导航，不各自拼地址。
export function useCodingAttentionTargetOpener() {
  const navigate = useNavigate();
  const location = useLocation();
  return useCallback((item: CodingAttentionLocationItem) => {
    openCodingAttentionLocation((targetLocation) => {
      if (targetLocation.to === "/workbench") {
        // UI-1：plan 目标开抽屉前种入地址簿（item 持有完整归一地址）。
        if (item.target.type === "coding_plan" && item.target.plan_id) {
          seedWorkbenchOverlayAddress(item.target.plan_id, {
            projectId: item.project_id,
            issueId: item.issue_id,
          });
        }
        const overlaySearch = targetLocation.search as Partial<WorkbenchSearch>;
        // 已在 /workbench 上时合并现有 search（保留 focus/抽屉定位）；跨路由
        // 进入时显式全新 search，避免 from 路由同名字段串味。
        void (
          navigate as unknown as (location: {
            to: string;
            params: Record<string, string>;
            search: unknown;
          }) => void
        )(
          location.pathname === "/workbench"
            ? {
                ...targetLocation,
                search: (previous: Partial<WorkbenchSearch>) =>
                  mergeWorkbenchSearch(previous, overlaySearch),
              }
            : { ...targetLocation, search: overlaySearch },
        );
        return;
      }
      void (navigate as unknown as LooseNavigate)({ ...targetLocation });
    }, item);
  }, [navigate, location.pathname]);
}

function LegacyCodingWorkspaceRouteComponent() {
  const { attemptId } = useParams({ strict: false });
  const navigate = useNavigate();
  return (
    <LegacyCodingWorkspaceRedirect
      attemptId={attemptId}
      onResolved={({ projectId, issueId, attemptId: resolvedAttemptId }) =>
        void navigate({
          to: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
          params: {
            projectId,
            issueId,
            attemptId: resolvedAttemptId,
          },
          replace: true,
        })
      }
      onBack={() => void navigate({ to: "/workbench" })}
    />
  );
}

const legacyCodingWorkspaceRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/workbench/coding/$attemptId",
  component: LegacyCodingWorkspaceRouteComponent,
});

const imageCreateRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/image-create",
  component: () => <ImageCreatePage />,
});

function ImageCreateSessionRouteComponent() {
  const { sessionId } = useParams({ from: "/image-create/$sessionId" });
  return <ImageCreatePage sessionId={sessionId} />;
}

const imageCreateSessionRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/image-create/$sessionId",
  component: ImageCreateSessionRouteComponent,
});

const guardedWorkflowTree = guardedWorkflowRoute.addChildren([
  indexRoute,
  workbenchRoute,
  workspaceRoute,
  codingWorkspaceRoute,
  codingPlanOverviewRoute,
  legacyCodingWorkspaceRoute,
  knowledgeBaseRoute,
]);

const routeTree = rootRoute.addChildren([
  guardedWorkflowTree,
  imageCreateRoute,
  imageCreateSessionRoute,
]);

export function createAppRouter(history?: RouterHistory) {
  return createRouter({
    routeTree,
    ...(history ? { history } : {}),
  });
}

export const router = createAppRouter();

export function AppRouter() {
  return <RouterProvider router={router} />;
}

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
import { useCallback, useEffect } from "react";
import { AppShell } from "./app-shell";
import { ProviderAvailabilityGuard } from "./components/providers/ProviderAvailabilityGuard";
import { CockpitShell } from "./components/cockpit/CockpitShell";
import { ChatWorkspacePage } from "./pages/ChatWorkspacePage";
import { CodingWorkspacePage } from "./pages/CodingWorkspacePage";
import { WorkbenchOverlayHost } from "./components/lifecycle/WorkbenchOverlayHost";
import type { CodingAttentionItemDto } from "./api/types";
import { ImageCreatePage } from "./pages/ImageCreatePage";
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
  // UI-1 冻结契约：/workbench 的任何 search 写入都必须合并现有 search
  // （mergeWorkbenchSearch）。此前 syncDrawerFocus 只写 {focus} 会覆盖掉
  // overlay 抽屉定位字段——本回调是必改点。
  const syncDrawerFocus = useCallback(
    (entityKey: string | null) => {
      void navigate({
        to: "/workbench",
        search: (previous: Partial<WorkbenchSearch>) =>
          mergeWorkbenchSearch(previous, { focus: entityKey ?? undefined }),
        replace: true,
      });
    },
    [navigate],
  );
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
  // P1-c：planId 存在即「从总览下钻」上下文——返回回总览，否则回 workbench。
  const planId = search.planId;
  // UI-1：总览入口改开 /workbench 抽屉（overlay=plan-overview），返回按钮
  // 的 planId 上下文同样落到抽屉。跨路由进入不做 functional search 合并
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
      onOpenPlanOverview={openPlanOverview}
      onBack={() =>
        planId
          ? openPlanOverview(planId)
          : void navigate({ to: "/workbench" })
      }
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
};

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

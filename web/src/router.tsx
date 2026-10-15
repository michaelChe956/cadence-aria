import {
  Navigate,
  Outlet,
  RouterProvider,
  createRootRoute,
  createRoute,
  createRouter,
  useNavigate,
  useParams,
  useSearch,
  type RouterHistory,
} from "@tanstack/react-router";
import { useCallback } from "react";
import { AppShell } from "./app-shell";
import { ProviderAvailabilityGuard } from "./components/providers/ProviderAvailabilityGuard";
import { CockpitShell } from "./components/cockpit/CockpitShell";
import { ChatWorkspacePage } from "./pages/ChatWorkspacePage";
import { CodingWorkspacePage } from "./pages/CodingWorkspacePage";
import { CodingPlanOverviewPage } from "./pages/CodingPlanOverviewPage";
import type { CodingAttentionItemDto } from "./api/types";
import { ImageCreatePage } from "./pages/ImageCreatePage";
import { LegacyCodingWorkspaceRedirect } from "./pages/LegacyCodingWorkspaceRedirect";

function RootRouteComponent() {
  const navigate = useNavigate();
  return (
    <CockpitShell
      onGoToInbox={(sessionId) =>
        void navigate({
          to: "/workbench/workspace/$sessionId",
          params: { sessionId },
        })
      }
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

type WorkbenchSearch = {
  focus?: string;
};

function WorkbenchRouteComponent() {
  const search = useSearch({ strict: false }) as WorkbenchSearch;
  const navigate = useNavigate();
  const syncDrawerFocus = useCallback(
    (entityKey: string | null) => {
      void navigate({
        to: "/workbench",
        search: { focus: entityKey ?? undefined },
        replace: true,
      });
    },
    [navigate],
  );
  return (
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
  );
}

const workbenchRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/workbench",
  validateSearch: (search: Record<string, unknown>): WorkbenchSearch => ({
    focus: typeof search.focus === "string" ? search.focus : undefined,
  }),
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
  const openPlanOverview = (targetPlanId: string) =>
    void navigate({
      to: "/workbench/projects/$projectId/issues/$issueId/coding-plans/$planId",
      params: { projectId, issueId, planId: targetPlanId },
    });
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

function searchParamString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function CodingPlanOverviewRouteComponent() {
  const { projectId, issueId, planId } = useParams({ strict: false });
  const navigate = useNavigate();
  return (
    <CodingPlanOverviewPage
      projectId={projectId}
      issueId={issueId}
      planId={planId}
      onBack={() => void navigate({ to: "/workbench" })}
      onOpenAttempt={(target) =>
        void navigate({
          to: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
          params: { projectId, issueId, attemptId: target.attemptId },
          search: {
            planId,
            workItemId: target.workItemId,
            gateId: target.gateId,
            choiceId: target.choiceId,
          },
        })
      }
    />
  );
}

const codingPlanOverviewRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/workbench/projects/$projectId/issues/$issueId/coding-plans/$planId",
  component: CodingPlanOverviewRouteComponent,
});

// ── P1-c（REQ-MTG-12）：plan 总览路由与 A1 待办 target 统一导航 ──
// 契约 A1：attempt 走既有 coding 路由、plan 走 coding-plans/$planId、plan
// 问题走 workspace/$sessionId；gateId/choiceId 焦点与 planId（返回总览上下文）
// 经 search 传递。块 4（P4-e）只回调本 helper，不重复实现导航。
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
    return {
      to: "/workbench/projects/$projectId/issues/$issueId/coding-plans/$planId",
      params: {
        projectId: item.project_id,
        issueId: item.issue_id,
        planId: item.target.plan_id,
      },
      search: {},
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
  return useCallback((item: CodingAttentionLocationItem) => {
    openCodingAttentionLocation(
      (location) =>
        void (navigate as unknown as LooseNavigate)({ ...location }),
      item,
    );
  }, [navigate]);
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

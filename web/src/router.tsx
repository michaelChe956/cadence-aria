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
import { ChatWorkspacePage } from "./pages/ChatWorkspacePage";
import { ChatRoomPage } from "./pages/ChatRoomPage";
import { CodingWorkspacePage } from "./pages/CodingWorkspacePage";
import { ImageCreatePage } from "./pages/ImageCreatePage";
import { LegacyCodingWorkspaceRedirect } from "./pages/LegacyCodingWorkspaceRedirect";

function RootRouteComponent() {
  return <Outlet />;
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
      onOpenGroupChat={(sessionId) =>
        void navigate({
          to: "/group-chat/$sessionId",
          params: { sessionId },
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
    />
  );
}

const workspaceRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/workbench/workspace/$sessionId",
  component: WorkspaceRouteComponent,
});

function ChatRoomRouteComponent() {
  const { sessionId } = useParams({ strict: false });
  const navigate = useNavigate();
  return (
    <ChatRoomPage
      sessionId={sessionId}
      onBack={() => void navigate({ to: "/workbench" })}
    />
  );
}

const chatRoomRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/group-chat/$sessionId",
  component: ChatRoomRouteComponent,
});

function CodingWorkspaceRouteComponent() {
  const { projectId, issueId, attemptId } = useParams({ strict: false });
  const navigate = useNavigate();
  return (
    <CodingWorkspacePage
      address={{ projectId, issueId, attemptId }}
      onBack={() => void navigate({ to: "/workbench" })}
    />
  );
}

const codingWorkspaceRoute = createRoute({
  getParentRoute: () => guardedWorkflowRoute,
  path: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
  component: CodingWorkspaceRouteComponent,
});

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
  chatRoomRoute,
  codingWorkspaceRoute,
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

import {
  Outlet,
  RouterProvider,
  createRootRoute,
  createRoute,
  createRouter,
  Navigate,
  useNavigate,
  useParams,
  useSearch,
} from "@tanstack/react-router";
import { useCallback } from "react";
import { AppShell } from "./app-shell";
import { ChatWorkspacePage } from "./pages/ChatWorkspacePage";
import { CodingWorkspacePage } from "./pages/CodingWorkspacePage";

const rootRoute = createRootRoute({ component: Outlet });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: () => <Navigate to="/workbench" />,
});

type WorkbenchSearch = {
  focus?: string;
};

function WorkbenchRouteComponent() {
  const search = useSearch({ from: "/workbench" });
  const navigate = useNavigate({ from: "/workbench" });
  const syncDrawerFocus = useCallback(
    (entityId: string | null) => {
      void navigate({
        search: (prev: WorkbenchSearch) => ({ ...prev, focus: entityId ?? undefined }),
        replace: true,
      });
    },
    [navigate],
  );
  return (
    <AppShell
      focusEntityId={search.focus ?? null}
      onDrawerFocusChange={syncDrawerFocus}
      onOpenWorkspace={(sessionId) =>
        void navigate({ to: "/workbench/workspace/$sessionId", params: { sessionId } })
      }
      onOpenCodingWorkspace={(attemptId) =>
        void navigate({ to: "/workbench/coding/$attemptId", params: { attemptId } })
      }
    />
  );
}

const workbenchRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workbench",
  validateSearch: (search: Record<string, unknown>): WorkbenchSearch => ({
    focus: typeof search.focus === "string" ? search.focus : undefined,
  }),
  component: WorkbenchRouteComponent,
});

function WorkspaceRouteComponent() {
  const { sessionId } = useParams({ from: "/workbench/workspace/$sessionId" });
  const navigate = useNavigate();
  return (
    <ChatWorkspacePage
      sessionId={sessionId}
      onBack={() => void navigate({ to: "/workbench" })}
    />
  );
}

const workspaceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workbench/workspace/$sessionId",
  component: WorkspaceRouteComponent,
});

function CodingWorkspaceRouteComponent() {
  const { attemptId } = useParams({ from: "/workbench/coding/$attemptId" });
  const navigate = useNavigate();
  return (
    <CodingWorkspacePage
      attemptId={attemptId}
      onBack={() => void navigate({ to: "/workbench" })}
    />
  );
}

const codingWorkspaceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workbench/coding/$attemptId",
  component: CodingWorkspaceRouteComponent,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  workbenchRoute,
  workspaceRoute,
  codingWorkspaceRoute,
]);

export const router = createRouter({ routeTree });

export function AppRouter() {
  return <RouterProvider router={router} />;
}

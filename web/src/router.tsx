import {
  Outlet,
  RouterProvider,
  createRootRoute,
  createRoute,
  createRouter,
  Navigate,
  useNavigate,
  useParams,
} from "@tanstack/react-router";
import { AppShell } from "./app-shell";
import { WorkspacePage } from "./pages/WorkspacePage";

const rootRoute = createRootRoute({ component: Outlet });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: () => <Navigate to="/workbench" />,
});

function WorkbenchRouteComponent() {
  const navigate = useNavigate();
  return (
    <AppShell
      onOpenWorkspace={(sessionId) =>
        void navigate({ to: "/workbench/workspace/$sessionId", params: { sessionId } })
      }
    />
  );
}

const workbenchRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workbench",
  component: WorkbenchRouteComponent,
});

function WorkspaceRouteComponent() {
  const { sessionId } = useParams({ from: "/workbench/workspace/$sessionId" });
  const navigate = useNavigate();
  return (
    <WorkspacePage
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

const routeTree = rootRoute.addChildren([indexRoute, workbenchRoute, workspaceRoute]);

export const router = createRouter({ routeTree });

export function AppRouter() {
  return <RouterProvider router={router} />;
}

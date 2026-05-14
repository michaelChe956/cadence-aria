import { RouterProvider, createRootRoute, createRouter } from "@tanstack/react-router";
import { AppShell } from "./app-shell";

const rootRoute = createRootRoute({ component: AppShell });
const routeTree = rootRoute;

export const router = createRouter({ routeTree });

export function AppRouter() {
  return <RouterProvider router={router} />;
}

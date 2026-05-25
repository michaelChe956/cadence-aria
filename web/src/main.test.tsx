import { createElement } from "react";
import type { ComponentType } from "react";
import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppShell } from "./main";

describe("AppShell", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("renders issue lifecycle as the default workbench", async () => {
    stubEmptyLifecycleFetch();

    render(<AppShell />);

    expect(
      await screen.findByRole("main", { name: "Issue 生命周期工作台" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Issue 卡片列表" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Issue 生命周期详情" })).toBeInTheDocument();
    expect(screen.queryByRole("main", { name: "Aria workbench" })).not.toBeInTheDocument();
  });

  it("does not open the removed legacy execution workbench from stale props", async () => {
    stubEmptyLifecycleFetch();

    render(
      createElement(AppShell as ComponentType<Record<string, unknown>>, {
        initialExecutionContext: {
          issueId: "issue_0001",
          workspaceId: "product:project_0001:repository_0001",
          taskId: "task_0001",
        },
      }),
    );

    expect(
      await screen.findByRole("main", { name: "Issue 生命周期工作台" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("main", { name: "Aria workbench" })).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Interaction window" })).not.toBeInTheDocument();
  });

  it("resets page scroll to the top when the workbench loads", () => {
    stubEmptyLifecycleFetch();
    const scrollTo = vi.fn();
    Object.defineProperty(window, "scrollTo", { configurable: true, value: scrollTo });
    Object.defineProperty(window.history, "scrollRestoration", {
      configurable: true,
      value: "auto",
      writable: true,
    });

    render(<AppShell />);

    expect(window.history.scrollRestoration).toBe("manual");
    expect(scrollTo).toHaveBeenCalledWith({ top: 0, left: 0, behavior: "auto" });
  });
});

function stubEmptyLifecycleFetch() {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === "/api/projects") return jsonResponse({ projects: [] });
      return jsonResponse({});
    }),
  );
}

function jsonResponse(body: unknown) {
  return Promise.resolve(new Response(JSON.stringify(body), { status: 200 }));
}

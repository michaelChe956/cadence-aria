// 批2 Task 1：控制面认证前端红绿测试。
// 覆盖：会话检查 204/401、token 仅 body 交换且不自动重发、AuthGate 认证闭环、
// 401 事件回认证态、三类 WS URL（workspace/coding/image-create）不加令牌。

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { listProjects, workspaceSessionWebSocketUrl } from "./client";
import { imageCreateChatWebSocketUrl } from "./image-create";
import { AuthGate, checkSession, exchangeBootstrapToken } from "./auth";
import { scopedCodingAttemptWebSocketUrl } from "../hooks/useCodingWorkspaceWs";

function jsonResponse(status: number, body: unknown = null): Response {
  if (body === null) {
    return new Response(null, { status });
  }
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function stubFetchSequence(responses: Array<() => Response>): {
  calls: Array<{ input: string; init?: RequestInit }>;
} {
  const calls: Array<{ input: string; init?: RequestInit }> = [];
  let index = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ input: String(input), init });
      const respond = responses[Math.min(index, responses.length - 1)];
      index += 1;
      return respond();
    }),
  );
  return { calls };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("api auth session client", () => {
  it("checkSession maps 204 to authenticated and 401 to unauthenticated", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse(204);
      }),
    );

    await expect(checkSession()).resolves.toBe(true);

    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse(401)),
    );
    await expect(checkSession()).resolves.toBe(false);

    expect(calls).toHaveLength(1);
    expect(calls[0].input).toBe("/api/auth/session");
    expect(calls[0].init?.method).toBeUndefined();
  });

  it("exchangeBootstrapToken posts token in body only, never in URL", async () => {
    const { calls } = stubFetchSequence([() => jsonResponse(204)]);
    const token = "a".repeat(64);

    await expect(exchangeBootstrapToken(token)).resolves.toBeUndefined();

    expect(calls).toHaveLength(1);
    expect(calls[0].input).toBe("/api/auth/session");
    expect(calls[0].init?.method).toBe("POST");
    expect(String(calls[0].init?.body)).toContain(token);
    expect(calls[0].input).not.toContain("token");
    expect(calls[0].input).not.toContain(token);
  });

  it("exchangeBootstrapToken rejects on 401 without auto-retry", async () => {
    const { calls } = stubFetchSequence([
      () =>
        jsonResponse(401, {
          code: "unauthorized",
          message: "bootstrap token invalid or already consumed",
          details: {},
        }),
    ]);

    await expect(exchangeBootstrapToken("bad".repeat(16))).rejects.toThrow(
      /invalid or already consumed/,
    );

    expect(calls).toHaveLength(1);
  });
});

describe("AuthGate", () => {
  it("shows bootstrap entry instead of children when unauthenticated", async () => {
    stubFetchSequence([() => jsonResponse(401)]);

    render(
      <AuthGate>
        <div>WORKBENCH-CHILDREN</div>
      </AuthGate>,
    );

    await waitFor(() =>
      expect(screen.getByLabelText(/一次性 Bootstrap Token/i)).toBeTruthy(),
    );
    expect(screen.queryByText("WORKBENCH-CHILDREN")).toBeNull();
  });

  it("renders children directly when session already authenticated", async () => {
    stubFetchSequence([() => jsonResponse(204)]);

    render(
      <AuthGate>
        <div>WORKBENCH-CHILDREN</div>
      </AuthGate>,
    );

    await waitFor(() => expect(screen.getByText("WORKBENCH-CHILDREN")).toBeTruthy());
    expect(screen.queryByLabelText(/一次性 Bootstrap Token/i)).toBeNull();
  });

  it("authenticates through token exchange exactly once, then renders children", async () => {
    const token = "b".repeat(64);
    const { calls } = stubFetchSequence([
      () => jsonResponse(401),
      () => jsonResponse(204),
    ]);

    render(
      <AuthGate>
        <div>WORKBENCH-CHILDREN</div>
      </AuthGate>,
    );

    const input = await waitFor(() =>
      screen.getByLabelText(/一次性 Bootstrap Token/i),
    );
    fireEvent.change(input, { target: { value: token } });
    fireEvent.click(screen.getByRole("button", { name: "认证" }));

    await waitFor(() => expect(screen.getByText("WORKBENCH-CHILDREN")).toBeTruthy());

    const posts = calls.filter((call) => call.init?.method === "POST");
    expect(posts).toHaveLength(1);
    expect(posts[0].input).toBe("/api/auth/session");
    expect(String(posts[0].init?.body)).toContain(token);
  });

  it("shows error and stays in auth entry when exchange is rejected", async () => {
    stubFetchSequence([
      () => jsonResponse(401),
      () =>
        jsonResponse(401, {
          code: "unauthorized",
          message: "bootstrap token invalid or already consumed",
          details: {},
        }),
    ]);

    render(
      <AuthGate>
        <div>WORKBENCH-CHILDREN</div>
      </AuthGate>,
    );

    const input = await waitFor(() =>
      screen.getByLabelText(/一次性 Bootstrap Token/i),
    );
    fireEvent.change(input, { target: { value: "c".repeat(64) } });
    fireEvent.click(screen.getByRole("button", { name: "认证" }));

    await waitFor(() => expect(screen.getByRole("alert")).toBeTruthy());
    expect(screen.getByRole("alert").textContent).toMatch(
      /invalid or already consumed/,
    );
    expect(screen.queryByText("WORKBENCH-CHILDREN")).toBeNull();
  });

  it("client 401 flips gate back to auth state without resending requests", async () => {
    const { calls } = stubFetchSequence([
      // 初始会话检查：已认证
      () => jsonResponse(204),
      // 随后的业务请求：401（服务重启后 cookie 失效）
      () =>
        jsonResponse(401, {
          code: "unauthorized",
          message: "authentication required",
          details: {},
        }),
    ]);

    render(
      <AuthGate>
        <div>WORKBENCH-CHILDREN</div>
      </AuthGate>,
    );
    await waitFor(() => expect(screen.getByText("WORKBENCH-CHILDREN")).toBeTruthy());

    await expect(listProjects()).rejects.toThrow(/authentication required/);

    await waitFor(() =>
      expect(screen.getByLabelText(/一次性 Bootstrap Token/i)).toBeTruthy(),
    );
    expect(screen.queryByText("WORKBENCH-CHILDREN")).toBeNull();

    // 失败请求不自动重发：/api/projects 仅被调用一次，且不存在任何 POST 重试。
    const projectCalls = calls.filter((call) => call.input.includes("/api/projects"));
    expect(projectCalls).toHaveLength(1);
    expect(calls.filter((call) => call.init?.method === "POST")).toHaveLength(0);
  });
});

describe("ws urls carry no token material", () => {
  it("workspace / coding / image-create WS URLs have no query or token", () => {
    const workspace = workspaceSessionWebSocketUrl("sess/1", {
      protocol: "https:",
      host: "aria.local:4317",
    });
    expect(workspace).toBe(
      "wss://aria.local:4317/api/workspace-sessions/sess%2F1/ws",
    );

    const coding = scopedCodingAttemptWebSocketUrl("p/1", "i/1", "a/1", {
      protocol: "http:",
      host: "aria.local:4317",
    });
    expect(coding).toBe(
      "ws://aria.local:4317/ws/projects/p%2F1/issues/i%2F1/coding-attempts/a%2F1",
    );

    const imageCreate = imageCreateChatWebSocketUrl("img/1", {
      protocol: "http:",
      host: "aria.local:4317",
    });
    expect(imageCreate).toBe(
      "ws://aria.local:4317/api/image-create/sessions/img%2F1/chat",
    );

    for (const url of [workspace, coding, imageCreate]) {
      expect(url).not.toContain("?");
      expect(url).not.toContain("token");
    }
  });
});

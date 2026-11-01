import { act } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  installWorkspaceWsTestHooks,
  renderWorkspaceHook,
} from "./useWorkspaceWs.test-utils";
import { useWorkspaceStore } from "../state/workspace-ws-store";

// REQ-DLS-02（driver-lease-self-healing Task 4）：前端写命令单次自动重试。
// ① STALE 后自动 hello+单次重放原命令成功无感；② 重放仍 STALE 回退手动
// 面不循环；③ 仅写命令触发（observer 拒绝不触发）。

function parseFrame(raw: string): Record<string, unknown> {
  return JSON.parse(raw) as Record<string, unknown>;
}

function staleDriverLease(received: string) {
  return {
    type: "protocol_error",
    code: "STALE_DRIVER_LEASE",
    message: `driver connection no longer holds the lease for write message ${received}`,
    context: { role: "driver", received },
  };
}

describe("useWorkspaceWs lease auto retry (REQ-DLS-02)", () => {
  installWorkspaceWsTestHooks();

  it("① STALE 后自动重发 driver hello 并单次重放原写命令，成功无感", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendConfirmGate();
    });
    expect(harness.ws.sent.map(parseFrame)).toEqual([{ type: "confirm" }]);

    harness.ws.sent.length = 0;
    act(() => {
      harness.ws.receive(staleDriverLease("confirm"));
    });

    expect(harness.ws.sent.map(parseFrame)).toEqual([
      {
        type: "hello",
        session_id: "session_001",
        last_seen_node_id: null,
        role: "driver",
      },
      { type: "confirm" },
    ]);
    const state = useWorkspaceStore.getState();
    expect(state.protocolError).toBeNull();
    expect(state.chatEntries.filter((entry) => entry.type === "error")).toEqual(
      [],
    );
  });

  it("② 重放仍 STALE：不再自动重试，回退既有手动接管面", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendHumanGateFeedback("验收命令缺失", "cmd_retry_once_1");
    });

    harness.ws.sent.length = 0;
    act(() => {
      harness.ws.receive(staleDriverLease("human_gate_feedback"));
    });
    expect(harness.ws.sent.map(parseFrame)).toEqual([
      {
        type: "hello",
        session_id: "session_001",
        last_seen_node_id: null,
        role: "driver",
      },
      {
        type: "human_gate_feedback",
        command_id: "cmd_retry_once_1",
        feedback: "验收命令缺失",
      },
    ]);

    act(() => {
      harness.ws.receive(staleDriverLease("human_gate_feedback"));
    });

    // 恰一次：重放被再次拒收后不再有新帧（无第二轮 hello/重放）。
    expect(harness.ws.sent).toHaveLength(2);
    const state = useWorkspaceStore.getState();
    expect(state.protocolError?.code).toBe("STALE_DRIVER_LEASE");
    expect(
      state.chatEntries.some(
        (entry) =>
          entry.type === "error" &&
          (entry.metadata as { code?: string } | undefined)?.code ===
            "STALE_DRIVER_LEASE",
      ),
    ).toBe(true);
  });

  it("③ 仅写命令触发自动重试：observer 拒绝不触发", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendAdvance("cmd_stale_contrast_1");
    });
    // 对照组：driver STALE 对写命令触发自动重试（与用例①同一前提）。
    harness.ws.sent.length = 0;
    act(() => {
      harness.ws.receive(staleDriverLease("advance"));
    });
    expect(harness.ws.sent.map(parseFrame)).toEqual([
      {
        type: "hello",
        session_id: "session_001",
        last_seen_node_id: null,
        role: "driver",
      },
      { type: "advance", command_id: "cmd_stale_contrast_1" },
    ]);

    // 观察者拒绝：同为写命令被拒，但不自动重试，走既有手动错误面。
    act(() => {
      harness.ws.sent.length = 0;
    });
    act(() => {
      harness.api.sendAdvance("cmd_observer_reject_1");
    });
    act(() => {
      harness.ws.receive({
        type: "protocol_error",
        code: "OBSERVER_WRITE_REJECTED",
        message: "observer connection cannot send write message advance",
        context: { role: "observer", received: "advance" },
      });
    });

    expect(harness.ws.sent.map(parseFrame)).toEqual([
      { type: "advance", command_id: "cmd_observer_reject_1" },
    ]);
    expect(useWorkspaceStore.getState().protocolError?.code).toBe(
      "OBSERVER_WRITE_REJECTED",
    );
  });
});

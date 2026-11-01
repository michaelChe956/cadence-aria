import { render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LeaseDiagnosticsSummary } from "../components/chat-workspace/LeaseDiagnosticsSummary";
import { useLeaseDiagnostics } from "./useLeaseDiagnostics";

// REQ-DLS-03 规范句「既有 STALE_DRIVER_LEASE 错误面 SHALL 引用最近相关转移事件」
// 的前端消费面：STALE 手动错误面激活时拉取只读诊断端点并渲染最近转移摘要；
// 端点失败静默降级为不展示（诊断不可用不阻塞恢复动作）。

function Probe({ sessionId, enabled }: { sessionId: string | null; enabled: boolean }) {
  const events = useLeaseDiagnostics(sessionId, enabled);
  return (
    <div>
      <p data-testid="stale-notice-sentinel">连接租约已失效</p>
      {events ? <LeaseDiagnosticsSummary events={events} /> : null}
    </div>
  );
}

// 服务端 lease_diagnostics_snapshot 形状（arbitration.rs lease_diagnostics_snapshot +
// lease_diagnostics.rs LeaseDiagnosticsRecord）：事件按打点时间序 append，最近在后。
function diagnosticsPayload() {
  return {
    session_id: "session_001",
    holder: "conn-thief",
    epoch: 4,
    last_holder: "conn-self",
    events: [
      {
        schema_version: 1,
        recorded_at: "2026-09-24T03:04:05.123456+00:00",
        workspace_session_id: "session_001",
        event: "hold",
        connection_id: "conn-older",
        role: "driver",
        reason: "hello",
        from_holder: null,
        to_holder: "conn-older",
        epoch: 1,
      },
      {
        schema_version: 1,
        recorded_at: "2026-09-24T03:05:06.123456+00:00",
        workspace_session_id: "session_001",
        event: "release",
        connection_id: "conn-self",
        role: "driver",
        reason: "connection_closed",
        from_holder: "conn-self",
        to_holder: null,
        epoch: 2,
      },
      {
        schema_version: 1,
        recorded_at: "2026-09-24T03:06:07.123456+00:00",
        workspace_session_id: "session_001",
        event: "hold",
        connection_id: "conn-thief",
        role: "driver",
        reason: "hello",
        from_holder: null,
        to_holder: "conn-thief",
        epoch: 3,
      },
      {
        schema_version: 1,
        recorded_at: "2026-09-24T03:07:08.123456+00:00",
        workspace_session_id: "session_001",
        event: "write_rejected_stale",
        connection_id: "conn-self",
        role: "driver",
        reason: "lease_mismatch",
        from_holder: "conn-thief",
        to_holder: "conn-thief",
        epoch: 4,
      },
    ],
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useLeaseDiagnostics", () => {
  it("fetches the diagnostics endpoint when the stale surface activates and renders the recent transfer summary", async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(diagnosticsPayload()), {
      status: 200,
    }));
    vi.stubGlobal("fetch", fetchMock);

    // 未激活（无 STALE 手动错误面）不拉取——诊断引用只在错误面渲染时需要。
    const { rerender } = render(
      <Probe sessionId="session_001" enabled={false} />,
    );
    expect(fetchMock).not.toHaveBeenCalled();
    expect(screen.queryByTestId("lease-diagnostics-summary")).toBeNull();

    rerender(<Probe sessionId="session_001" enabled={true} />);

    await waitFor(() => {
      expect(screen.getByTestId("lease-diagnostics-summary")).toBeInTheDocument();
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/workspace-sessions/session_001/lease-diagnostics",
      expect.anything(),
    );

    const summary = screen.getByTestId("lease-diagnostics-summary");
    expect(summary).toHaveTextContent("最近租约转移");
    const lines = within(summary).getAllByTestId("lease-diagnostics-event");
    // 最近 1-3 条封顶：4 条事件只保留最后 3 条（conn-older 滚出），
    // 时间序保持服务端 append 序——最近（含偷窃者 hold 与本次写拒）在后。
    expect(lines).toHaveLength(3);
    expect(lines[0].textContent).toMatch(/^\d{2}:\d{2}:\d{2} · 释放 · conn-self$/);
    expect(lines[1].textContent).toMatch(/^\d{2}:\d{2}:\d{2} · 持有 · conn-thief$/);
    expect(lines[2].textContent).toMatch(
      /^\d{2}:\d{2}:\d{2} · 写被拒（租约易主） · conn-self$/,
    );
    expect(summary).not.toHaveTextContent("conn-older");
  });

  it("degrades silently when the endpoint fails: no summary and the error surface stays unblocked", async () => {
    // 网络失败与 HTTP 500 两种失败形态都静默降级。
    const networkFailure = vi.fn(async () => {
      throw new TypeError("network down");
    });
    vi.stubGlobal("fetch", networkFailure);

    const { unmount } = render(<Probe sessionId="session_001" enabled={true} />);
    await waitFor(() => {
      expect(networkFailure).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByTestId("stale-notice-sentinel")).toBeInTheDocument();
    expect(screen.queryByTestId("lease-diagnostics-summary")).toBeNull();
    unmount();

    const serverFailure = vi.fn(
      async () => new Response(JSON.stringify({ code: "internal_error" }), { status: 500 }),
    );
    vi.stubGlobal("fetch", serverFailure);
    render(<Probe sessionId="session_001" enabled={true} />);
    await waitFor(() => {
      expect(serverFailure).toHaveBeenCalledTimes(1);
    });
    expect(screen.getByTestId("stale-notice-sentinel")).toBeInTheDocument();
    expect(screen.queryByTestId("lease-diagnostics-summary")).toBeNull();
  });
});

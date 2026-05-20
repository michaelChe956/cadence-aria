import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  DisconnectBanner,
  loadAcknowledgedAbortedNodes,
  saveAcknowledgedAbortedNode,
} from "./DisconnectBanner";

describe("DisconnectBanner", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("shows reconnect progress after multiple attempts", () => {
    const onManualReconnect = vi.fn();

    render(
      <DisconnectBanner
        isReconnecting={true}
        attemptCount={2}
        onManualReconnect={onManualReconnect}
      />,
    );

    expect(screen.getByText(/重连中/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "手动重连" }));
    expect(onManualReconnect).toHaveBeenCalled();
  });

  it("does not show reconnect progress for the first attempt", () => {
    const { container } = render(<DisconnectBanner isReconnecting={true} attemptCount={1} />);

    expect(container).toBeEmptyDOMElement();
  });

  it("shows aborted banner and acknowledges the aborted node", () => {
    const onAcknowledge = vi.fn();

    render(
      <DisconnectBanner
        abortedByDisconnect={{ nodeId: "node-aborted-1", ts: "2026-05-20T14:32:00Z" }}
        onAcknowledge={onAcknowledge}
      />,
    );

    expect(screen.getByText(/上次运行因断开被中止/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "我知道了" }));

    expect(onAcknowledge).toHaveBeenCalledWith(["node-aborted-1"]);
    expect(loadAcknowledgedAbortedNodes()).toEqual(["node-aborted-1"]);
  });

  it("deduplicates acknowledged aborted nodes in localStorage", () => {
    saveAcknowledgedAbortedNode("node-aborted-1");
    const result = saveAcknowledgedAbortedNode("node-aborted-1");

    expect(result).toEqual(["node-aborted-1"]);
    expect(window.localStorage.getItem("aria.workspace.aborted_ack_nodes")).toBe(
      JSON.stringify(["node-aborted-1"]),
    );
  });
});

// web/src/components/coding-workspace/dashboard/CodingDependencyChain.test.tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { CodingDependencyChain } from "../../../state/coding-dashboard-projection";
import { CodingDependencyChainView } from "./CodingDependencyChain";

function node(workItemId: string, state: CodingDependencyChain["upstream"][number]["state"], title = workItemId) {
  return { workItemId, unitId: `u_${workItemId}`, title, orderIndex: 0, state, completionCommit: null };
}

const chain: CodingDependencyChain = {
  workItemId: "wi_c",
  upstream: [node("wi_b", "failed", "契约单元")],
  downstream: [node("wi_d", "pending", "消费单元")],
  blockingSources: [node("wi_b", "failed", "契约单元")],
};

describe("CodingDependencyChainView", () => {
  it("pinpoints blocking sources and lists upstream providers and downstream consumers", () => {
    render(<CodingDependencyChainView chain={chain} onSelectWorkItem={() => undefined} />);
    expect(screen.getByTestId("coding-chain-blocking")).toHaveTextContent("为什么被挡");
    expect(screen.getByTestId("coding-chain-blocking")).toHaveTextContent("契约单元");
    expect(screen.getByTestId("coding-chain-upstream")).toHaveTextContent("契约单元");
    expect(screen.getByTestId("coding-chain-downstream")).toHaveTextContent("消费单元");
  });

  it("moves the chain focus when a listed unit is clicked", async () => {
    const user = userEvent.setup();
    const onSelectWorkItem = vi.fn();
    render(<CodingDependencyChainView chain={chain} onSelectWorkItem={onSelectWorkItem} />);
    await user.click(screen.getByRole("button", { name: /契约单元/ }));
    expect(onSelectWorkItem).toHaveBeenCalledWith("wi_b");
  });

  it("renders explicit empty copy when a unit has no neighbours and no blocker", () => {
    render(
      <CodingDependencyChainView
        chain={{ workItemId: "wi_solo", upstream: [], downstream: [], blockingSources: [] }}
        onSelectWorkItem={() => undefined}
      />,
    );
    expect(screen.queryByTestId("coding-chain-blocking")).toBeNull();
    expect(screen.getByTestId("coding-chain-no-upstream")).toHaveTextContent("无上游依赖");
    expect(screen.getByTestId("coding-chain-no-downstream")).toHaveTextContent("无下游消费");
  });
});

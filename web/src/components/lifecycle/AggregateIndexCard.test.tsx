import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AggregateIndexActiveResponse } from "../../api/types";
import { AggregateIndexCard } from "./AggregateIndexCard";

function renderCard(
  index: AggregateIndexActiveResponse,
  rebuilding = false,
  onRebuild = vi.fn(),
) {
  render(
    <AggregateIndexCard
      index={index}
      rebuilding={rebuilding}
      onRebuild={onRebuild}
    />,
  );
  return onRebuild;
}

describe("AggregateIndexCard", () => {
  it.each([
    ["active", "索引可用"],
    ["stale", "索引已过期"],
    ["degraded", "索引降级"],
    ["rebuilding", "正在重建索引"],
    ["missing", "尚未建立索引"],
  ] as const)("renders %s state with its fixed label", (state, label) => {
    renderCard({ state, revision: 7, indexed_at: "2026-08-18T00:00:00Z" });

    expect(screen.getByTestId("aggregate-index-status")).toHaveTextContent(
      state,
    );
    expect(screen.getByText(label)).toBeInTheDocument();
  });

  it("renders degraded warning and disables rebuild while synchronous rebuild is pending", async () => {
    const onRebuild = vi.fn();
    render(<AggregateIndexCard index={{ state: "degraded", revision: 7, indexed_at: "2026-08-18T00:00:00Z", warning: "sync failed" }} rebuilding onRebuild={onRebuild} />);
    expect(screen.getByTestId("aggregate-index-status")).toHaveTextContent("degraded");
    expect(screen.getByText("sync failed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重建索引" })).toBeDisabled();
    expect(screen.getByTestId("aggregate-index-spinner")).toBeInTheDocument();
  });

  it("keeps rebuild available when the aggregate index is missing", () => {
    const onRebuild = renderCard({ state: "missing" });

    fireEvent.click(screen.getByRole("button", { name: "重建索引" }));

    expect(onRebuild).toHaveBeenCalledOnce();
  });

  it("shows an actionable stale notice", () => {
    renderCard({
      state: "stale",
      revision: 6,
      indexed_at: "2026-08-18T00:00:00Z",
    });

    expect(screen.getByTestId("aggregate-index-stale-notice")).toHaveTextContent(
      "索引已过期，请重建后再使用聚合上下文。",
    );
  });
});

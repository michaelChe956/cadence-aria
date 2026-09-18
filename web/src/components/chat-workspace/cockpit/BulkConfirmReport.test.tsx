import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { BulkConfirmReport } from "./BulkConfirmReport";
import { useBulkConfirmStore } from "../../../state/bulk-confirm-store";

describe("BulkConfirmReport", () => {
  beforeEach(() => useBulkConfirmStore.getState().reset());

  it("如实标注并发上限与逐条结果（成功/失败清单）", () => {
    useBulkConfirmStore.setState({
      busy: false,
      runs: [{
        id: "run1",
        startedAtIso: "2026-09-18T00:00:00Z",
        concurrency: 3,
        items: [
          {
            itemId: "s2:gate:g2",
            sessionId: "s2",
            gateKey: "g2",
            title: "方案定稿",
            status: "confirmed",
            detail: null,
            atIso: "2026-09-18T00:00:01Z",
          },
          {
            itemId: "s3:gate:g3",
            sessionId: "s3",
            gateKey: "g3",
            title: "计划确认",
            status: "rejected",
            detail: "INVALID_MESSAGE_FOR_STAGE",
            atIso: "2026-09-18T00:00:01Z",
          },
        ],
      }],
    });

    render(<BulkConfirmReport />);

    const report = screen.getByTestId("bulk-confirm-report");
    expect(report).toHaveTextContent("并发上限 3");
    expect(report).toHaveTextContent("成功 1");
    expect(report).toHaveTextContent("失败 1");
    expect(report).toHaveTextContent("INVALID_MESSAGE_FOR_STAGE");
    expect(report).toHaveTextContent("每个所选会话使用一条独立短连接");
    expect(report).toHaveTextContent("失败不自动重试");
  });

  it("无 run 时不渲染", () => {
    render(<BulkConfirmReport />);

    expect(screen.queryByTestId("bulk-confirm-report")).toBeNull();
  });
});

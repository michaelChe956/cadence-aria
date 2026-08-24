// Task 8（展示组件 + 接线）：逻辑代码库运维摘要条单测。
// 契约与 Plan 逐字一致：
// - 摘要条 testid `lc-summary-bar`，一行展示 LC 名 / 索引状态 / 发布状态 chip + 「管理」展开按钮；
// - `hasWarning` 时以 amber/rose 警示样式突出并渲染 `lc-summary-warning`；
// - `onToggle` 由「管理」按钮触发，`expanded` 反映在 aria-expanded；
// - 视觉规范：hover 仅改颜色/边框/阴影（不位移）、150–300ms 过渡 + motion-reduce 降级、
//   cursor-pointer、focus-visible 主色 ring。
// 接线部分（IssueLifecycleWorkbench）：默认折叠只见摘要条、点「管理」后既有面板 testid 出现、
// 展开状态按 projectId 记忆到 localStorage["aria.workbench.lcSummary.<projectId>"]。
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  installIssueLifecycleWorkbenchTestHooks,
  lifecycleFetch,
  projectRecord,
} from "./IssueLifecycleWorkbench.test-utils";
import { LogicalCodebaseSummaryBar } from "./LogicalCodebaseSummaryBar";
import type { LogicalCodebaseSummary } from "./LogicalCodebaseSummaryBar";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

// 视觉规范允许的 hover 反馈前缀：仅颜色/边框/阴影（不得有位移/尺寸类）。
const ALLOWED_HOVER_PREFIXES = [
  "hover:bg-",
  "hover:text-",
  "hover:border-",
  "hover:shadow-",
];

function summary(
  overrides: Partial<LogicalCodebaseSummary> = {},
): LogicalCodebaseSummary {
  return {
    lcName: "platform",
    indexState: "active",
    publicationStatus: "completed_all",
    hasWarning: false,
    ...overrides,
  };
}

describe("LogicalCodebaseSummaryBar", () => {
  it("一行展示 LC 名 / 索引状态 / 发布状态 chip 与「管理」展开按钮", () => {
    render(
      <LogicalCodebaseSummaryBar
        summary={summary()}
        expanded={false}
        onToggle={vi.fn()}
      />,
    );

    const bar = screen.getByTestId("lc-summary-bar");
    // 一行：flex 行容器且不换行堆叠
    expect(bar).toHaveClass("flex", "items-center");
    expect(bar.className).not.toContain("flex-col");

    expect(screen.getByTestId("lc-summary-name")).toHaveTextContent("platform");
    expect(screen.getByTestId("lc-summary-index")).toHaveAttribute(
      "data-state",
      "active",
    );
    expect(screen.getByTestId("lc-summary-publication")).toHaveAttribute(
      "data-status",
      "completed_all",
    );
    // 状态标识一律胶囊 chip
    for (const testId of [
      "lc-summary-name",
      "lc-summary-index",
      "lc-summary-publication",
    ]) {
      expect(screen.getByTestId(testId)).toHaveClass("rounded-full");
    }
    expect(screen.getByTestId("lc-summary-toggle")).toBeInTheDocument();
  });

  it("缺少 LC 名 / 索引 / 发布数据时展示占位文案而非空白", () => {
    render(
      <LogicalCodebaseSummaryBar
        summary={summary({
          lcName: null,
          indexState: null,
          publicationStatus: null,
        })}
        expanded={false}
        onToggle={vi.fn()}
      />,
    );

    expect(screen.getByTestId("lc-summary-name")).toHaveTextContent("未选择");
    expect(screen.getByTestId("lc-summary-index")).toHaveTextContent(
      "索引 未建立",
    );
    expect(screen.getByTestId("lc-summary-publication")).toHaveTextContent(
      "发布 未发布",
    );
  });

  it("hasWarning 时摘要条以 amber 警示样式突出并渲染警示标记", () => {
    const { rerender } = render(
      <LogicalCodebaseSummaryBar
        summary={summary({ hasWarning: false })}
        expanded={false}
        onToggle={vi.fn()}
      />,
    );

    expect(screen.getByTestId("lc-summary-bar")).toHaveAttribute(
      "data-warning",
      "false",
    );
    expect(screen.queryByTestId("lc-summary-warning")).not.toBeInTheDocument();

    rerender(
      <LogicalCodebaseSummaryBar
        summary={summary({
          indexState: "missing",
          publicationStatus: "completed_partial",
          hasWarning: true,
        })}
        expanded={false}
        onToggle={vi.fn()}
      />,
    );

    const bar = screen.getByTestId("lc-summary-bar");
    expect(bar).toHaveAttribute("data-warning", "true");
    expect(bar).toHaveClass("border-amber-300", "bg-amber-50");
    expect(screen.getByTestId("lc-summary-warning")).toBeInTheDocument();
    // 发布失败/部分失败用 rose 强调
    expect(screen.getByTestId("lc-summary-publication")).toHaveClass(
      "text-rose-700",
    );
  });

  it("「管理」按钮触发 onToggle 并用 aria-expanded 反映展开状态", async () => {
    const onToggle = vi.fn();
    const user = userEvent.setup();
    const { rerender } = render(
      <LogicalCodebaseSummaryBar
        summary={summary()}
        expanded={false}
        onToggle={onToggle}
      />,
    );

    const toggle = screen.getByTestId("lc-summary-toggle");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(toggle).toHaveAccessibleName("展开逻辑代码库管理");

    await user.click(toggle);
    expect(onToggle).toHaveBeenCalledTimes(1);

    rerender(
      <LogicalCodebaseSummaryBar
        summary={summary()}
        expanded
        onToggle={onToggle}
      />,
    );
    const expandedToggle = screen.getByTestId("lc-summary-toggle");
    expect(expandedToggle).toHaveAttribute("aria-expanded", "true");
    expect(expandedToggle).toHaveAccessibleName("折叠逻辑代码库管理");
  });

  it("视觉规范：过渡 150–300ms + motion-reduce 降级、cursor-pointer、focus ring、hover 不位移", () => {
    render(
      <LogicalCodebaseSummaryBar
        summary={summary()}
        expanded={false}
        onToggle={vi.fn()}
      />,
    );

    const bar = screen.getByTestId("lc-summary-bar");
    expect(bar).toHaveClass("transition-colors", "motion-reduce:transition-none");
    expect(
      ["duration-150", "duration-200", "duration-300"].some((duration) =>
        bar.classList.contains(duration),
      ),
    ).toBe(true);

    const toggle = screen.getByTestId("lc-summary-toggle");
    expect(toggle).toHaveClass(
      "cursor-pointer",
      "transition-colors",
      "motion-reduce:transition-none",
      "focus-visible:ring-2",
      "focus-visible:ring-[var(--aria-primary)]",
    );
    expect(
      ["duration-150", "duration-200", "duration-300"].some((duration) =>
        toggle.classList.contains(duration),
      ),
    ).toBe(true);

    for (const element of [bar, toggle]) {
      const hoverClasses = Array.from(element.classList).filter((token) =>
        token.startsWith("hover:"),
      );
      for (const token of hoverClasses) {
        expect(
          ALLOWED_HOVER_PREFIXES.some((prefix) => token.startsWith(prefix)),
        ).toBe(true);
      }
    }
  });
});

describe("IssueLifecycleWorkbench 运维摘要条接线", () => {
  installIssueLifecycleWorkbenchTestHooks();
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("默认折叠：只见摘要条，完整管理面板不占用首屏", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [projectRecord("project_0001", "Aria")],
        logicalCodebases: [{ id: "lc_0001", name: "platform", member_count: 1 }],
      }),
    );

    render(<IssueLifecycleWorkbench />);

    const bar = await screen.findByTestId("lc-summary-bar");
    expect(bar).toHaveTextContent("platform");
    expect(
      screen.queryByTestId("pointer-publication-panel"),
    ).not.toBeInTheDocument();
    expect(screen.queryByTestId("lc-selector-platform")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "登记成员" }),
    ).not.toBeInTheDocument();
    // Task 7 外壳结构不受影响
    expect(screen.getByTestId("workbench-shell")).toBeInTheDocument();
    expect(screen.getByTestId("issue-queue-column")).toBeInTheDocument();
  });

  it("点「管理」展开后既有面板 testid 全部可见，再点收起", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [projectRecord("project_0001", "Aria")],
        logicalCodebases: [{ id: "lc_0001", name: "platform", member_count: 1 }],
        aggregateIndex: {
          state: "active",
          revision: 3,
          indexed_at: "2026-08-18T00:00:00Z",
          warning: null,
        },
      }),
    );
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("lc-summary-toggle"));

    expect(
      await screen.findByTestId("pointer-publication-panel"),
    ).toBeInTheDocument();
    expect(await screen.findByTestId("aggregate-index-status")).toHaveAttribute(
      "data-state",
      "active",
    );
    expect(
      await screen.findByTestId("aggregate-initialization-card"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("lc-selector-platform")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "登记成员" })).toBeEnabled();

    await user.click(screen.getByTestId("lc-summary-toggle"));
    await waitFor(() =>
      expect(
        screen.queryByTestId("pointer-publication-panel"),
      ).not.toBeInTheDocument(),
    );
  });

  it("索引非 active 或发布失败时摘要条进入警示态", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [projectRecord("project_0001", "Aria")],
        logicalCodebases: [{ id: "lc_0001", name: "platform", member_count: 1 }],
        aggregateIndex: {
          state: "stale",
          revision: 2,
          indexed_at: "2026-08-18T00:00:00Z",
          warning: null,
        },
      }),
    );

    render(<IssueLifecycleWorkbench />);

    // 先等聚合索引加载完成（state 变为 stale），避免 null 阶段的警示被误判。
    await waitFor(() =>
      expect(screen.getByTestId("lc-summary-index")).toHaveAttribute(
        "data-state",
        "stale",
      ),
    );
    expect(screen.getByTestId("lc-summary-bar")).toHaveAttribute(
      "data-warning",
      "true",
    );
    expect(screen.getByTestId("lc-summary-warning")).toBeInTheDocument();
  });

  it("聚合索引缺失（aggregateIndex 为 null，尚未建立）时摘要条进入警示态", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [projectRecord("project_0001", "Aria")],
        logicalCodebases: [{ id: "lc_0001", name: "platform", member_count: 0 }],
        // 无成员 → 尚未建立聚合索引 → aggregateIndex 保持 null。
        logicalCodebaseMembers: [],
      }),
    );

    render(<IssueLifecycleWorkbench />);

    await waitFor(() =>
      expect(screen.getByTestId("lc-summary-bar")).toHaveAttribute(
        "data-warning",
        "true",
      ),
    );
    expect(screen.getByTestId("lc-summary-index")).toHaveAttribute(
      "data-state",
      "unknown",
    );
    expect(screen.getByTestId("lc-summary-warning")).toBeInTheDocument();
  });

  it("展开状态按 projectId 记忆到 localStorage 并在重挂载后回填", async () => {
    const fetchMock = lifecycleFetch({
      projects: [projectRecord("project_0001", "Aria")],
      logicalCodebases: [{ id: "lc_0001", name: "platform", member_count: 1 }],
    });
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    const view = render(<IssueLifecycleWorkbench />);

    await user.click(await screen.findByTestId("lc-summary-toggle"));
    await screen.findByTestId("pointer-publication-panel");
    expect(
      window.localStorage.getItem("aria.workbench.lcSummary.project_0001"),
    ).toBe("1");

    view.unmount();
    render(<IssueLifecycleWorkbench />);

    // 记忆命中：重挂载后无需再点即展开
    expect(
      await screen.findByTestId("pointer-publication-panel"),
    ).toBeInTheDocument();

    await user.click(screen.getByTestId("lc-summary-toggle"));
    await waitFor(() =>
      expect(
        window.localStorage.getItem("aria.workbench.lcSummary.project_0001"),
      ).toBe("0"),
    );
  });

  it("无逻辑代码库的 project 不渲染摘要条", async () => {
    vi.stubGlobal(
      "fetch",
      lifecycleFetch({
        projects: [projectRecord("project_0001", "Aria")],
        logicalCodebases: [],
      }),
    );

    render(<IssueLifecycleWorkbench />);

    expect(
      await screen.findByRole("button", { name: "新建 Issue" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("lc-summary-bar")).not.toBeInTheDocument();
  });
});

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ArtifactVersionSummary } from "../../../api/types";
import { RevisionDiffView } from "./RevisionDiffView";

vi.mock("../../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

function version(versionNo: number, markdown?: string): ArtifactVersionSummary {
  return {
    version: versionNo,
    markdown,
    generated_by: "fake",
    reviewed_by: null,
    review_verdict: null,
    confirmed_by: null,
    is_current: false,
    created_at: "2026-09-14T08:00:00Z",
    source_node_id: `node_${versionNo}`,
  };
}

function renderView(
  versions: ArtifactVersionSummary[],
  options: {
    load?: (version: number) => Promise<string>;
    cache?: Record<number, string>;
    advisoryCount?: number | null;
  } = {},
) {
  return render(
    <RevisionDiffView
      sessionId="session_001"
      versions={versions}
      contentCache={options.cache ?? {}}
      loadVersionMarkdown={options.load ?? null}
      onCacheVersionMarkdown={vi.fn()}
      advisoryFindingCount={options.advisoryCount ?? null}
    />,
  );
}

describe("RevisionDiffView", () => {
  // F-38：单版本会话此前只渲染「暂无对比对象」空话术——plan 会话首轮（无第二轮）
  // 的产物全文与评审结论在「轮次差异」视图里不可达。单版本分支改为渲染当前版
  // 全文（markdown）+ review_verdict 只读标签 + 可选建议条数。
  it("renders the single round full text with its review verdict and advisory count", async () => {
    const load = vi.fn().mockResolvedValue("# Work Item Plan\n\n## WI-001 登录\n");
    renderView(
      [{ ...version(1), review_verdict: "pass", is_current: true }],
      { load, advisoryCount: 3 },
    );

    const fullText = await screen.findByTestId("monaco-viewer");
    expect(fullText).toHaveTextContent("# Work Item Plan");
    expect(fullText).toHaveTextContent("WI-001 登录");
    expect(load).toHaveBeenCalledWith(1);
    expect(screen.getByTestId("revision-diff-single-version")).toBeVisible();
    expect(screen.getByTestId("revision-diff-verdict")).toHaveTextContent("v1 审批：通过");
    expect(screen.getByTestId("revision-diff-advisory-count")).toHaveTextContent(
      "可选建议 3 条",
    );
  });

  it("renders the single round full text from the version summary without fetching", () => {
    const load = vi.fn();
    renderView([version(1, "# 计划全文\n目标 A\n")], { load });

    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("计划全文");
    expect(load).not.toHaveBeenCalled();
  });

  it("keeps the single round verdict label while omitting an empty advisory count", () => {
    renderView(
      [{ ...version(1, "# 计划\n"), review_verdict: "revise" }],
      { advisoryCount: null },
    );

    expect(screen.getByTestId("revision-diff-verdict")).toHaveTextContent(
      "v1 审批：建议返修",
    );
    expect(screen.queryByTestId("revision-diff-advisory-count")).toBeNull();
  });

  it("explains there is no artifact round yet for a session without versions", () => {
    renderView([]);
    expect(screen.getByTestId("revision-diff-view")).toHaveTextContent(
      "当前会话还没有 artifact 轮次",
    );
  });

  it("surfaces a single round load failure inline without crashing", async () => {
    const load = vi.fn().mockRejectedValue(new Error("boom"));
    renderView([version(1)], { load });
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("加载失败");
  });

  it("prefers the page cache for the single round without fetching", () => {
    const load = vi.fn();
    renderView([version(1)], { load, cache: { 1: "# 缓存全文\n" } });

    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("缓存全文");
    expect(load).not.toHaveBeenCalled();
  });

  it("keeps a version single-flight while a sibling request updates cache", async () => {
    let resolveSecond!: (markdown: string) => void;
    const second = new Promise<string>((resolve) => { resolveSecond = resolve; });
    const load = vi.fn((versionNo: number) =>
      versionNo === 1 ? Promise.resolve("v1\n") : second,
    );
    renderView([version(1, ""), version(2, "")], { load });
    await screen.findByText("正在加载轮次内容…");
    resolveSecond("v2\n");
    await screen.findByTestId("revision-diff-summary");
    expect(load).toHaveBeenCalledTimes(2);
    expect(load).toHaveBeenCalledWith(1);
    expect(load).toHaveBeenCalledWith(2);
  });

  it("defaults to the two latest rounds, loads them, and shows the summary", async () => {
    const load = vi
      .fn()
      .mockResolvedValueOnce("# 计划\n目标 A\n")
      .mockResolvedValueOnce("# 计划\n目标 B\n");
    renderView([version(1, ""), version(2, ""), version(3, "")], { load });

    const summary = await screen.findByTestId("revision-diff-summary");
    expect(summary).toHaveTextContent("+0");
    expect(summary).toHaveTextContent("-0");
    expect(summary).toHaveTextContent("~1");
    expect(load).toHaveBeenCalledTimes(2);
    expect(load).toHaveBeenCalledWith(2);
    expect(load).toHaveBeenCalledWith(3);

    const base = screen.getByTestId("revision-diff-base-select") as HTMLSelectElement;
    const target = screen.getByTestId("revision-diff-target-select") as HTMLSelectElement;
    expect(base.value).toBe("2");
    expect(target.value).toBe("3");
  });

  it("recomputes the diff when the user picks another base round", async () => {
    const markdowns: Record<number, string> = {
      1: "# 计划\n目标 0\n",
      2: "# 计划\n目标 A\n",
      3: "# 计划\n目标 B\n",
    };
    const load = vi.fn(async (v: number) => markdowns[v]);
    const user = userEvent.setup();
    renderView([version(1, ""), version(2, ""), version(3, "")], { load });

    await screen.findByTestId("revision-diff-summary");
    await user.selectOptions(
      screen.getByTestId("revision-diff-base-select"),
      "1",
    );
    const summary = await screen.findByTestId("revision-diff-summary");
    expect(summary).toHaveTextContent("~1");
    expect(load).toHaveBeenCalledWith(1);
  });

  it("renders hunk headers and per-line markers with line numbers", async () => {
    const load = vi
      .fn()
      .mockResolvedValueOnce("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n")
      .mockResolvedValueOnce("l1\nl2\nl3\nL4\nl5\nl6\nl7\nl8\n");
    renderView([version(1, ""), version(2, "")], { load });

    await screen.findByTestId("revision-diff-summary");
    const hunk = screen.getByTestId("revision-diff-hunk");
    // 8 行文件、index 3 变更、context 3：hunk 覆盖 1–7 行（3 上下文+1 变更+3 上下文），
    // 头为 -1,7 +1,7（controller 修正 1，与 T1 revision-diff.test.ts 同构用例一致）。
    expect(within(hunk).getByText("@@ -1,7 +1,7 @@")).toBeVisible();
    const removedLine = screen.getByTestId("revision-diff-line-removed");
    expect(removedLine).toHaveTextContent("l4");
    const addedLine = screen.getByTestId("revision-diff-line-added");
    expect(addedLine).toHaveTextContent("L4");
  });

  it("prefers cached markdown without loading", () => {
    const load = vi.fn();
    renderView(
      [version(1, ""), version(2, "")],
      { load, cache: { 1: "a\n", 2: "a\n" } },
    );
    // 「两轮内容一致」在 summary div 的兄弟 <p> 内，断言挂容器（controller 修正 2）。
    expect(screen.getByTestId("revision-diff-view")).toHaveTextContent(
      "两轮内容一致",
    );
    expect(screen.getByTestId("revision-diff-summary")).toHaveTextContent("+0");
    expect(load).not.toHaveBeenCalled();
  });

  it("surfaces read-only review verdicts for the compared rounds", () => {
    // controller 修正 3：并列呈现「为什么改」——被对比轮次的 review_verdict 只读标签。
    renderView(
      [
        { ...version(1, ""), review_verdict: "pass" },
        { ...version(2, ""), review_verdict: "revise" },
      ],
      { cache: { 1: "a\n", 2: "b\n" } },
    );

    expect(screen.getAllByTestId("revision-diff-verdict")).toHaveLength(2);
    expect(screen.getByTestId("revision-diff-view")).toHaveTextContent(
      "v1 审批：通过",
    );
    expect(screen.getByTestId("revision-diff-view")).toHaveTextContent(
      "v2 审批：建议返修",
    );
  });

  it("surfaces load failures inline without crashing", async () => {
    const load = vi.fn().mockRejectedValue(new Error("boom"));
    renderView([version(1, ""), version(2, "")], { load });
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("加载失败");
  });
});

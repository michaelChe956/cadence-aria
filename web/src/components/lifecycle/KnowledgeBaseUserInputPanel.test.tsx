// Task 6：知识库输入确认区组件测试（REQ-KB-15 前端面）。
// 覆盖：9 份文件三态徽标与「待确认 N 份」提示、逐份确认携带当前读到的
// digest、全部确认只覆盖待确认文件、过期确认（stale 409）的可执行提示与
// 投影刷新、「已修改，需重新确认」失效提示、点击某份交给页面打开。
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { KnowledgeBaseUserInputPanel } from "./KnowledgeBaseUserInputPanel";
import type { KnowledgeBaseUserInputSummaryDto } from "../../api/types/knowledge-base";

type FetchCall = { url: string; method: string; body?: unknown };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

/** 便捷构造汇总投影：默认已填写且未确认（filled ⇒ pending）。 */
function userInputSummary(
  files: { path: string; filled?: boolean; confirmed?: boolean }[],
): KnowledgeBaseUserInputSummaryDto {
  const rows = files.map((file) => {
    const filled = file.filled ?? true;
    const confirmed = file.confirmed ?? false;
    return { path: file.path, filled, confirmed, pending: filled && !confirmed };
  });
  return {
    files: rows,
    confirmed_count: rows.filter((row) => row.confirmed).length,
    total_count: rows.length,
  };
}

function installFetch(
  overrides: {
    confirm?: () => Response;
    digest?: (path: string) => string;
  } = {},
): { calls: FetchCall[] } {
  const calls: FetchCall[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    calls.push({
      url,
      method,
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    });
    if (url.includes("/knowledge-base/user-input/confirm")) {
      return overrides.confirm
        ? overrides.confirm()
        : jsonResponse({ confirmed: [] });
    }
    if (url.includes("/knowledge-base/file")) {
      const path = decodeURIComponent(
        new URL(url, "http://localhost").searchParams.get("path") ?? "",
      );
      const digest = overrides.digest
        ? overrides.digest(path)
        : `digest-${path.split("/").at(-1)}`;
      return jsonResponse({ path, content: "# 内容", digest });
    }
    return jsonResponse({ code: "not_found", message: `未知请求 ${url}` }, 404);
  });
  vi.stubGlobal("fetch", fetchMock);
  return { calls };
}

function renderPanel(
  summary: KnowledgeBaseUserInputSummaryDto,
  props: {
    invalidatedPaths?: ReadonlySet<string>;
    selectedPath?: string | null;
    onOpenFile?: (path: string) => void;
    onSummaryChanged?: () => void;
    onAllConfirmed?: () => void;
    initializing?: boolean;
  } = {},
) {
  const onOpenFile = vi.fn();
  const onSummaryChanged = vi.fn();
  render(
    <KnowledgeBaseUserInputPanel
      projectId="p1"
      lcId="lc1"
      summary={summary}
      invalidatedPaths={props.invalidatedPaths ?? new Set<string>()}
      selectedPath={props.selectedPath ?? null}
      onOpenFile={props.onOpenFile ?? onOpenFile}
      onSummaryChanged={props.onSummaryChanged ?? onSummaryChanged}
      onAllConfirmed={props.onAllConfirmed}
      initializing={props.initializing}
    />,
  );
  return { onOpenFile, onSummaryChanged };
}

/** 按 path 定位徽标节点。 */
function badgeOf(path: string) {
  return screen
    .getAllByTestId("kb-user-input-badge")
    .find((node) => node.getAttribute("data-path") === path);
}

describe("KnowledgeBaseUserInputPanel", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("renders_file_badges_and_pending_count_hint", () => {
    installFetch();
    renderPanel(
      userInputSummary([
        { path: "user-input/goal.md", confirmed: true },
        { path: "user-input/scope.md" },
        { path: "user-input/architecture.md" },
        { path: "user-input/blank.md", filled: false },
      ]),
    );

    expect(badgeOf("user-input/goal.md")).toHaveTextContent("已确认");
    expect(badgeOf("user-input/scope.md")).toHaveTextContent("待确认");
    expect(badgeOf("user-input/architecture.md")).toHaveTextContent("待确认");
    expect(badgeOf("user-input/blank.md")).toHaveTextContent("未填");
    // 待确认数 = 总数 - 已确认数（含未填份额，与后端 not_confirmed 口径一致）。
    const summary = screen.getByTestId("kb-user-input-summary");
    expect(summary).toHaveTextContent("待确认 3 份");
    expect(summary).toHaveTextContent("已确认 1/4");
  });

  it("all_confirmed_shows_ready_hint_and_disables_confirm_all", () => {
    installFetch();
    renderPanel(
      userInputSummary([
        { path: "user-input/goal.md", confirmed: true },
        { path: "user-input/scope.md", confirmed: true },
      ]),
    );

    expect(screen.getByTestId("kb-user-input-summary")).toHaveTextContent(
      "可执行【知识库初始化】",
    );
    expect(screen.getByTestId("kb-user-input-confirm-all")).toBeDisabled();
  });

  it("confirm_button_only_appears_for_pending_files", () => {
    installFetch();
    renderPanel(
      userInputSummary([
        { path: "user-input/goal.md", confirmed: true },
        { path: "user-input/scope.md" },
        { path: "user-input/blank.md", filled: false },
      ]),
    );

    const buttons = screen.getAllByTestId("kb-user-input-confirm");
    expect(buttons.map((button) => button.getAttribute("data-path"))).toEqual([
      "user-input/scope.md",
    ]);
  });

  it("clicking_file_row_opens_file", () => {
    installFetch();
    const { onOpenFile } = renderPanel(
      userInputSummary([{ path: "user-input/goal.md" }]),
    );

    fireEvent.click(screen.getByTestId("kb-user-input-open"));

    expect(onOpenFile).toHaveBeenCalledWith("user-input/goal.md");
  });

  it("confirm_one_carries_current_digest_and_refreshes", async () => {
    const { calls } = installFetch();
    const { onSummaryChanged } = renderPanel(
      userInputSummary([{ path: "user-input/goal.md" }]),
    );

    fireEvent.click(screen.getByTestId("kb-user-input-confirm"));

    // 确认请求必须携带「当前读到的 digest」：先读文件取服务端摘要再提交。
    await waitFor(() => {
      const post = calls.find((call) => call.method === "POST");
      expect(post?.body).toEqual({
        path: "user-input/goal.md",
        digest: "digest-goal.md",
      });
    });
    expect(await screen.findByTestId("kb-user-input-notice")).toHaveTextContent(
      "已确认",
    );
    expect(onSummaryChanged).toHaveBeenCalled();
  });

  it("confirm_all_only_covers_pending_files_with_current_digests", async () => {
    const { calls } = installFetch();
    const { onSummaryChanged } = renderPanel(
      userInputSummary([
        { path: "user-input/goal.md", confirmed: true },
        { path: "user-input/scope.md" },
        { path: "user-input/architecture.md" },
        { path: "user-input/blank.md", filled: false },
      ]),
    );

    fireEvent.click(screen.getByTestId("kb-user-input-confirm-all"));

    // 批量确认只带「待确认」文件（已确认无需重复；未填的确认无意义——
    // 服务端投影的 confirmed 只计已填写文件）。
    await waitFor(() => {
      const post = calls.find((call) => call.method === "POST");
      expect(post?.body).toEqual({
        all: true,
        items: [
          { path: "user-input/scope.md", digest: "digest-scope.md" },
          { path: "user-input/architecture.md", digest: "digest-architecture.md" },
        ],
      });
    });
    expect(onSummaryChanged).toHaveBeenCalled();
  });

  it("stale_confirmation_shows_actionable_error_and_refreshes", async () => {
    installFetch({
      confirm: () =>
        jsonResponse(
          {
            code: "knowledge_base_user_input_confirmation_stale",
            message:
              "确认已过期：user-input/goal.md 的当前内容与提交的摘要不一致，请刷新后重新确认。",
          },
          409,
        ),
    });
    const { onSummaryChanged } = renderPanel(
      userInputSummary([{ path: "user-input/goal.md" }]),
    );

    fireEvent.click(screen.getByTestId("kb-user-input-confirm"));

    // 过期 ⇒ 文件内容已变化：给出可执行提示并刷新汇总投影（徽标回最新事实）。
    expect(await screen.findByTestId("kb-user-input-notice")).toHaveTextContent(
      "确认已过期",
    );
    expect(onSummaryChanged).toHaveBeenCalled();
  });

  it("invalidated_file_shows_reconfirmation_hint", () => {
    installFetch();
    renderPanel(
      userInputSummary([
        { path: "user-input/goal.md" },
        { path: "user-input/scope.md" },
      ]),
      { invalidatedPaths: new Set(["user-input/goal.md"]) },
    );

    const hints = screen.getAllByTestId("kb-user-input-invalidated");
    expect(hints).toHaveLength(1);
    expect(hints[0]).toHaveTextContent("已修改，需重新确认");
    expect(hints[0].getAttribute("data-path")).toBe("user-input/goal.md");
  });

  // ── OpenSpec change add-knowledge-base-interactive-init-flow ──────────────

  it("renders_step_by_step_guidance", () => {
    installFetch();
    renderPanel(
      userInputSummary([{ path: "user-input/goal.md" }]),
      { onAllConfirmed: vi.fn() },
    );

    const guide = screen.getByTestId("kb-user-input-guide");
    // 四步指引齐全：补充 → 确认 → 全部确认后初始化 → 留在本页继续编辑。
    expect(guide.textContent).toContain("逐份打开");
    expect(guide.textContent).toContain("需重新确认");
    expect(guide.textContent).toContain("确认并初始化");
    expect(guide.textContent).toContain("以你的修改为准");
  });

  it("unfilled_files_block_initialization_with_reason", () => {
    installFetch();
    renderPanel(
      userInputSummary([
        { path: "user-input/goal.md" },
        { path: "user-input/blank.md", filled: false },
      ]),
      { onAllConfirmed: vi.fn() },
    );

    // 未填写的文件无法被确认 ⇒ 不允许建库（只引导，不放宽后端口径）。
    expect(screen.getByTestId("kb-user-input-unfilled")).toHaveTextContent(
      "还有 1 份未填写",
    );
    expect(
      screen.getByTestId("kb-user-input-confirm-and-initialize"),
    ).toBeDisabled();
  });

  it("confirm_and_initialize_confirms_pending_then_notifies_host", async () => {
    const { calls } = installFetch();
    const onAllConfirmed = vi.fn();
    renderPanel(
      userInputSummary([
        { path: "user-input/goal.md", confirmed: true },
        { path: "user-input/scope.md" },
      ]),
      { onAllConfirmed },
    );

    fireEvent.click(screen.getByTestId("kb-user-input-confirm-and-initialize"));

    // 先批量确认（携带当前 digest），再交给宿主建库。
    await waitFor(() => {
      expect(
        calls.some((call) => call.url.includes("/user-input/confirm")),
      ).toBe(true);
    });
    await waitFor(() => {
      expect(onAllConfirmed).toHaveBeenCalledTimes(1);
    });
  });

  it("confirm_and_initialize_does_not_start_bootstrap_when_confirmation_fails", async () => {
    installFetch({
      confirm: () =>
        jsonResponse(
          {
            code: "knowledge_base_user_input_confirmation_stale",
            message: "确认已过期：请刷新后重新确认。",
          },
          409,
        ),
    });
    const onAllConfirmed = vi.fn();
    renderPanel(
      userInputSummary([{ path: "user-input/goal.md" }]),
      { onAllConfirmed },
    );

    fireEvent.click(screen.getByTestId("kb-user-input-confirm-and-initialize"));

    expect(await screen.findByTestId("kb-user-input-notice")).toHaveTextContent(
      "确认已过期",
    );
    expect(onAllConfirmed).not.toHaveBeenCalled();
  });

  it("confirm_and_initialize_is_disabled_while_initializing", () => {
    installFetch();
    renderPanel(
      userInputSummary([{ path: "user-input/goal.md" }]),
      { onAllConfirmed: vi.fn(), initializing: true },
    );

    const button = screen.getByTestId("kb-user-input-confirm-and-initialize");
    expect(button).toBeDisabled();
    expect(button).toHaveTextContent("正在初始化知识库…");
  });
});

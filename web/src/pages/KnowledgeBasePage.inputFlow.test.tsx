// 知识库详情页的**输入流**用例：起草完成后自动打开输入文件、确认区与
// 【确认并初始化】的行为。
//
// 单独成文件是为了守住「单文件行数红线」（1200）：主体用例文件已近上限。
// 共享夹具在 `KnowledgeBasePage.test-utils.ts`；`vi.mock` 留在本文件
// （mock 注册按文件生效）。
import { fireEvent, screen, waitFor } from "@testing-library/react";
import { useEffect } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  installFetch,
  jsonResponse,
  renderPage,
  userInputSummary,
  waitForEditorValue,
} from "./KnowledgeBasePage.test-utils";

vi.mock("@monaco-editor/react", () => ({
  loader: {
    config: vi.fn(),
  },
  Editor: ({
    value,
    onChange,
    options,
  }: {
    value?: string;
    onChange?: (value: string | undefined) => void;
    options?: { readOnly?: boolean };
  }) => {
    useEffect(() => undefined, []);
    return (
      <div
        data-testid="monaco-editor"
        data-readonly={options?.readOnly ? "true" : "false"}
      >
        <textarea
          data-testid="kb-editor"
          value={value ?? ""}
          onChange={(event) => onChange?.(event.target.value)}
        />
      </div>
    );
  },
}));

vi.mock("../hooks/useWorkspaceSessionObservers", () => ({
  useWorkspaceSessionObservers: () => ({
    records: [],
    inbox: [],
    countedInbox: [],
    watchedSessionIds: [],
    watchSession: () => undefined,
    codingAttemptForSession: () => null,
    displayItems: [],
    actionableCount: 0,
    notificationCandidates: [],
  }),
}));

describe("KnowledgeBasePage 输入流", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("opens_first_pending_user_input_file_when_knowledge_base_not_built", async () => {
    // 起草完成、知识库尚未建成 ⇒ 自动打开第一份待确认的输入文件。
    installFetch({
      status: () =>
        jsonResponse({
          exists: false,
          schema_version: null,
          initialization_status: null,
          global_validation: null,
          completed_stages: [],
          skipped_stages: [],
          skills_available: true,
        }),
      userInput: () =>
        jsonResponse(
          userInputSummary([
            { path: "user-input/goal.md" },
            { path: "user-input/scope.md" },
          ]),
        ),
    });
    renderPage();

    // 第一份待确认文件在右侧编辑器打开（内容为该文件正文）。
    await waitForEditorValue("# 目标");
  });

  it("does_not_auto_open_user_input_when_knowledge_base_already_exists", async () => {
    installFetch({
      userInput: () =>
        jsonResponse(userInputSummary([{ path: "user-input/goal.md" }])),
    });
    renderPage();

    await screen.findByTestId("kb-file-tree");
    // 已建成的知识库：保持默认打开的第一份文件，不被输入文件抢焦点。
    await waitForEditorValue("# 基础信息");
  });

  it("disables_the_initialize_button_once_the_knowledge_base_is_built", async () => {
    // 知识库已建成 ⇒ 「确认并初始化」不再可用（重复跑一次 bootstrap 没有意义）。
    installFetch({
      status: () =>
        jsonResponse({
          exists: true,
          schema_version: "4.0",
          initialization_status: "complete",
          global_validation: "passed",
          completed_stages: ["base-info"],
          skipped_stages: [],
          skills_available: true,
        }),
      userInput: () =>
        jsonResponse(userInputSummary([{ path: "user-input/goal.md" }])),
    });
    renderPage();

    const button = await screen.findByTestId(
      "kb-user-input-confirm-and-initialize",
    );
    expect(button).toBeDisabled();
    expect(button).toHaveTextContent("知识库已初始化");
  });

  it("confirm_and_initialize_creates_bootstrap_operation_and_refreshes", async () => {
    const { calls } = installFetch({
      status: () =>
        jsonResponse({
          exists: false,
          schema_version: null,
          initialization_status: null,
          global_validation: null,
          completed_stages: [],
          skipped_stages: [],
          skills_available: true,
        }),
      userInput: () =>
        jsonResponse(userInputSummary([{ path: "user-input/goal.md" }])),
      initializations: () =>
        jsonResponse({
          operation_id: "kb_op_0002",
          project_id: "p1",
          logical_codebase_id: "lc1",
          kind: "bootstrap",
          status: "running",
          steps: [],
          error: null,
          created_at: "2026-01-01T00:00:00Z",
          started_at: null,
          completed_at: null,
        }),
      // 建库 operation 已是终态 completed ⇒ 页面刷新并给出完成提示。
      activeOperation: () =>
        jsonResponse({
          operation_id: "kb_op_0002",
          project_id: "p1",
          logical_codebase_id: "lc1",
          kind: "bootstrap",
          status: "completed",
          steps: [],
          error: null,
          created_at: "2026-01-01T00:00:00Z",
          started_at: null,
          completed_at: null,
        }),
    });
    renderPage();

    const button = await screen.findByTestId(
      "kb-user-input-confirm-and-initialize",
    );
    fireEvent.click(button);

    // 建库 operation 被创建（kind=bootstrap）。
    await waitFor(() => {
      const post = calls.find(
        (call) =>
          call.method === "POST" && call.url.includes("/knowledge-base/initializations"),
      );
      expect(post).toBeDefined();
      expect(post?.body).toMatchObject({ kind: "bootstrap" });
    });
    // 建库完成后给出「可继续编辑」提示。
    expect(await screen.findByTestId("kb-init-notice")).toHaveTextContent(
      "知识库已生成，可继续编辑",
    );
  });
});

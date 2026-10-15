import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../state/chat-entries";
import type * as ApiClient from "../api/client";
import {
  ApiRequestError,
  getCodingAttemptSnapshot,
  postCodingChoiceResponse,
} from "../api/client";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
  readyCodingState,
} from "./CodingWorkspacePage.test-utils";

vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiClient>()),
  confirmWorkItemExecutionPlan: vi.fn(),
  deleteCodingAttempt: vi.fn(),
  getCodingAttemptDiff: vi.fn(),
  getCodingAttemptSnapshot: vi.fn(),
  postCodingChoiceResponse: vi.fn(),
  requestWorkItemExecutionPlanChange: vi.fn(),
}));

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => <div data-testid="monaco-viewer">{value}</div>,
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: ({ original, modified }: { original: string; modified: string }) => (
    <div data-testid="monaco-diff-viewer">
      <span>{original}</span>
      <span>{modified}</span>
    </div>
  ),
}));


function restChoiceEntry(overrides: Partial<ChatEntry> = {}): ChatEntry {
  return {
    id: "choice_request:choice-1",
    type: "choice_request",
    role: "coder",
    content: "编码拆分确认",
    timestamp: "2026-10-15T00:00:00Z",
    metadata: {
      request_id: "choice-1",
      prompt: "编码拆分确认",
      source: "provider",
      options: [],
      allow_multiple: false,
      allow_free_text: false,
      questions: [
        {
          id: "q-1",
          prompt: "是否补齐测试",
          options: [
            { id: "yes", label: "补齐" },
            { id: "no", label: "暂不" },
          ],
          allow_multiple: false,
          allow_free_text: false,
        },
        {
          id: "q-2",
          prompt: "评审节奏",
          options: [
            { id: "one", label: "先自审" },
            { id: "two", label: "直接评审" },
          ],
          allow_multiple: false,
          allow_free_text: false,
        },
      ],
      expected_run_id: "run-1",
    },
    ...overrides,
  } as ChatEntry;
}

function entryWithoutExpectedRunId(): ChatEntry {
  const entry = restChoiceEntry();
  const metadata = { ...(entry.metadata as Record<string, unknown>) };
  delete metadata.expected_run_id;
  return { ...entry, metadata };
}

function snapshotWithChoices(choices: unknown[]): CodingAttemptSnapshotResponse {
  return { pending_choices: choices } as CodingAttemptSnapshotResponse;
}

describe("CodingWorkspacePage choice REST submission (P2-e)", () => {
  installCodingWorkspacePageTestHooks();

  function renderReadyPage(entry: ChatEntry) {
    const wsApi = mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "waiting_for_human",
      stage: "coding",
      chatEntries: [entry],
    });
    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);
    return { wsApi };
  }

  async function answerBothQuestions(user: UserEvent) {
    const card = screen.getByTestId("choice-request-entry");
    await user.click(within(card).getByRole("radio", { name: "补齐" }));
    await user.click(within(card).getByRole("radio", { name: "直接评审" }));
    await user.click(within(card).getByRole("button", { name: "提交选择" }));
  }

  beforeEach(() => {
    vi.mocked(postCodingChoiceResponse).mockReset();
    vi.mocked(getCodingAttemptSnapshot).mockReset();
  });

  // P2-e 红测③：多题卡提交走 REST——body 恒带逐题 answers、command_id、
  // expected_run_id；不再走 WS choice_response。
  it("submits a multi-question choice via REST with per-question answers", async () => {
    const user = userEvent.setup();
    vi.mocked(postCodingChoiceResponse).mockResolvedValue({
      command_id: "cmd-1",
      expected_run_id: "run-1",
      choice_id: "choice-1",
      state: "delivered",
    });
    const { wsApi } = renderReadyPage(restChoiceEntry());

    await answerBothQuestions(user);

    expect(postCodingChoiceResponse).toHaveBeenCalledTimes(1);
    expect(postCodingChoiceResponse).toHaveBeenCalledWith(CODING_ATTEMPT_ADDRESS, "choice-1", {
      command_id: expect.any(String),
      expected_run_id: "run-1",
      answers: [
        { question_id: "q-1", selected_option_ids: ["yes"], free_text: null },
        { question_id: "q-2", selected_option_ids: ["two"], free_text: null },
      ],
    });
    expect(wsApi.respondChoice).not.toHaveBeenCalled();
    expect(await screen.findByText("已选择")).toBeInTheDocument();
  });

  // P2-e 红测④（X1）：无 expected_run_id（快照回落也无活跃 run）→ 不发请求，
  // 卡片显示人话提示且没有「恢复运行」按钮。
  it("shows a plain hint and sends nothing when no active run holds the choice", async () => {
    const user = userEvent.setup();
    vi.mocked(getCodingAttemptSnapshot).mockResolvedValue(snapshotWithChoices([]));
    renderReadyPage(entryWithoutExpectedRunId());

    await answerBothQuestions(user);

    await waitFor(() => {
      expect(getCodingAttemptSnapshot).toHaveBeenCalledWith(CODING_ATTEMPT_ADDRESS);
    });
    expect(postCodingChoiceResponse).not.toHaveBeenCalled();
    expect(
      await screen.findByText("AI 当前没有在运行，这个问题暂时无法回答"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "恢复运行" })).toBeNull();
  });

  // 设计 §3.6：expected_run_id 取自 entry.metadata（快照路径）或最新快照——
  // 实时帧 entry 不带化身时回落 REST 快照（handler 按活跃 run 注入）。
  it("falls back to the latest snapshot for the active run incarnation", async () => {
    const user = userEvent.setup();
    vi.mocked(getCodingAttemptSnapshot).mockResolvedValue(
      snapshotWithChoices([
        {
          choice_id: "choice-1",
          expected_run_id: "run-from-snapshot",
        },
      ]),
    );
    vi.mocked(postCodingChoiceResponse).mockResolvedValue({
      command_id: "cmd-2",
      expected_run_id: "run-from-snapshot",
      choice_id: "choice-1",
      state: "delivered",
    });
    renderReadyPage(entryWithoutExpectedRunId());

    await answerBothQuestions(user);

    await waitFor(() => {
      expect(postCodingChoiceResponse).toHaveBeenCalledWith(
        CODING_ATTEMPT_ADDRESS,
        "choice-1",
        expect.objectContaining({ expected_run_id: "run-from-snapshot" }),
      );
    });
  });

  // P2-e 红测⑤：expired（410）→ 失效提示，不再渲染可作答控件。
  it("marks the card expired when the REST reply is gone", async () => {
    const user = userEvent.setup();
    vi.mocked(postCodingChoiceResponse).mockRejectedValue(
      new ApiRequestError({
        code: "coding_choice_expired",
        message: "choice response targets a finished or missing coding run",
        details: {},
      }),
    );
    renderReadyPage(restChoiceEntry());

    await answerBothQuestions(user);

    expect(await screen.findByText("这个问题已失效（AI 已重新开始）")).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "提交选择" })).toBeNull();
    });
  });

  // F-10：提交冲突/失败保留卡片可改答案重试（显示 errorMessage，不失效）。
  it("keeps the card retryable with the error message on a conflict", async () => {
    const user = userEvent.setup();
    vi.mocked(postCodingChoiceResponse).mockRejectedValue(
      new ApiRequestError({
        code: "coding_choice_conflict",
        message: "应答与在途命令冲突",
        details: {},
      }),
    );
    renderReadyPage(restChoiceEntry());

    await answerBothQuestions(user);

    expect(await screen.findByText(/应答与在途命令冲突/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "提交选择" })).toBeEnabled();
  });

  // 精确断言入口：act 内直接触发（避免 user 计时器噪音）后同步检查 store。
  it("does not touch the WS choice channel when submitting through REST", async () => {
    const user = userEvent.setup();
    vi.mocked(postCodingChoiceResponse).mockResolvedValue({
      command_id: "cmd-3",
      expected_run_id: "run-1",
      choice_id: "choice-1",
      state: "delivered",
    });
    const { wsApi } = renderReadyPage(restChoiceEntry());

    await answerBothQuestions(user);

    await waitFor(() => {
      expect(postCodingChoiceResponse).toHaveBeenCalled();
    });
    expect(wsApi.respondChoice).not.toHaveBeenCalled();
    act(() => {});
  });
});

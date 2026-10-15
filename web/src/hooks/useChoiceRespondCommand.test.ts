import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../api/client";
import { ApiRequestError, getCodingChoiceResponseStatus, postCodingChoiceResponse } from "../api/client";
import type { ChoiceReplyStatus } from "../api/types";
import type { ChoiceResponsePayload } from "../state/chat-entries";
import { useChoiceRespondCommand, type ChoiceRespondCommandState } from "./useChoiceRespondCommand";

vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiClient>()),
  postCodingChoiceResponse: vi.fn(),
  getCodingChoiceResponseStatus: vi.fn(),
}));

const ADDRESS = {
  projectId: "project_0001",
  issueId: "issue_0001",
  attemptId: "coding_attempt_0001",
} as const;

const TWO_QUESTION_PAYLOAD: ChoiceResponsePayload = {
  selected_option_ids: ["yes", "two"],
  free_text: null,
  answers: [
    { question_id: "q-1", selected_option_ids: ["yes"], free_text: null },
    { question_id: "q-2", selected_option_ids: ["two"], free_text: "两轮更稳" },
  ],
};

function reply(commandId: string, state: ChoiceReplyStatus["state"]): ChoiceReplyStatus {
  return {
    command_id: commandId,
    expected_run_id: "run-1",
    choice_id: "choice-1",
    state,
  };
}

describe("useChoiceRespondCommand", () => {
  beforeEach(() => {
    vi.mocked(postCodingChoiceResponse).mockReset();
    vi.mocked(getCodingChoiceResponseStatus).mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // P2-e 红测③：多题提交走 REST，body 恒带逐题 answers + command_id + expected_run_id。
  it("posts per-question answers with command_id and expected_run_id", async () => {
    vi.mocked(postCodingChoiceResponse).mockResolvedValue(reply("cmd-1", "delivered"));
    const { result } = renderHook(() => useChoiceRespondCommand(ADDRESS));

    await act(async () => {
      result.current.respondChoice("choice-1", "run-1", TWO_QUESTION_PAYLOAD);
    });

    expect(postCodingChoiceResponse).toHaveBeenCalledTimes(1);
    expect(postCodingChoiceResponse).toHaveBeenCalledWith(ADDRESS, "choice-1", {
      command_id: expect.any(String),
      expected_run_id: "run-1",
      answers: [
        { question_id: "q-1", selected_option_ids: ["yes"], free_text: null },
        { question_id: "q-2", selected_option_ids: ["two"], free_text: "两轮更稳" },
      ],
    });
    expect(result.current.choiceCommands["choice-1"]?.status).toBe("delivered");
  });

  // P2-e 红测⑥：无逐题 answers 的旧单题 payload 回落单一 default 题。
  it("falls back to a single default question answer", async () => {
    vi.mocked(postCodingChoiceResponse).mockResolvedValue(reply("cmd-2", "delivered"));
    const { result } = renderHook(() => useChoiceRespondCommand(ADDRESS));

    await act(async () => {
      result.current.respondChoice("choice-1", "run-1", {
        selected_option_ids: ["dp"],
        free_text: "use iterative dp",
      });
    });

    expect(postCodingChoiceResponse).toHaveBeenCalledWith(ADDRESS, "choice-1", {
      command_id: expect.any(String),
      expected_run_id: "run-1",
      answers: [{ question_id: "default", selected_option_ids: ["dp"], free_text: "use iterative dp" }],
    });
  });

  // P2-e 红测⑤：expired（410 coding_choice_expired）不路由新命令，状态面标失效。
  it("maps coding_choice_expired to the expired terminal state", async () => {
    vi.mocked(postCodingChoiceResponse).mockRejectedValue(
      new ApiRequestError({
        code: "coding_choice_expired",
        message: "choice response targets a finished or missing coding run",
        details: {},
      }),
    );
    const { result } = renderHook(() => useChoiceRespondCommand(ADDRESS));

    await act(async () => {
      result.current.respondChoice("choice-1", "run-1", TWO_QUESTION_PAYLOAD);
    });

    expect(result.current.choiceCommands["choice-1"]).toMatchObject({
      status: "expired",
      errorCode: "coding_choice_expired",
    });
  });

  // 命令谱系（抽取自 useCockpitChoiceRespond）：错误后重试复用同 command_id。
  it("retries with the same command id after an error", async () => {
    vi.mocked(postCodingChoiceResponse)
      .mockRejectedValueOnce(
        new ApiRequestError({ code: "coding_choice_conflict", message: "冲突", details: {} }),
      )
      .mockResolvedValueOnce(reply("cmd-3", "delivered"));
    const { result } = renderHook(() => useChoiceRespondCommand(ADDRESS));

    await act(async () => {
      result.current.respondChoice("choice-1", "run-1", TWO_QUESTION_PAYLOAD);
    });
    await act(async () => {
      result.current.respondChoice("choice-1", "run-1", TWO_QUESTION_PAYLOAD);
    });

    const calls = vi.mocked(postCodingChoiceResponse).mock.calls;
    expect(calls[1][2].command_id).toBe(calls[0][2].command_id);
    expect(calls[1][2].answers).toEqual(calls[0][2].answers);
    expect(result.current.choiceCommands["choice-1"]?.status).toBe("delivered");
  });

  // 命令谱系：202 后同 command GET 复查到 delivered 收敛。
  it("rechecks a 202 response with the same command until delivered", async () => {
    vi.useFakeTimers();
    vi.mocked(postCodingChoiceResponse).mockResolvedValue(reply("cmd-4", "submitting"));
    vi.mocked(getCodingChoiceResponseStatus).mockResolvedValue(reply("cmd-4", "delivered"));
    const { result } = renderHook(() => useChoiceRespondCommand(ADDRESS));

    await act(async () => {
      result.current.respondChoice("choice-1", "run-1", TWO_QUESTION_PAYLOAD);
    });
    expect(result.current.choiceCommands["choice-1"]?.status).toBe("submitting");

    await act(async () => {
      vi.advanceTimersByTime(250);
    });

    expect(getCodingChoiceResponseStatus).toHaveBeenCalledWith(ADDRESS, "choice-1", "cmd-4");
    expect(result.current.choiceCommands["choice-1"]?.status).toBe("delivered");
  });

  // 命令谱系：状态变迁通过 onStateChange 通知调用方（页面据此收敛卡片）。
  it("notifies state transitions with the assembled request", async () => {
    vi.mocked(postCodingChoiceResponse).mockResolvedValue(reply("cmd-5", "delivered"));
    const observed: { choiceId: string; state: ChoiceRespondCommandState }[] = [];
    const { result } = renderHook(() =>
      useChoiceRespondCommand(ADDRESS, (choiceId, state) => {
        observed.push({ choiceId, state });
      }),
    );

    await act(async () => {
      result.current.respondChoice("choice-1", "run-1", TWO_QUESTION_PAYLOAD);
    });

    expect(observed.map((entry) => entry.state.status)).toEqual(["submitting", "delivered"]);
    expect(observed[1].state.commandId).toBe("cmd-5");
    expect(observed.every((entry) => entry.choiceId === "choice-1")).toBe(true);
  });
});

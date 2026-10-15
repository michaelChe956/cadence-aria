import { useCallback, useRef, useState } from "react";
import type {
  ChoiceAnswer,
  ChoiceReplyStatus,
  ChoiceResponseRequest,
  CodingAttemptAddress,
} from "../api/types";
import { getCodingChoiceResponseStatus, postCodingChoiceResponse } from "../api/client";
import type { ChoiceResponsePayload } from "../state/chat-entries";
import { newCommandId } from "./useWorkspaceWs";

export type ChoiceRespondCommandStatus =
  | "submitting"
  | "resolving"
  | "delivered"
  | "expired"
  | "error";

export type ChoiceRespondCommandState = {
  commandId: string;
  status: ChoiceRespondCommandStatus;
  errorCode?: string;
  errorMessage?: string;
};

/**
 * P2-e（REQ-CRO-11）：Coding 页 choice 就地作答命令台账——抽取自
 * `useCockpitChoiceRespond` 的命令谱系：同 choice 同 command 重试（服务端按
 * command_id 幂等/异 payload 409）、202 后同 command GET 复查、expired（410）
 * 不路由新 run。提交恒走 REST `postCodingChoiceResponse`，答案恒按题号逐题
 * 组装（payload 缺逐题 answers 时回落单一 default 题）。
 *
 * 调用方纪律：`expected_run_id` 缺失（无唯一活跃 run）时不得调用本命令——
 * X1 裁定不发请求、只显示人话提示、无「恢复运行」按钮。
 */
export function useChoiceRespondCommand(
  address: CodingAttemptAddress,
  onStateChange?: (
    choiceId: string,
    state: ChoiceRespondCommandState,
    request: ChoiceResponseRequest,
  ) => void,
) {
  const [choiceCommands, setChoiceCommands] = useState<
    Record<string, ChoiceRespondCommandState>
  >({});
  const onStateChangeRef = useRef(onStateChange);
  onStateChangeRef.current = onStateChange;

  const respondChoice = useCallback(
    (choiceId: string, expectedRunId: string, payload: ChoiceResponsePayload) => {
      const prior = choiceCommands[choiceId];
      // 同命令重试纪律：错误/冲突后重试复用首个 command_id 与同一份 answers
      //（服务端按 command_id 幂等/异 payload 409）；新作答才生成新 id。
      const commandId = prior?.commandId ?? newCommandId();
      const answers: ChoiceAnswer[] =
        payload.answers && payload.answers.length > 0
          ? payload.answers.map((answer) => ({
              question_id: answer.question_id,
              selected_option_ids: [...answer.selected_option_ids],
              free_text: answer.free_text ?? null,
            }))
          : [
              {
                question_id: "default",
                selected_option_ids: payload.selected_option_ids,
                free_text: payload.free_text,
              },
            ];
      const request: ChoiceResponseRequest = {
        command_id: commandId,
        expected_run_id: expectedRunId,
        answers,
      };
      const applyStatus = (next: {
        status: ChoiceRespondCommandStatus;
        commandId?: string;
        errorCode?: string;
        errorMessage?: string;
      }) => {
        const record: ChoiceRespondCommandState = {
          commandId: next.commandId ?? commandId,
          status: next.status,
          errorCode: next.errorCode,
          errorMessage: next.errorMessage,
        };
        setChoiceCommands((previous) => {
          const current = previous[choiceId];
          // 竞态守卫：只更新仍属本命令谱系的台账（本地生成 id 与服务端回执
          // 回显 id 同谱；用户已换新命令时不回写旧状态）。
          if (
            current !== undefined &&
            current.commandId !== commandId &&
            current.commandId !== record.commandId
          ) {
            return previous;
          }
          return { ...previous, [choiceId]: record };
        });
        onStateChangeRef.current?.(choiceId, record, request);
      };
      const mapState = (
        state: ChoiceReplyStatus["state"],
      ): ChoiceRespondCommandStatus => {
        if (state === "delivered") {
          return "delivered";
        }
        if (state === "expired") {
          return "expired";
        }
        if (state === "rejected") {
          return "error";
        }
        return state === "submitting" ? "submitting" : "resolving";
      };
      const recheck = (attemptIndex: number, recheckCommandId: string) => {
        const delays = [250, 500, 1000, 2000, 4000];
        const delay = delays[attemptIndex];
        if (delay === undefined) {
          return;
        }
        window.setTimeout(() => {
          void getCodingChoiceResponseStatus(address, choiceId, recheckCommandId)
            .then((status) => {
              const next = mapState(status.state);
              applyStatus({ status: next, commandId: status.command_id });
              if (next === "submitting" || next === "resolving") {
                recheck(attemptIndex + 1, status.command_id);
              }
            })
            .catch(() => {
              // 复查失败保留当前状态面；服务端 pending 广播是权威收敛。
            });
        }, delay);
      };
      applyStatus({ status: "submitting" });
      void postCodingChoiceResponse(address, choiceId, request)
        .then((status) => {
          const next = mapState(status.state);
          applyStatus({ status: next, commandId: status.command_id });
          if (next === "submitting" || next === "resolving") {
            recheck(0, status.command_id);
          }
        })
        .catch((error: unknown) => {
          const code =
            typeof error === "object" && error !== null && "code" in error
              ? String(error.code)
              : "choice_response_failed";
          const message =
            error instanceof Error && error.message !== "" ? error.message : "应答请求失败";
          applyStatus({
            status:
              code === "workspace_choice_expired" || code === "coding_choice_expired"
                ? "expired"
                : "error",
            errorCode: code,
            errorMessage: message,
          });
        });
    },
    [choiceCommands, address],
  );
  return { choiceCommands, respondChoice };
}

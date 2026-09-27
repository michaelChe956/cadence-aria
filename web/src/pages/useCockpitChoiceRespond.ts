import { useCallback, useState } from "react";
import type { ChoiceAnswer, ChoiceReplyStatus } from "../api/types";
import {
  getCodingChoiceResponseStatus,
  getWorkspaceChoiceResponseStatus,
  postCodingChoiceResponse,
  postWorkspaceChoiceResponse,
} from "../api/client";
import { newCommandId } from "../hooks/useWorkspaceWs";
import type { ChoiceResponsePayload } from "../state/chat-entries";
import type { CockpitInboxItem } from "../state/workspace-cockpit-projection";

export function useCockpitChoiceRespond(sessionId: string) {
  // P0 1.3（REQ-WIGA-05）Task 11：choice 就地作答命令台账——key
  // `${scopeSessionId}:${choiceId}`；202（submitting/resolving）保卡复查、
  // delivered 移除卡片、expired（410）不路由新 run、error（409 等）保留
  // 同 command 重试。服务端 pending 广播收敛是权威，本台账只补 202→回执
  // 窗口的状态面。
  const [choiceCommands, setChoiceCommands] = useState<
    Record<
      string,
      {
        commandId: string;
        status: "submitting" | "resolving" | "delivered" | "expired" | "error";
        errorCode?: string;
        errorMessage?: string;
      }
    >
  >({});
  const handleChoiceRespond = useCallback(
    (item: CockpitInboxItem, payload: ChoiceResponsePayload) => {
      const choice = item.choice;
      if (choice === null || choice.expectedRunId === null) {
        return;
      }
      const scopeId = choice.sessionId ?? sessionId;
      const key = `${scopeId}:${choice.choiceId}`;
      const prior = choiceCommands[key];
      // 同命令重试纪律：错误/冲突后重试复用首个 command_id 与同一份 answers
      //（服务端按 (command_id) 幂等/异 payload 409）；新作答才生成新 id。
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
      const request = {
        command_id: commandId,
        expected_run_id: choice.expectedRunId,
        answers,
      };
      const applyStatus = (
        next: {
          status: "submitting" | "resolving" | "delivered" | "expired" | "error";
          commandId?: string;
          errorCode?: string;
          errorMessage?: string;
        },
      ) => {
        setChoiceCommands((previous) => {
          const current = previous[key];
          // 竞态守卫：只更新仍属本命令谱系的台账（本地生成 id 与服务端回执
          // 回显 id 同谱；用户已换新命令时不回写旧状态）。
          const lineageId = next.commandId ?? commandId;
          if (
            current !== undefined &&
            current.commandId !== commandId &&
            current.commandId !== lineageId
          ) {
            return previous;
          }
          return {
            ...previous,
            [key]: {
              commandId: next.commandId ?? commandId,
              status: next.status,
              errorCode: next.errorCode,
              errorMessage: next.errorMessage,
            },
          };
        });
      };
      const mapState = (
        state: ChoiceReplyStatus["state"],
      ): "submitting" | "resolving" | "delivered" | "expired" | "error" => {
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
          void (choice.source === "coding" && choice.attemptAddress
            ? getCodingChoiceResponseStatus(choice.attemptAddress, choice.choiceId, recheckCommandId)
            : getWorkspaceChoiceResponseStatus(scopeId, choice.choiceId, recheckCommandId)
          )
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
      void (choice.source === "coding" && choice.attemptAddress
        ? postCodingChoiceResponse(choice.attemptAddress, choice.choiceId, request)
        : postWorkspaceChoiceResponse(scopeId, choice.choiceId, request)
      )
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
    [choiceCommands, sessionId],
  );
  return { choiceCommands, handleChoiceRespond };
}

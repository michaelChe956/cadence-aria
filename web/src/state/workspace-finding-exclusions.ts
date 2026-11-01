import type { RevokeFindingExclusionsResult, WorkspaceHumanAction } from "../api/types";

/** REQ-WSC-13/14（k3 P0-2C 前端面）：撤销排除动作的窄化类型（不带 expected_gate_id——会话级事实）。 */
export type RevokeFindingExclusionsAction = Extract<
  WorkspaceHumanAction,
  { type: "revoke_finding_exclusions" }
>;

/**
 * 撤销排除的调用结果（never-throw）：ok=false 时 code/message 为服务端或
 * 网络层的如实原因，供门卡就地呈现——失败不冒充成功、不静默。
 */
export type RevokeFindingExclusionsOutcome =
  | { ok: true; result: RevokeFindingExclusionsResult }
  | { ok: false; code: string; message: string };

/**
 * REQ-WSC-13/14（k3 P0-2C）：POST /api/workspace-sessions/{session_id}/human-actions
 * 的 `revoke_finding_exclusions` 专用出站。
 *
 * 不复用 `postWorkspaceHumanAction`：该函数按 `gate_id` 校验回执形态，而撤销
 * 排除的回执（`RevokeFindingExclusionsResult`）是会话级事实、无 gate_id。失败
 * （422 `human_action_rejected` 等）不抛异常，原样带回 wire 的 code/message
 * （与 ApiError 错误体同形态），由门卡按行呈现。
 *
 * 幂等由服务端按 key 清单承担（无匹配 key 零写）——`finding_keys` 传空数组
 * 即零副作用只读：回执的 `remaining_finding_keys` 就是该会话 durable 已排除
 * 清单全量，页面刷新后用它恢复「已排除」呈现（不再只在本地）。
 */
export async function postRevokeFindingExclusionsAction(
  sessionId: string,
  action: RevokeFindingExclusionsAction,
): Promise<RevokeFindingExclusionsOutcome> {
  try {
    const response = await fetch(
      `/api/workspace-sessions/${encodeURIComponent(sessionId)}/human-actions`,
      {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(action),
      },
    );
    const body: unknown = await response.json().catch(() => null);
    if (response.ok && isRevokeFindingExclusionsResult(body)) {
      return { ok: true, result: body };
    }
    return {
      ok: false,
      code: errorCodeFromBody(body) ?? `revoke_finding_exclusions_http_${response.status}`,
      message:
        errorMessageFromBody(body) ?? `撤销排除被服务端拒绝（HTTP ${response.status}）`,
    };
  } catch (error) {
    return {
      ok: false,
      code: "revoke_finding_exclusions_network",
      message:
        error instanceof Error && error.message !== ""
          ? error.message
          : "撤销排除请求未送达（网络错误）",
    };
  }
}

function isRevokeFindingExclusionsResult(
  value: unknown,
): value is RevokeFindingExclusionsResult {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    typeof record.command_id === "string" &&
    record.state === "accepted" &&
    Array.isArray(record.remaining_finding_keys) &&
    record.remaining_finding_keys.every((key) => typeof key === "string")
  );
}

function errorCodeFromBody(body: unknown): string | null {
  return typeof body === "object" && body !== null && "code" in body
    ? String((body as Record<string, unknown>).code)
    : null;
}

function errorMessageFromBody(body: unknown): string | null {
  return typeof body === "object" && body !== null && "message" in body
    ? String((body as Record<string, unknown>).message)
    : null;
}

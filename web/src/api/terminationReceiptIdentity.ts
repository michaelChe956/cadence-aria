// 2026-11-06 陈旧回执修复（r3）：从 client.ts 原样抽出的终止回执身份解析
// （large_file_guard 行数门禁：client.ts 登记上限 1313 行）。逻辑零改动，
// 仅函数搬移 + findWorkspaceSessionRecord 加 export 供 client.ts 复用。
import type { WorkspaceSessionSummary } from "./types";
import { getIssueLifecycle, listProductIssues, listProjects } from "./client";

/** observers 同源目录（projects → issues → lifecycle workspace_sessions）
 * 的只读会话记录定位；找不到/读取失败返回 null（调用方保持静默）。 */
export async function findWorkspaceSessionRecord(
  sessionId: string,
): Promise<{
  projectId: string;
  issueId: string;
  session: WorkspaceSessionSummary;
} | null> {
  try {
    const { projects } = await listProjects();
    for (const project of projects) {
      const listed = await listProductIssues(project.project_id);
      for (const issue of listed.issues) {
        const lifecycle = await getIssueLifecycle(issue.issue_id, project.project_id);
        const session = lifecycle.workspace_sessions.find(
          (candidate) => candidate.workspace_session_id === sessionId,
        );
        if (session !== undefined) {
          return { projectId: project.project_id, issueId: issue.issue_id, session };
        }
      }
    }
  } catch {
    return null;
  }
  return null;
}

/**
 * 2026-11-06 陈旧回执修复（REQ-WIGA-14）：workspace session →
 * (project_id, issue_id, plan_id) 只读身份解析——终止横幅的当前 plan 身份源。
 * plan_id 取 plan 会话记录的 entity_id：与 enrollment/终止响应的 plan_id
 * 同源（`issue_work_item_plan_auto_*`，后端 PreparedPlanIntent 同一派生），
 * 且**终止态仍可得**（终止 CAS 只写 enrollment.enabled/terminated_at，不清
 * 会话记录；停点形态投影（control）终止即清空，不得作横幅身份源）。
 * 找不到/非 plan 会话/记录缺 entity_id 返回 planId null（调用方
 * fail-closed，不猜身份）。
 */
export async function findWorkspaceSessionAutomationIdentity(
  sessionId: string,
): Promise<{ projectId: string; issueId: string; planId: string | null } | null> {
  const record = await findWorkspaceSessionRecord(sessionId);
  if (record === null) {
    return null;
  }
  const entityId = record.session.entity_id;
  return {
    projectId: record.projectId,
    issueId: record.issueId,
    planId:
      record.session.workspace_type === "work_item_plan" &&
      typeof entityId === "string" &&
      entityId !== ""
        ? entityId
        : null,
  };
}

import { jsonResponse } from "./IssueLifecycleWorkbench.test-data";
import type { LifecycleFetchOptions } from "./IssueLifecycleWorkbench.test-utils-aggregate";

// P1 WIGA Task 2：durable enrollment 状态——同键同 payload 幂等返回原记录，
// 异 payload 409 携带当前 revision；GET 返回当前记录或 null。
export function createAutomationEnrollmentRoutes(
  options?: LifecycleFetchOptions,
) {
  let automationEnrollment: Record<string, unknown> | null = null;
  let automationPutFailures = options?.automationPutFailures ?? 0;
  const automationTargetLogicalId =
    options?.automationTarget === undefined
      ? "00000000-0000-0000-0000-000000000001"
      : options.automationTarget;
  const automationRoleChainViolations = options?.automationRoleChainViolations;
  return (
    url: string,
    init?: RequestInit,
  ): Response | Promise<Response> | undefined => {
    const automationEnrollmentMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/automation-enrollment$/,
    );
    if (automationEnrollmentMatch) {
      const [, projectId, issueId] = automationEnrollmentMatch;
      if (init?.method === "PUT") {
        if (automationPutFailures > 0) {
          automationPutFailures -= 1;
          return new Response(
            JSON.stringify({
              code: "automation_put_failed",
              message: "automation enrollment put failed",
              details: {},
            }),
            { status: 500 },
          );
        }
        if (options?.automationEnrollmentConflict) {
          return new Response(
            JSON.stringify({
              code: "automation_enrollment_conflict",
              message: "automation enrollment revision conflict",
              details: { current_revision: 1 },
            }),
            { status: 409 },
          );
        }
        const payload = JSON.parse(String(init.body)) as {
          expected_revision: number | null;
          command: {
            type: string;
            selection_key: string;
            source: Record<string, unknown>;
            options: Record<string, unknown>;
            target?: {
              kind: string;
            } & Record<string, unknown>;
          };
        };
        const command = payload.command;
        if (command?.type !== "enable") {
          return new Response(
            JSON.stringify({
              code: "automation_unsupported",
              message: "only enable is supported in the lifecycle fixture",
              details: {},
            }),
            { status: 422 },
          );
        }
        if (automationRoleChainViolations) {
          // C5 Task 3：role-chain 预检 422——details.violations 逐角色列出。
          return new Response(
            JSON.stringify({
              code: "automation_role_chain_unsupported",
              message: "automation role chain is not supported for this carrier",
              details: { violations: automationRoleChainViolations },
            }),
            { status: 422 },
          );
        }
        if (!command.target) {
          return new Response(
            JSON.stringify({
              code: "automation_target_required",
              message: "enable command requires an enrollment target",
              details: {},
            }),
            { status: 422 },
          );
        }
        const frozenPayload = {
          source: command.source,
          options: command.options,
          target: command.target,
        };
        if (automationEnrollment) {
          const sameKey =
            automationEnrollment.selection_key === command.selection_key;
          const samePayload =
            JSON.stringify(automationEnrollment.frozen_payload) ===
            JSON.stringify(frozenPayload);
          if (sameKey && samePayload) {
            return jsonResponse(automationEnrollment);
          }
          return new Response(
            JSON.stringify({
              code: "automation_enrollment_conflict",
              message: "automation enrollment revision conflict",
              details: {
                current_revision: automationEnrollment.policy_revision,
              },
            }),
            { status: 409 },
          );
        }
        automationEnrollment = {
          enrollment_id: "enrollment_0001",
          selection_key: command.selection_key,
          project_id: projectId,
          issue_id: issueId,
          enabled: true,
          policy_revision: 1,
          source: command.source,
          options: command.options,
          target: command.target,
          prepare_intent_id: "prepare_intent_0001",
          plan_id: null,
          session_id: null,
          created_at: "2026-09-27T00:00:00Z",
          updated_at: "2026-09-27T00:00:00Z",
          frozen_payload: frozenPayload,
        };
        return jsonResponse(automationEnrollment);
      }
      return jsonResponse(automationEnrollment ?? null);
    }
    const automationTargetMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/automation-target/,
    );
    if (automationTargetMatch) {
      if (automationTargetLogicalId === null) {
        return new Response(
          JSON.stringify({
            code: "automation_enrollment_invalid_scope",
            message:
              "automation target requires exactly one logical repository",
            details: {},
          }),
          { status: 422 },
        );
      }
      const query = new URL(url, "http://localhost").searchParams;
      return jsonResponse({
        // C5 Task 1：响应只含 enrollment_target + resolved_options（不再
        // 携带顶层 logical_repository_id 冗余字段）。
        enrollment_target: {
          kind: "logical_codebase",
          logical_codebase_id: "11111111-1111-1111-1111-111111111111",
          logical_repository_id: automationTargetLogicalId,
        },
        resolved_options: {
          author_provider: query.get("author_provider") ?? "codex",
          reviewer_provider: query.get("reviewer_provider") ?? "claude_code",
          review_rounds: 1,
          superpowers_enabled: true,
          openspec_enabled: true,
          plan_options: {
            include_integration_tests:
              query.get("include_integration_tests") !== "false",
            include_e2e_tests: query.get("include_e2e_tests") === "true",
            force_frontend_backend_split:
              query.get("force_frontend_backend_split") === "true",
            require_execution_plan_confirm:
              query.get("require_execution_plan_confirm") === "true",
          },
        },
      });
    }
    return undefined;
  };
}

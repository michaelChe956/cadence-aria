import type {
  AggregateIndexActiveResponse,
  AggregateInitializationOperationSnapshot,
  CodingAttempt,
  PointerPublicationDto,
  RegistrationBatchDto,
  RegistrationPreflightResponse,
} from "../../api/types";
import {
  jsonResponse,
  projectRecord,
  repositoryRecord,
} from "./IssueLifecycleWorkbench.test-data";

export type LifecycleFetchOptions = {
  duplicateCardIds?: boolean;
  sharedLifecycleIdsAcrossIssues?: boolean;
  emptyLifecycle?: boolean;
  invalidLifecycle?: boolean;
  confirmedWorkItem?: boolean;
  issueDescription?: string;
  issueTitles?: string[];
  issueTitlesByProject?: Record<string, string>;
  issuesByProject?: Record<string, Array<Record<string, unknown>>>;
  projects?: Array<ReturnType<typeof projectRecord>>;
  repositoriesByProject?: Record<string, ReturnType<typeof repositoryRecord>[]>;
  projectResponses?: Array<Promise<Response>>;
  splitWorkItems?: boolean;
  workItemPlans?: unknown[];
  skippedIntegrationRisk?: boolean;
  codingAttempts?: CodingAttempt[];
  pointerPublications?: PointerPublicationDto[];
  aggregateIndex?: AggregateIndexActiveResponse;
  aggregateIndexRebuild?: AggregateIndexActiveResponse;
  aggregateInitializationStart?: AggregateInitializationOperationSnapshot;
  aggregateInitializationSnapshots?: AggregateInitializationOperationSnapshot[];
  aggregateInitializationCancel?: AggregateInitializationOperationSnapshot;
  registrationPreflight?: RegistrationPreflightResponse;
  registrationSubmit?: RegistrationBatchDto;
  registrationResume?: RegistrationBatchDto;
  registrationCancel?: RegistrationBatchDto;
  logicalCodebases?: Array<{
    id: string;
    name: string;
    member_count?: number;
  }>;
  logicalCodebaseMembers?: Array<{
    logical_repository_id: string;
    alias: string;
    status: "active" | "removed" | "tombstoned";
    physical_repository_id?: string | null;
  }>;
  // R8：按 lc_id 区分的成员列表（多 LC 分区/primary 选择测试）。
  logicalCodebaseMembersByLc?: Record<
    string,
    Array<{
      logical_repository_id: string;
      alias: string;
      status: "active" | "removed" | "tombstoned";
      physical_repository_id?: string | null;
    }>
  >;
  // REQ-TGT-05：后端 work_item_repository_groups 的 mock 值；缺省空数组（单仓扁平兼容）。
  workItemRepositoryGroups?: Array<Record<string, unknown>>;
  // Task 6：构造“有 story 无 design”的阶段 fixture（默认阶段应落在 design）。
  emptyDesignSpecs?: boolean;
  // F-29：初始 story 处于 draft（confirm 后 fake server 会把 durable 投影落成
  // confirmed，供 invalidation 刷新测试观察 draft→confirmed 的卡片状态迁移）。
  storyDraftInitially?: boolean;
  // P1 WIGA Task 2：初始 design 处于 draft——自动化模式选择只对已确认 Design 开放。
  designDraftInitially?: boolean;
  // P1 WIGA Task 2：automation-target GET 的 mock 逻辑仓 id；null 表示目标不可用(422)。
  automationTarget?: string | null;
  // C5 Task 3：PUT enrollment 一律 422 automation_role_chain_unsupported，
  // details.violations 逐角色携带该值（验证弹窗逐角色渲染）。
  automationRoleChainViolations?: Array<{
    role: string;
    provider: string;
    reason_code: string;
  }>;
  // P1 WIGA Task 2：PUT enrollment 前 N 次返回 500（验证同 selection_key 重试）。
  automationPutFailures?: number;
  // P1 WIGA Task 2：PUT enrollment 一律 409（验证冲突留弹窗、不自动 Disable）。
  automationEnrollmentConflict?: boolean;
};

export function aggregateInitializationOperation(
  status: AggregateInitializationOperationSnapshot["status"],
  overrides: Partial<AggregateInitializationOperationSnapshot> = {},
): AggregateInitializationOperationSnapshot {
  const completed = status === "completed";
  const failed = status === "failed";
  const running = status === "running";
  return {
    operation_id: "aggregate_initialization_0001",
    project_id: "project_0001",
    status,
    profile: null,
    steps: [
      {
        step_id: "machine_skills",
        status: completed || running || failed ? "completed" : "pending",
      },
      {
        step_id: "aggregate_preflight",
        status: failed ? "failed" : completed || running ? "completed" : "pending",
      },
      {
        step_id: "pre_check",
        status: completed || running ? "running" : "pending",
      },
      { step_id: "rule_and_mcp_config", status: completed ? "completed" : "pending" },
      { step_id: "openspec_and_examples", status: completed ? "completed" : "pending" },
    ],
    current_step: completed ? null : running || failed ? "pre_check" : "machine_skills",
    failed_step: failed ? "aggregate_preflight" : null,
    member_projections: [],
    cancellation: null,
    error:
      status === "failed"
        ? { code: "aggregate_initialization_failed", message: "初始化命令执行失败", details: {} }
        : null,
    created_at: "2026-08-18T00:00:00Z",
    updated_at: "2026-08-18T00:00:01Z",
    completed_at: completed ? "2026-08-18T00:01:00Z" : null,
    ...overrides,
  };
}

// aggregate-index/initialization 路由段（自 IssueLifecycleWorkbench.test-utils.ts
// lifecycleFetch 逐行移出；命中返回 Response，未命中返回 undefined 交后续路由）。
export function createAggregateInitializationRoutes(
  options?: LifecycleFetchOptions,
) {
  let aggregateInitializationGetCount = 0;
  return (url: string, init?: RequestInit): Promise<Response> | undefined => {
    const aggregateIndexMatch = url.match(
      /^\/api\/projects\/([^/]+)\/logical-codebases\/([^/]+)\/aggregate-indexes\/(active|rebuild)$/,
    );
    if (aggregateIndexMatch) {
      const action = aggregateIndexMatch[3];
      return jsonResponse(
        action === "rebuild"
          ? (options?.aggregateIndexRebuild ?? options?.aggregateIndex ?? {
              state: "active",
              revision: 1,
              indexed_at: "2026-08-18T00:00:00Z",
              warning: null,
            })
          : (options?.aggregateIndex ?? {
              state: "missing",
              revision: null,
              indexed_at: null,
              warning: null,
            }),
      );
    }
    const aggregateInitializationStartMatch = url.match(
      /^\/api\/projects\/([^/]+)\/logical-codebases\/([^/]+)\/initializations$/,
    );
    if (aggregateInitializationStartMatch && init?.method === "POST") {
      return jsonResponse(
        options?.aggregateInitializationStart ??
          options?.aggregateInitializationSnapshots?.[0] ??
          aggregateInitializationOperation("created"),
      );
    }
    const aggregateInitializationCancelMatch = url.match(
      /^\/api\/projects\/([^/]+)\/logical-codebases\/([^/]+)\/initializations\/([^/]+)\/cancel$/,
    );
    if (aggregateInitializationCancelMatch && init?.method === "POST") {
      return jsonResponse(
        options?.aggregateInitializationCancel ??
          aggregateInitializationOperation("cancelled"),
      );
    }
    const aggregateInitializationGetMatch = url.match(
      /^\/api\/projects\/([^/]+)\/logical-codebases\/([^/]+)\/initializations\/([^/]+)$/,
    );
    if (aggregateInitializationGetMatch && init?.method !== "POST") {
      const snapshots = options?.aggregateInitializationSnapshots ?? [];
      const snapshot =
        snapshots[aggregateInitializationGetCount] ??
        snapshots[snapshots.length - 1] ??
        aggregateInitializationOperation("completed");
      aggregateInitializationGetCount += 1;
      return jsonResponse(snapshot);
    }
    return undefined;
  };
}

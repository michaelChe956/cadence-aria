// v1.3 §4/R2：统一 codebases 混合列表 + 逻辑代码库 CRUD DTO。
export type CodebaseSummaryKind = "single_repo" | "logical";

export type CodebaseSummaryDto = {
  id: string;
  name: string;
  kind: CodebaseSummaryKind;
  repository_id: string | null;
  logical_codebase_id: string | null;
  member_count: number | null;
};

export type CodebaseListResponse = {
  codebases: CodebaseSummaryDto[];
};

/**
 * 2026-10-29：LC 列表条目（`GET /logical-codebases?include_deleted=true`）。
 * 墓碑 LC 带 `deleted=true` + `deleted_at`，供「已删除」分区渲染。
 */
export type LogicalCodebaseListEntryDto = {
  id: string;
  name: string;
  aggregate_root: string;
  created_at: string;
  deleted: boolean;
  deleted_at: string | null;
};

export type LogicalCodebaseListResponse = {
  logical_codebases: LogicalCodebaseListEntryDto[];
};

export type CreateLogicalCodebaseRequest = {
  name: string;
  aggregate_root: string;
};

export type LogicalCodebaseDto = {
  id: string;
  name: string;
  aggregate_root: string;
  created_at: string;
};

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

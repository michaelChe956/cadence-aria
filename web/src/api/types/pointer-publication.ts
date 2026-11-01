// 后端 PointerPublication / PointerPublicationEntry 的 serde snake_case 投影。
// 对应 src/product/logical_codebase/pointer_publication.rs 与
// src/web/handlers/pointer_publication.rs 的响应 DTO。
export type PointerPublicationBatchKind = "full" | "incremental";

export type PointerPublicationStatus =
  | "in_progress"
  | "completed_all"
  | "completed_partial"
  | "revoked";

export type PointerPublicationEntryState =
  | "pending"
  | "skipped"
  | "conflict"
  | "committed"
  | "pushed"
  | "review_created"
  | "failed"
  | "revoked";

export type PointerPublicationEntryDto = {
  member_repo_id: string;
  state: PointerPublicationEntryState;
  branch_name: string | null;
  commit_sha: string | null;
  push_error: string | null;
  conflict_detail: string | null;
};

export type PointerPublicationDto = {
  id: string;
  project_id: string;
  logical_codebase_id: string;
  batch_kind: PointerPublicationBatchKind;
  entries: PointerPublicationEntryDto[];
  status: PointerPublicationStatus;
  created_at: string;
  updated_at: string;
};

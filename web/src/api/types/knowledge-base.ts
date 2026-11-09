// 后端知识库模块的 serde snake_case 投影。
// 对应 src/product/logical_codebase/knowledge_base/* 与
// src/web/handlers/knowledge_base.rs 的响应 DTO
// （openspec add-logical-codebase-knowledge-base 计划 Task 7 端点表）。
import type { ApiError } from "./common";
import type { AggregateProviderSelectionDto } from "./aggregate-initialization";

/** GET /status 响应（manifest 只读投影；未初始化时 exists=false）。 */
export type KnowledgeBaseStatusDto = {
  exists: boolean;
  schema_version: string | null;
  initialization_status: string | null;
  global_validation: string | null;
  completed_stages: string[];
  skipped_stages: { stage: string; reason: string }[];
  skills_available: boolean;
};

/** GET /files 的文件树节点。 */
export type KnowledgeBaseFileNodeDto = {
  path: string;
  is_dir: boolean;
  size: number;
  editable: boolean;
};

/** GET /file 响应（content 为原文，digest 为服务端内容摘要）。 */
export type KnowledgeBaseFileContentDto = {
  path: string;
  content: string;
  digest: string;
};

/** 变更包门禁违规项（ChangePackageViolation 投影）。 */
export type KnowledgeBaseChangePackageViolationDto = {
  code: string;
  document: string | null;
  field: string | null;
  detail: string;
};

/** 变更包字段填充状态（ChangePackageFieldStatus 投影）。 */
export type KnowledgeBaseChangePackageFieldStatusDto = {
  document: string;
  field: string;
  filled: boolean;
};

/** GET /change-packages/lint 响应（ChangePackageLint 投影）。 */
export type KnowledgeBaseChangePackageLintDto = {
  violations: KnowledgeBaseChangePackageViolationDto[];
  fields: KnowledgeBaseChangePackageFieldStatusDto[];
};

/** GET /change-packages 的变更包条目。 */
export type KnowledgeBaseChangePackageDto = {
  id: string;
  path: string;
  violation_count: number;
  violations: KnowledgeBaseChangePackageViolationDto[];
};

/** 步骤状态复用聚合初始化四态语义（不含 skipped——知识库只有一个 provider 步骤）。 */
export type KnowledgeBaseOperationStepStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed";

/** operation 步骤记录（KnowledgeBaseStepRecord 投影）。 */
export type KnowledgeBaseOperationStepDto = {
  step_id: string;
  status: KnowledgeBaseOperationStepStatus;
  started_at: string | null;
  completed_at: string | null;
  output_summary: string | null;
};

/** operation 创建期冻结的输入（KnowledgeBaseOperationInput 投影）。 */
export type KnowledgeBaseOperationInputDto = {
  idempotency_key: string;
  provider: string;
  package_path: string | null;
  package_id: string | null;
};

/** 知识库 operation 投影（KnowledgeBaseOperation 投影）。 */
export type KnowledgeBaseOperationSnapshotDto = {
  operation_id: string;
  project_id: string;
  logical_codebase_id: string;
  kind: "bootstrap" | "update";
  input: KnowledgeBaseOperationInputDto;
  status: "running" | "completed" | "failed" | "cancelled";
  steps: KnowledgeBaseOperationStepDto[];
  provider_selection: AggregateProviderSelectionDto | null;
  error: ApiError | null;
  created_at: string;
  started_at: string | null;
  completed_at: string | null;
};

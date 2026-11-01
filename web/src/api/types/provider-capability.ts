// add-provider-revalidate-probe：LC provider capability 只读投影与显式
// 重验证的 serde snake_case DTO。对应 src/web/handlers/
// logical_codebase_capabilities.rs 的 GET/POST 响应面。
export type ProviderCapabilityType =
  | "claude_code"
  | "codex"
  | "pi"
  | "kimi_code";

export type ProviderCapabilityEvidenceStatus =
  | "confirmed"
  | "denied"
  | "unknown";

export type ProviderCapabilityActionName =
  | "coding_target_write"
  | "planning_read_only"
  | "review_read_only";

/** durable action 分格行：action × launch/resume/write_boundary 证据。 */
export type ProviderCapabilityActionRowDto = {
  action: ProviderCapabilityActionName;
  launch: ProviderCapabilityEvidenceStatus;
  resume: ProviderCapabilityEvidenceStatus;
  write_boundary: ProviderCapabilityEvidenceStatus;
  evidence_ref: string;
};

export type ProviderCapabilityProviderSnapshotDto = {
  provider_type: ProviderCapabilityType;
  cli_program: string;
  version: string | null;
  probed_at: string | null;
  probe_artifact_ref: string | null;
  rows: ProviderCapabilityActionRowDto[];
};

export type ProviderCapabilitiesResponse = {
  providers: ProviderCapabilityProviderSnapshotDto[];
};

/** POST capability-revalidate 结果：全行同版本已验证秒回不重跑，或本次
 * 真实探针完成并原子导入（失败走 ApiError：unsupported=422 /
 * probe_failed=503，detail 携带 action 与原始错误）。 */
export type CapabilityRevalidateResult =
  | {
      provider_type: ProviderCapabilityType;
      outcome: "already_confirmed";
      version: string;
    }
  | {
      provider_type: ProviderCapabilityType;
      outcome: "revalidated";
      version: string;
      imported_actions: ProviderCapabilityActionName[];
    };

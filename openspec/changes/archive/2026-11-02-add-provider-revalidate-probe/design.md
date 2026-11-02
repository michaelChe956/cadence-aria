# Design

## 决策记录（契约四开放点）

1. **动作挂哪**：新端点，不扩 `LogicalCodebaseBootstrapStep`。
   - `POST /api/projects/{project_id}/logical-codebases/{lc_id}/capability-revalidate`，body `{ "provider_type": "claude_code" | "codex" | "pi" | "kimi_code" }`。
   - `GET /api/projects/{project_id}/logical-codebases/{lc_id}/capabilities`（只读投影）。
   - 理由：capability 的键是 provider 而非五步链的 step；往 `dispatch_action` 加分支必须先加第六个 step，破坏五步契约与 `LogicalCodebaseBootstrapStep::V1` 冻结序。既有 bootstrap Revalidate 按钮继续服务 step 面（policy 漂移等）；capability 面是独立动作端点，语义同为「核验」。

2. **fixture 在 server 进程怎么建**：每次点击建一个 `tempfile::TempDir`（/tmp 族）作为 `run_cli_boundary_probe` 的 `base`（root/member/member_b/home/target 全在 base 内、按调用隔离，天然满足 /tmp 强制规则）；`evidence_root` 落 LC 子树 `lc_scope_root/capability-evidence/boundary`（durable、可审计，与 capabilities.json 同作用域）。用后 TempDir 自动清理；evidence 持久保留。

3. **同步还是 detached**：同步 await（探针本体全异步，直接在 handler 内 await），先例是 G3 aggregate index rebuild 分钟级同步动作（spawn_blocking 同步执行 CodeGraph CLI）。不做 detached 任务状态机——scope 纪律「只加一条」；幂等由 fast-path 语义承担（全 Confirmed 秒回），无需 command 台账。

4. **多 provider 还是逐家**：逐家（端点 per-provider）。一次点击 = 一家 provider 的完整三 action 面（与矩阵 harness 同覆盖：Coding/Planning/Review + resume 面），避免跨家批量引入额外编排。

## 结构

- 产品层新文件 `src/product/logical_codebase/provider_capability_revalidate.rs`：
  - `provider_probe_channel(provider) -> Option<(&'static str /*cli_program*/, ResumeChannelKind)>`：四家冻结映射 + Fake None（产品侧与矩阵 harness 同口径）。
  - `ProviderCapabilityRevalidateService`：构造持 `ProductAppPaths`；可注入两个 seam（仅测试用）：
    - `version_source: Arc<dyn Fn(&str) -> Result<String, ProviderBoundaryError> + Send + Sync>`（默认 `probe_cli_version`）；
    - `probe_runner: Arc<dyn Fn(ProbeInvocation) -> BoxFuture<Result<CliBoundaryProbeOutcome, ProviderBoundaryError>> + Send + Sync>`（默认真实 `run_cli_boundary_probe`）。
  - `revalidate(project_id, lc_id, provider) -> Result<RevalidateOutcome, RevalidateError>`：
    1. 通道解析：None → `Unsupported`（稳定码 `provider_capability_probe_unsupported`）。
    2. 版本探测（一次 `cli --version`）→ 与 durable 记录比对；三 action 行全 Confirmed@当前版本 → `AlreadyConfirmed { version }`，零探针。
    3. 否则对缺失/漂移 action 逐个 `run_cli_boundary_probe(provider, cli, action, base, evidence_root, label, Some(ResumeProbeSpec::new(kind, REVALIDATE_PROMPT)))`；成功即 `record_verified_probe`（`ProviderCapabilityProbeService::with_durable_writer(ProviderCapabilityStore::for_lc(...))`，与 gateway 读子树同构——由同一 `lc_scope_root` 派生保证）。
    4. 任一失败 → `ProbeFailed { action, detail }`（已导入的成功行保留，durable 如实）。
  - `capability_snapshot(project_id, lc_id)`：只读投影四家行。
- Web 层 `src/web/handlers/logical_codebase_capabilities.rs`：GET/POST handler + DTO；`WebAppState` 增 `capability_revalidate_runner: Option<...>` 注入口（默认 None=生产真实 seam；it_web 注入 fake runner/version source 断言 HTTP 面）。
- 前端：`web/src/api/provider-capability.ts`（GET/POST 客户端）+ `web/src/components/lifecycle/ProviderCapabilityCard.tsx`（四家行 + 状态徽标 + 核验按钮 + 结果回显），挂入 `LogicalCodebaseManagementPanel`。

## 错误面（稳定码）

- `provider_capability_probe_unsupported`（422）：Fake/未知 provider。
- `provider_capability_probe_failed`（runtime）：CLI 缺失/版本门漂移/探针失败关闭/导入被拒；detail 携带 action 与原始错误。
- store 错误走既有 `product_store_api_error`。

## 测试策略

- 产品层单测（新文件内 `mod tests`，注入 seam）：
  1. 全 Confirmed@同版本 → AlreadyConfirmed，probe_runner 零调用；
  2. Unknown → 逐 action 探测导入，capabilities.json 三行 Confirmed、version 钉定；
  3. 版本漂移 → 仅漂移行重探；
  4. 探针失败 → 错误携带 action+detail，该 action 无 durable 写入；先成功行保留；
  5. Fake/未支持 → Unsupported 稳定码。
- it_web（新 `tests/it_web/web_lc_capability_revalidate.rs`，沿 `web_lc_bootstrap_repair.rs` 模式）：GET 只读投影；POST unsupported；注入 runner → revalidated + durable 断言 + 再点秒回 already_confirmed；注入失败 runner → 稳定错误码。
- 前端：`ProviderCapabilityCard.test.tsx`（mock api：状态渲染、点击回显、错误如实展示）。

# Tasks

## 1. 后端：产品层 revalidate 服务（TDD 先红）

- [x] 1.1 写产品层失败测试（`provider_capability_revalidate.rs` 内 `mod tests`，注入 version_source/probe_runner seam）：全 Confirmed 秒回不探测；Unknown 逐 action 探测+导入 durable Confirmed；版本漂移仅补漂移行；探针失败如实错误且该行不落盘；Fake/未支持稳定码。运行确认红（骨架期 7/8 失败实证）。
- [x] 1.2 实现 `provider_probe_channel` 映射 + `ProviderCapabilityRevalidateService`（fast-path 判定、/tmp uuid base 目录、LC 子树 evidence_root、`run_cli_boundary_probe` 默认 runner、`record_verified_probe` 导入、稳定错误面）。运行至绿（8/8）。

## 2. 后端：HTTP 面（TDD 先红）

- [x] 2.1 写 it_web 失败测试（`tests/it_web/web_lc_capability_revalidate.rs`）：GET capabilities 只读投影 Unknown；POST fake → 422 unsupported；seed 全 Confirmed + 注入版本源 → POST already_confirmed 秒回（runner 被触发即 panic）；注入失败 runner → 稳定错误码 + durable 不变；未知 LC 404。运行确认红（注：探针成功导入→revalidated 的 HTTP 快乐路径留在产品层 1.1 覆盖——it_web 侧 `ProviderPolicyProjection::new`/`ProviderBoundaryEvidence::new` 为 pub(crate) 冻结可见性，无法构造三方一致材料包，不为此放宽设计冻结）。
- [x] 2.2 实现 handler `src/web/handlers/logical_codebase_capabilities.rs`（GET/POST + DTO）、`WebAppState` 注入口、`app.rs` 路由注册、error.rs 稳定码映射（unsupported=422 / probe_failed=503）。运行至绿（6/6）。

## 3. 前端：capability 卡（TDD 先红）

- [x] 3.1 写前端失败测试 `ProviderCapabilityCard.test.tsx`：四家状态渲染；点击核验回显 already_confirmed/revalidated；失败如实展示错误。运行确认红（组件模块缺失 unresolved import，1 failed 实证）。
- [x] 3.2 实现 `web/src/api/provider-capability.ts` + `ProviderCapabilityCard.tsx`，挂入 `LogicalCodebaseManagementPanel`。运行至绿（6/6）。附带 E2E 接线：`e2e/spec/real/s4-story.spec.ts` S4 前置新增 `revalidateCodexCapability`（首击真实探针有界 330s / 第二击秒回 already_confirmed 幂等断言）。

## 4. 回归与门禁

- [x] 4.1 产品层回归：capability/bootstrap 相关测试全绿（`cargo test --lib logical_codebase`：430 passed / 0 failed / 4 ignored）。
- [ ] 4.2 it_web 回归：bootstrap repair / gateway multi-provider（不含 live 矩阵）/ 注册 API 等既有面全绿。（部分完成：新增面 6/6 绿；`web_lc_bootstrap_repair` a03/a04 在**不含本 change 改动的 HEAD 上同样失败**（`aggregate_pre_check_failed`，stash 归因实证）——既有问题待 controller 裁决归属，非本 change 引入。）
- [x] 4.3 前端回归：`web` 相关 vitest 全绿（208 文件 / 2023 测试全过；`tsc -b` 零错）。初跑 4 文件 13 测挂=capability 卡挂载后遇 mock 兜底 `{}` 无 providers 数组渲染崩溃，已修为如实报错不崩溃（归因：非预存，本 change 引入后修复）。
- [ ] 4.4 四门禁基线按名对照（controller 终审执行）。

# Tasks: fix-aggregate-lc-registration-gates

## 1. 登记门禁移除

- [x] 1.1 删除 `reject_owned_root_files` 与其调用；`validate_common` 并入 `validate`（`registration_preflight.inc.rs`）
- [x] 1.2 删除回执窄豁免通道：`validate_recipe_replay`、`proven_recipe_entry_digests`、`proven_digests_from_receipt`、`ReplayEvidenceBreak` 及其测试
- [x] 1.3 保留其余准入校验（非 git 根、成员边界、symlink 逃逸、嵌套 worktree、根重叠），既有用例全绿
- [x] 1.4 新增用例：根目录含 `AGENTS.md`+`CLAUDE.md`+`.claude/rules/language.md`+`.mcp.json`+`.gitignore` 时登记预检必须通过

## 2. 同一目录全局唯一

- [x] 2.1 `AggregateRootPreflight` 增加 `lc_id: Option<String>` 与 `for_lc(...)`；两个调用方（登记预检、聚合初始化复验）传具体 LC id
- [x] 2.2 重叠检查去掉"同项目跳过"，改为"跳过自己"；身份按权威存储键（legacy id / per-LC 目录名）比对
- [x] 2.3 枚举扩到所有项目的全部 LC 子树（legacy 迁移副本跳过），读取失败 fail-closed（实现见 `existing_lc_manifests`；用例覆盖同项目/跨项目 per-LC 子树 manifest——`aggregate_root_preflight_rejects_second_lc_on_same_root`；「legacy 迁移副本跳过」「读取失败 fail-closed」两分支的直接用例待补）
- [x] 2.4 软删墓碑期间仍占用该根（注释见 `reject_overlapping_manifest_root`；用例见 `aggregate_root_preflight_counts_manifest_less_and_tombstoned_lc_records`——软删后仍拒、purge 后才释放）
- [x] 2.5 用例：同项目不同 LC 同根 → 拒绝；跨项目同根 → 拒绝；同一 LC 复验 → 通过
- [x] 2.6 k3 审查 A1：HTTP 面拆分 `aggregate_root_overlap` 稳定码（409），不再折叠成 `aggregate_root_ownership_conflict`（旧臂保留兼容历史客户端）；用例：`aggregate_root_api_error_maps_overlap_to_its_own_stable_code`、`aggregate_root_api_error_keeps_legacy_ownership_conflict_arm`（lib）与 `registration_preflight_for_second_lc_on_occupied_root_returns_overlap_409`（HTTP 面，`details.reason` 含被占根与占用方）

## 3. 聚合初始化按需跳过 skills

- [x] 3.1 `AggregateInitializationStepStatus` 增加 `Skipped`；步骤记录增加 `skip_reason`
- [x] 3.2 判定：`AGENTS.md` 且 `.claude/rules/language.md` 存在 → 已初始化；不看 `<root>/.aria/`；IO 失败 fail-closed
- [x] 3.3 判定窗口：仅三步全 Pending 且无命令审计事实时可改判；已有命令事实即关窗
- [x] 3.4 判定与三步 `Skipped` 单次 durable 写落盘；operation 记录 `skills_initialization`（含两文件 sha256）
- [x] 3.5 回执双语义：跳过式 finalized（命令审计必须为空、重读盘复验摘要）与完整 recipe 互斥
- [x] 3.6 跳过时仍发布聚合政策：抽共用 `publish_and_verify_root_policy`；无通道 fail-closed
- [x] 3.7 用例：已初始化跳过四命令 + 政策发布成功；未初始化走完整五步；已有命令事实不改判；跳过无发布通道 fail-closed；续跑复用冻结判定

## 4. 聚合重新初始化入口

- [x] 4.1 `POST /api/projects/{project_id}/logical-codebases/{lc}/reinitializations` → `begin_reinitialization`（强制完整五步）
- [x] 4.2 与普通初始化共用通道 / 幂等 / 并发仲裁
- [x] 4.3 用例：已初始化根上重新初始化跑全部四命令

## 5. LC 生命周期

- [x] 5.1 `list_deleted` 报告墓碑 LC（含 `deleted_at`）；`GET /logical-codebases?include_deleted=true`
- [x] 5.2 `purge`：先软删后可用，删子树 → 清政策正文与成员指针 → 最后删墓碑；legacy 别名连旧根清
- [x] 5.3 `DELETE /logical-codebases/{lc}/purge`；`logical_codebase_not_tombstoned` → 409；跨 project → NotFound
- [x] 5.4 前端：`restoreLogicalCodebase` / `purgeLogicalCodebase` / `listLogicalCodebases(includeDeleted)`；「已删除」分区 + 恢复 / 彻底删除（二次确认写明不可恢复与清理范围）
- [x] 5.5 用例：存活硬删被拒且零副作用；软删后恢复政策正文仍在；硬删清正文与指针但保留 `AGENTS.md`/`CLAUDE.md`/`.claude/rules/*`；幂等；跨项目拒绝

## 6. 错误信息可读性

- [x] 6.1 前端预检 `formatError` 优先展示 `details.reason`（回落 `code: message`，再回落 fallback）
- [x] 6.2 用例：reason 存在 / 缺失 / 全缺三种回落路径

## 7. 回归与门禁

- [x] 7.1 拆分 `registration_tests.inc.rs`（1453 → 22+370+203+861）并回收登记项
- [x] 7.2 拆分 `src/web/types.rs`（1594 → 1144 + 3 子模块）与 `IssueLifecycleWorkbench.test-utils.ts`（1235 → 1077 + 181）并回收登记项
- [x] 7.3 `cargo test --locked --test it_core large_file_guard` 全绿
- [ ] 7.4 全量回归（Rust lib + it_core + it_web + 前端 test/build）由 controller 在重发前统一跑

## 8. 契约与留痕

- [x] 8.1 本 change 的 proposal / design / 两份 spec delta / tasks
- [ ] 8.2 归档前补独立审查结论

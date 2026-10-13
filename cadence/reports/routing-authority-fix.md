# 路由权威重灌修复报告(2026-10-13)

> 诊断权威:`local://diag-routing-authority.md`(2026-10-13 17:56)。
> 代码基线:`feat-b-0808-add-monorepo` @ `d4e88957`;修复 commit:`517f18be`(fix: 身份迁移器误判空项目重灌默认LC槽位,9 文件 +349/−30)。
> 取证方法:主树被 SG3a.0 在途 WIP 挡编译,红绿与回归全部在隔离树 `/tmp/routing-auth-verify`(干净 `d4e88957` + 本修复 diff)完成。

## 修法落地(对照诊断 d 节冻结方案)

1. **收紧 ensure 门**(`repository_store.rs:131-149`):`ensure_identity_schema` 在「feature 开 + 槽位 manifest 缺失」之外,新增 `repos.json 存在` 才执行迁移。迁移执行器的唯一 legacy 源就是 repos.json(`legacy_repositories` 只读它);无 repos.json 的 v1.3 原生项目零迁移、零写入,权威留在 per-LC 子树由 issue 归属解析。
2. **禁止 project_root 兜底聚合根**(`migration_executor.inc.rs::ensure_manifest`):删除 `common_non_git_parent(inputs).unwrap_or_else(project_root)` 兜底;`None` 分支下零输入(或无公共非 git 父)一律 `fail_identity_mismatch(logical_codebase_manifest)` fail-closed,不再伪造 0 成员空 LC。既有 manifest 一致性比对改用 `provider_context_root.as_deref() == Some(manifest...)`,语义不变。
3. **仓名解析改走 issue 归属**:
   - `issue_delivery.rs::resolve_repository_name`(+`compute_issue_delivery_summary` 捕获 issue 归属)→ `resolve_logical_repository_for_issue_codebase(issue.logical_codebase_id, logical_id)`;
   - `plan_group_projection/mod.rs::resolve_plan_target_repository_name` 同款;
   - `web/handlers/lifecycle.rs` 分组视图 alias 补充(best-effort 分支)同款,不再驱动权威迁移。
   归属非 legacy LC 时走 per-LC 子树权威(`resolve_logical_repository_in_lc`,无 ensure 副作用);`None`/legacy 别名保持既有 strict 语义,legacy 项目行为字节级不变。
4. **create 流程补靴(修法 1/2 的必要收尾)**(`repository_store_parts/create.inc.rs::ensure_manifest_membership`):旧路径依赖被误判的 ensure「空 bootstrap」槽位 manifest(数据目录根)后才加成员;收紧后 feature 项目的首个仓在 create 事务内以**新仓公共非 git 父目录**为聚合根 bootstrap 槽位 manifest(与迁移语义一致),已有非本事务成员而 manifest 缺失属损坏窗口,保持 `NotFound` fail-closed 不猜。

## TDD 证据(隔离树 `/tmp/routing-auth-verify`)

红(HEAD + 仅测试,3/3 失败,失败形态精确复刻事故):

- `v13_native_project_without_repos_json_never_bootstraps_default_slot`:strict 解析返回非 NotFound(槽位已被 bootstrap 假 manifest 覆写)。
- `v13_native_delivery_summary_resolves_names_via_issue_codebase_without_migration`:`Err NotFound { kind: "identity_migration_mapping", id: "repository_0001" }`——与事故现场 journal 卡 `backfilling_compatibility` 同款回填失败。
- `empty_legacy_repositories_fail_closed_without_bootstrapping_empty_manifest`:零 legacy 仓 ensure 成功 bootstrap 空 LC(未 fail-closed)。

绿(修复后同三测 3/3 通过):v1.3 原生项目读路径零写入(槽位无 manifest/aggregate-policy/identity-migration)、仓名来自归属 LC 成员 checkout 路径(`checkout_repo_alpha`)、`ProjectStore::get` 后无 legacy 别名 LC/无 migrate_legacy 锁。

## 回归(全部隔离树)

- 核心族过滤(repository_store/logical_codebase/issue_delivery/plan_group_projection):**589 passed / 0 failed / 4 ignored**。
- 全量 `--lib`:4498 passed / 2 failed。其中 `campaign_stage3_advance_confirmed_plan_is_ready_without_provider_start` 在干净 HEAD 基线同败(非本修复引入,SG3a.0 在途面);另一个失败逐轮漂移(automation_p1 / sc_confirm_cancelled)且单跑均过 → 并发 flake,与本修复无关。
- `--test it_web` 交付聚合族:`legacy_single_repo_all_pushed_marks_issue_completed` 过;两个 multi_repo 用例在干净 HEAD 基线同败(`ClaudeCode capability record missing`,环境/在途面,非本修复引入)。
- 中途回归裁定:首次回归 10 失败(create 族依赖旧伪造路径)经 create 补靴修复后全绿,基线对照确认均为本修复引入且已消除。

## 隔离树本地解堵(不入提交)

干净 HEAD 的 lib test 编译自带 3 处 `PlanCandidateMechanicalReportRecord.structure_report` E0560(tests 先行提交引用 SG3a.0 未落库字段)。仅在 `/tmp/routing-auth-verify/src/product/lifecycle_store/workspace_single_candidate_tests.rs` 本地删 3 行解堵取证,不在 517f18be 中。`web/dist` 从 feat-b 拷入以满足 rust-embed 编译。

## 后续(交 controller,顺序敏感)

数据清理按诊断 d 节步骤执行(前置:先部署含 517f18be 的二进制再清数据,否则首次页面刷新复发)。

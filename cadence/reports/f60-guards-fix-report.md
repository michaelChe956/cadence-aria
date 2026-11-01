# F-60 guards 存量测试失败定责与修复报告

日期：2026-09-26 · worktree：feat-b-0808-add-monorepo（BASE 8ba23f47）

## 一、失败形态

`web_logical_codebase_entrypoints::guards::single_repo_rejects_logical_codebase_routes_without_persisting_artifacts`
两轮全量（F59Fix/F60Fix）均红，stash 基线对照同样失败（预先存在）。复现报错：

```
assert_eq 422 != 200 —— POST /api/projects/project_0002/issues
{"code":"repository_branch_list_failed",
 "message":"基准分支解析所需 git 仓库不可用：git [\"show-ref\",...,\"refs/heads/main\"]
            in /tmp/.../legacy: No such file or directory"}
```

## 二、定责：PIB 夹具族（非逻辑冲突）

- 失败断言是测试尾部 **legacy 单仓 issue 创建期望 200**，不是三条 logical-codebase 路由的 404 断言（均通过）。
- PIB（87f2f034 起）给单仓 issue 创建加了创建时基线锁定（`product_resources.rs:188` →
  `resolve_effective_base_branch`），git 仓库不可用 fail-closed 422 是**有意生产语义**
  （REQ-PIB-01/`issue_baseline.rs` 模块注释钉死，spec 无降级口径）。
- 该测试的 `repository_legacy.path = root/legacy` 是**不存在的假路径**，属 PIB 夹具族
  遗漏——先例 3fb8d54b（PibItc/PibB）已同构修 it_core/it_web 多文件，guards.rs 被漏掉。

## 三、修复：夹具同构（不改生产代码）

`tests/web_logical_codebase_entrypoints/guards.rs`：`repository_legacy` 指向处改为真实
git 仓库（`git init -q -b main` + 初始提交，复用 `crate::planning::git` helper），同构于
planning.rs `LegacyPlanningHttpFixture` 与 it_web `git_repo_with_branches` 惯例。
issue 创建走默认链锁定 `main` → 200，原有断言（不写 codebase-selection.json 等）全部保持。

## 四、验证

- 目标用例：`cargo test --locked --test web_logical_codebase_entrypoints guards::single_repo…` **ok**
- 整个二进制：44 passed / 0 failed
- 四门禁：`cargo fmt --check` ✅；`cargo clippy --all-targets --all-features --locked -- -D warnings` ✅
- 全量 `cargo test --locked`：**0 failed**（lib 3643、it_core 187、it_web 351、
  it_product 210、it_provider 54、it_task_run 31、it_interactive 43、本二进制 44 等全绿）

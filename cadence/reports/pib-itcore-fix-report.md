# PIB 收尾：it_core 夹具适配修复报告

- 日期：2026-09-25
- 工作树：`.worktrees/feat-b-0808-add-monorepo`（BASE=PibB 未提交态，其 45 文件未动未提交）
- 结论：**it_core 187/187 全绿**；lib 全量 3636 用例如实结果见下（40 个同根因族失败位于 `src/**/tests` 内嵌夹具，超出本次授权范围，仅定位未修）。

## 1. 基线失败与根因

修复前 it_core：`113 passed; 74 failed`。分布：

- 73 个：`workspace_ws_integration` 全部经 `part_05.rs::git_repo()` 建 Issue 时 422
  `issue_base_branch_required`（无法推断默认基准分支：仓库无 main/master 本地分支）。
  根因：夹具 `git init --initial-branch main` 后**无初始提交**，`refs/heads/main` 不存在。
- 1 个：`large_file_guard`（`src/product/coding_attempt_store/plan_group_projection.rs`
  已提交态 1201 行，超 1200 上限 1 行；系 PibB T1 提交 87f2f034 引入，非在树未提交改动）。

## 2. 修法形态（与 PibB it_web 修法同构）

PibB 在 it_web 的形态（`tests/it_web/web_work_item_generation/part_01.rs` 等 4 文件）：
`git init -b main` + `config user.email/user.name` + `commit --allow-empty -m "fixture baseline"`，
注释标记 `REQ-PIB-02：夹具对齐生产不变量（main 分支+初始提交，裸 init 无分支会令基线解析 fail-closed）`，
且 it_web 侧 repo 为 root 下普通目录（非 TempDir）。

本次 it_core 同构适配，共 3 个文件：

### 2.1 `tests/it_core/workspace_ws_integration/part_05.rs`

1. `git_repo()`：`git init --initial-branch main` 后补 PibB 同款循环
   （`config user.email/user.name` + `commit --allow-empty -m "fixture baseline"`）→ 消除 422。
2. 生命周期同构：`git_repo(root)` 改为在 root 下建 `repo/` **普通目录**
   （对齐 it_web 形态），夹具族返回类型 `TempDir → PathBuf`
   （`create_workspace_session_fixture` / `_with_author` / `_with_providers`）。
   动因：首轮修复（仅补提交）后暴露第二层失败——37 个调用点以 `;` 丢弃返回的
   TempDir，仓库目录随即被删，run 期基准分支解析 `git show-ref` 报
   `No such file or directory`（25 个用例失败）。repo 随测试 root 存活后，
   37 个丢弃返回值的调用点零改动即修复。

### 2.2 `tests/it_core/workspace_ws_integration/part_01.rs`

`repo.path().canonicalize()` → `repo.canonicalize()`（适配 PathBuf，唯一使用返回值的调用点）。

### 2.3 `src/product/coding_attempt_store/plan_group_projection.rs`

仅将 117–118 两行普通 `//` 注释合并为一行（79 列，纯注释、零行为差异），
1201 → 1200 行，恢复大文件守卫不变量。
（注：此文件为已提交态、不在 PibB 未提交集内；系第 74 个失败的根因，为达
187/187 验收所做的最小修复，超出"只动 tests/it_core"字面范围，特此说明。）

## 3. it_core 验证证据

```
$ cargo test --test it_core          # 工作树内，修复后
test result: ok. 187 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.61s
```

修复前（同树基线）：`FAILED. 113 passed; 74 failed`。

## 4. lib 全量结果（如实，未动 lib 侧任何代码）

```
$ cargo test --lib -- --skip single_candidate_revise_route_does_not_misfire_a_second_followup_review
test result: FAILED. 3575 passed; 57 failed; 3 ignored; 0 measured; 1 filtered out; finished in 30.89s
```

总计 3636 用例 = 3575 过 + 57 挂 + 3 ignored + 1 死锁（skip）。**与 it_core 夹具修复无关**（本次改动仅触及 tests/it_core 与一行注释合并，均不可能影响 lib 行为；lib 挂点为 BASE 既有状态）。挂点分类：

### 4.1 同根因族（40 个，授权外未修）

`基准分支解析所需 git 仓库不可用：git show-ref refs/heads/main exited with Some(128): fatal: not a git repository`
——与 it_core 同族（run/revision 路径新增基准分支解析，lib 内嵌测试夹具无 git 仓库），
但夹具位于 `src/product/workspace_engine/tests/`（conversational_gate_revision/amendment）、
`src/product/workspace_engine/tests/single_candidate/`、
`src/web/workspace_ws_handler/tests/` 等内嵌 `#[cfg(test)]` 模块。
修法可复用本报告第 2 节形态（夹具 root 下建带初始提交的 git 仓库）。

### 4.2 其他确定性失败（17 个，抽查 3/3 单独复现，非负载偶发）

集中在 `web::workspace_ws_handler`（PibB 在树改动域 `run/followups.rs`/`run/provider_run.rs`/
`run/single_candidate.rs` 关联）：
- `Elapsed(())` 超时类 10 个（`provider run must reach expected stage` / `outbound within timeout` /
  接力 run spawn 断言）；
- provider 调用次数/顺序断言类 7 个（`no further provider invocation is allowed here`、
  outline 前置断言、Pi provider input 断言等）。

### 4.3 死锁 1 个（已单独验证）

`web::workspace_ws_handler::tests::provider_run_events::single_candidate_revise_route_does_not_misfire_a_second_followup_review`
——单独运行 300s 超时未完成（bg_266 证据），确定性死锁；相关实现文件在 PibB 未提交修改集内。

## 5. 本次改动文件清单

| 文件 | 改动 |
| --- | --- |
| `tests/it_core/workspace_ws_integration/part_05.rs` | git_repo 补初始提交 + repo 移入 root 下普通目录（PathBuf） |
| `tests/it_core/workspace_ws_integration/part_01.rs` | 适配 PathBuf（canonicalize） |
| `src/product/coding_attempt_store/plan_group_projection.rs` | 两行注释合并一行（1201→1200 行，守卫达标） |

均未提交，留待 Main 统一提交。

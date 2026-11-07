# 计划文档：实施 · 聚合初始化 root recipe 对 provider 无关（方案 B）

> **给实施 worker：** 必用 `subagent-driven-development`（推荐）或 `executing-plans` 逐 Task 执行本计划。步骤用 checkbox 语法跟踪。

**目标：** 让聚合初始化的 root recipe 不再绑死 Claude Code —— 选择器选中的 provider 都能接那四条命令（命令按各家 skill 调用语法生成），且「命令没被认出来」不再静默通过。

**架构：** ① 删掉 `require_root_recipe_supported` 的 Claude-only 硬拒，凭据三道检查一条不减；② 四条命令的生成点从「一处写死的裸 Claude 形态」改为「按 provider 生成前缀」，命令名与参数逐字不变；③ root recipe 的 step 结束时若 receipt 判为零动作 ⇒ 判该 step 失败。

**技术栈：** Rust（edition 2024，工具链见 `rust-toolchain.toml`）。

**规格（实施前必读，Plan 只展开它、不改它）：**
- `openspec/changes/root-recipe-provider-agnostic/specs/non-interrupt-repository-bootstrap/spec.md`（REQ-BOOT-03/07/15/16/17）
- `openspec/changes/root-recipe-provider-agnostic/design.md`（决策 1~4 + 17 条事实表）
- `openspec/changes/root-recipe-provider-agnostic/proposal.md`

## 全局约束（每条 Task 隐含包含）

- 🔴 **cargo 必须串行**：`flock /tmp/aria-cargo.lock cargo ...`
- 🔴 **禁止触碰** `/tmp/aria-verify`（owner 共享验收实例，正在人工测试）、`/tmp/aria-accept`；测试产物只落 `/tmp`（`tempfile::tempdir()`）
- 🔴 **TDD**：先写失败测试并跑出失败（贴命令与输出），再实现到绿
- 🔴 **只改实现，不改期望值**：MUST NOT 删用例、MUST NOT 加 `ignore`、MUST NOT 放宽断言强度；**既有成功判据一条不减**
- 🔴 **不扩大范围**：不改 capability 记录结构、不改 provider 优先级、不改安装机制、不改五步结构、不做「非 Claude 的 recipe 证据 per-provider 化」（方案 C，独立立项）、不改 root-recipe 对普通会话的隔离契约
- 🔴 **不新增第二套「有无变更」判定**：零变更判据 SHALL 复用既有 receipt auditor
- 🔴 **行数门禁**：`tests/it_core/large_file_guard.rs` —— 新增文件 < 1200 行；既有超限文件不得增长
- 提交只暂存本 Task 明确的文件；**禁 `git add -A` / `git add .`**；禁 `reset --hard`/`clean -f`/强推/`--no-verify`
- 分支 `bugfix-b-11-02`；MUST NOT 在 `main` 上提交

## 与机器级化 change 的文件面冲突（**必须串行**）

| 文件 | 本 Plan | 机器级化 Plan |
|---|---|---|
`src/product/logical_codebase/production_policy_resolvers.rs` | Task 1 | Task 3 / Task 4 |
`src/product/logical_codebase/coordinator_provider_turn.inc.rs` | Task 2 | Task 2 |

⇒ **本 Plan 的 Task 1 与 Task 2 SHALL 在机器级化 change 的 Task 1~4 全部提交之后才开始。**（机器级化 Plan：`cadence/plans/2026-11-05_计划文档_实施_Provider能力核验机器级化_v1.0.md`）

## 审查焦点（规格暗示但没有任务覆盖、最容易咬人的五类）

1. **形态没生成对** —— provider 名与前缀映射拼错（`kimi_code` 的拼写、`$cadence-init:` 的连字符）⇒ 命令静默无效
2. **单仓路径漏改** —— 单仓初始化（`repository_store/initializer.rs:81-85`）同样消费 `command()`；只改聚合路径 ⇒ 单仓仍发 Claude 形态
3. **凭据检查被顺手放宽** —— 删硬拒时误删版本钉定或凭据重核验 ⇒ 非 Claude 绕过凭据门
4. **零变更误伤** —— 把「合法零变更」的 step 判成失败 ⇒ 正常链路被打断
5. **旧失败码残留** —— `root_recipe_requires_fixed_claude_provider` 仍在某条失败路径上出现 ⇒ 用户看到过时且误导的原因

---

### Task 1: 放开 root-recipe 相位的 Claude-only 硬拒

**文件：**
- 修改：`src/product/logical_codebase/production_policy_resolvers.rs:366-371`
- 测试：同文件单测 + `src/product/logical_codebase/provider_admission_preflight_tests_policy_body.inc.rs`（既有放行/拒绝面用例）

**接口：**
- 消费：无（前置）
- 产出：`require_root_recipe_supported` 不再因 provider 类型拒绝；失败原因**只**可能是三种既有原因（重核验通道缺失 / 版本漂移 / 凭据重核验被拒）

- [ ] **步骤 1：写失败测试** —— 加三条用例：
  - `root_recipe_admission_accepts_non_claude_provider`：构造 provider 为 Pi、凭据三道检查全通过 ⇒ 准入返回 Ok
  - `root_recipe_admission_still_denies_when_credential_channel_missing` ⇒ Err，错误码为凭据通道缺失
  - `root_recipe_admission_still_denies_when_credential_recheck_fails` ⇒ Err，错误码为凭据重核验被拒
- [ ] **步骤 2：跑测试确认失败** —— `flock /tmp/aria-cargo.lock cargo test --locked --lib production_policy_resolvers` ⇒ 期望 FAIL（第一条：Pi 被拒）
- [ ] **步骤 3：实现** —— 删掉 `provider.provider_type != ProviderRefType::ClaudeCode` 分支及其错误构造；**保留**其余三道检查原样
- [ ] **步骤 4：跑测试确认通过** —— 同步骤 2 ⇒ PASS
- [ ] **步骤 5：清理旧失败码** —— `grep -rn "root_recipe_requires_fixed_claude_provider" src/` ⇒ 目标 **0 命中**；若有命中（常量定义、拼装点、测试断言）**全部清理**（常量、拼装、断言一并删，不留别名）
- [ ] **步骤 6：跑 admission 面测试** —— `flock /tmp/aria-cargo.lock cargo test --locked --lib provider_admission_preflight` ⇒ PASS（断言强度不得放宽）
- [ ] **步骤 7：提交** —— `git add src/product/logical_codebase/production_policy_resolvers.rs` + `git commit -m "fix(product): root recipe 相位准入不再绑死 Claude Code（凭据三道检查不变）"`

---

### Task 2: 命令按 provider 的 skill 调用语法生成

**文件：**
- 修改：`src/product/repository_store/types.rs:140-148`（命令生成）
- 修改：`src/product/logical_codebase/coordinator_provider_turn.inc.rs:78-103`（`recipe_commands` 消费点）
- 修改：`src/product/repository_store/initializer.rs:81-85`（单仓路径消费点）
- 测试：`src/product/repository_store/types.rs` 单测 + `coordinator_provider_turn.inc.rs` 既有「recipe 命令文本必须保持 prompt 逐字前缀」用例（`:1211-1215`）

**接口：**
- 产出：`RepositoryInitializationStepKind::command_for(provider: ProviderName) -> Option<String>`（**保留** `command()` 本身不动还是替换，由实现者按最小改动决定；但**唯一**的运行时命令来源 SHALL 是 provider-aware 的那个）
- 产出：前缀映射（**逐字固定**，禁止运行时拼装其它形式）——
  ```
  ClaudeCode => "/"
  Pi         => "/skill:"
  KimiCode   => "/skill:"
  Codex      => "$cadence-init:"
  ```
  完整形态 = 前缀 + 命令名 + `" "` + 原 args（args 逐字不变）

- [ ] **步骤 1：写失败测试** —— 加表驱动用例：对 4 家 provider × 4 条命令 = 16 个形态**逐字断言**：
  ```
  ClaudeCode, PreCheck            => "/pre-check --no-interrupt --upgrade 用大陆镜像"
  Pi,         PreCheck            => "/skill:pre-check --no-interrupt --upgrade 用大陆镜像"
  KimiCode,   PreCheck            => "/skill:pre-check --no-interrupt --upgrade 用大陆镜像"
  Codex,      PreCheck            => "$cadence-init:pre-check --no-interrupt --upgrade 用大陆镜像"
  ```
  其余三条命令同理（`rule-config` / `mcp-configuration` / `project-rules-examples`，args 各自原样）
  再加两条：`command_for` 对同一 (provider, step) **两次调用结果相同**；四家之间**只有前缀不同**（命令名与 args 部分相同）
- [ ] **步骤 2：跑测试确认失败** —— `flock /tmp/aria-cargo.lock cargo test --locked --lib repository_store` ⇒ 期望 FAIL
- [ ] **步骤 3：实现** —— 在 `types.rs` 加 provider-aware 生成函数（**纯函数、确定性**）；两个消费点改为按所选 provider 生成
- [ ] **步骤 4：跑测试确认通过** —— 同步骤 2 ⇒ PASS
- [ ] **步骤 5：跑 recipe 消费面测试** —— `flock /tmp/aria-cargo.lock cargo test --locked --lib coordinator_provider_turn` ⇒ PASS（既有「prompt 逐字前缀」用例若因形态变化而红 ⇒ **按新形态改写断言**，不得放宽）
- [ ] **步骤 6：验收 grep** —— `grep -rn "cadence-init:" src/` ⇒ 命中处 SHALL 仅在**前缀定义/映射**处；SHALL NOT 出现硬编码的完整命令串
- [ ] **步骤 7：提交** —— 显式列出 3 个文件 + `git commit -m "feat(product): recipe 命令按 provider 的 skill 调用语法生成"`

---

### Task 3: root recipe turn 零变更必须显式失败

**文件：**
- 修改：root recipe step 的完成判定处（**复用**既有 receipt auditor 的零变更判据，不新增判定逻辑）
- 测试：既有 coordinator / provider_turn 单测 + 新增零变更用例

**接口：**
- 产出：root recipe 的四个 step（`PreCheck` / `RuleAndMcpConfig` / `OpenspecAndExamples`）结束时，零动作 ⇒ 该 step 判失败，失败事实含 **provider、step、零变更** 三项
- 复用：既有 receipt auditor 的动作判定（**MUST NOT** 出现第二套「有无变更」判定）

- [ ] **步骤 1：写失败测试** —— 造一个「turn 正常结束但零动作」的 root recipe step ⇒ 期望该 step 判**失败**且失败事实含 provider / step / 零变更三项；再造一个「有变更」的 ⇒ 期望照既有语义成功
- [ ] **步骤 2：跑测试确认失败** —— `flock /tmp/aria-cargo.lock cargo test --locked --lib coordinator` ⇒ 期望 FAIL（第一步）
- [ ] **步骤 3：实现** —— 在既有零变更判据的**消费点**上，把 root recipe step 的「零动作」判为失败；SHALL NOT 新增第二套判定逻辑；SHALL NOT 改动其它 step 的成功判据
- [ ] **步骤 4：跑测试确认通过** —— 同步骤 2 ⇒ PASS
- [ ] **步骤 5：回归** —— `flock /tmp/aria-cargo.lock cargo test --locked --lib provider_turn aggregate_initialization` ⇒ PASS；既有断言不得被删或放宽
- [ ] **步骤 6：提交** —— 显式列出文件 + `git commit -m "fix(product): root recipe turn 零变更判失败（不再静默通过）"`

---

### Task 4: 验证矩阵

**文件：** 无代码改动（只跑门禁）

- [ ] **步骤 1** —— `flock /tmp/aria-cargo.lock cargo test --locked --all-targets` ⇒ 全绿（贴输出）
- [ ] **步骤 2** —— `flock /tmp/aria-cargo.lock cargo clippy --locked --all-targets -- -D warnings` ⇒ exit 0
- [ ] **步骤 3** —— `cargo fmt --check` ⇒ 干净
- [ ] **步骤 4** —— `openspec validate --specs --strict` ⇒ 仍 **78 passed / 0 failed**；`openspec validate root-recipe-provider-agnostic --strict` ⇒ valid
- [ ] **步骤 5：验收 grep** ——
  ```
  grep -rn "root_recipe_requires_fixed_claude_provider" src/   ⇒ 0
  grep -rn "cadence-init:" src/                                 ⇒ 仅前缀映射处
  ```
- [ ] **步骤 6** —— `git status --porcelain` ⇒ 空

---

## 实施后（不在本 Plan 内，另行处理）

- **真机验收**（tasks 4.7）：owner 在 4317 上重跑聚合初始化 ⇒ 选择器选中 Pi ⇒ Pi 收到 `/skill:pre-check …` ⇒ pre_check **真的产生变更**（或**明确失败**并指出 provider + step），SHALL NOT 静默零变更
- **归档**：本 change MUST 在 `globalize-provider-capability-verification` 之后归档
- **独立立项**：非 Claude provider 的 recipe 证据 per-provider 化（方案 C）

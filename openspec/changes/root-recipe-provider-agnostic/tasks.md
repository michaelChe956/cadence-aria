# Tasks: root-recipe-provider-agnostic

> owner 2026-11-05 拍板（方案 B：放开 Claude 硬拒 + 按 provider 生成命令 + 零变更即失败）。决策记录见同目录 `design.md`。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记高层工作包，每个工作包映射到相关 requirement。
> **纪律**：本工作包只改 `src/`、`tests/` 与本 change 的 OpenSpec 产物；MUST NOT 改 `openspec/specs/**`（主 spec 等归档时 sync）。

## 1. 放开 root-recipe 相位的 Claude-only 硬拒

映射 requirement：REQ-BOOT-15、REQ-BOOT-07

- [x] 1.1（`1cb9253`）`production_policy_resolvers.rs:366-371` 删除 `provider_type != ClaudeCode` 的硬拒分支；**保留**其余三道检查（重核验通道在场 / 版本钉定 / 凭据对 durable Running operation 重核验）与其 fail-closed 语义
- [x] 1.2 失败面：准入失败时**不得**再出现 `root_recipe_requires_fixed_claude_provider`；SHALL 返回与真实原因相符的稳定码（凭据通道缺失 / 版本漂移 / 凭据重核验被拒），且三者的中文文案各自可区分
- [x] 1.3 验收 grep：`grep -rn "root_recipe_requires_fixed_claude_provider" src/` ⇒ **0 命中**（常量、拼装点、测试断言一并清理）
- [x] 1.4 单测：非 Claude provider（用 Pi）在凭据三道检查通过时**放行**；凭据通道缺失 / 版本漂移 / 凭据重核验被拒三种情形**分别**仍 fail-closed 且稳定码不同

## 2. recipe 命令按 provider 的 skill 调用语法生成

映射 requirement：REQ-BOOT-16、REQ-BOOT-03

- [x] 2.1（`827bb0d`）`src/product/repository_store/types.rs:140-148` 的命令生成改为**按 provider** 出前缀：Claude `/<skill>`、Pi 与 Kimi Code `/skill:<skill>`、Codex `$cadence-init:<skill>`；**命令名与 args 逐字不变**
- [x] 2.2 两个消费点同步：`coordinator_provider_turn.inc.rs:78-103`（`recipe_commands`）与 `repository_store/initializer.rs:81-85`（单仓路径）
- [x] 2.3 生成 SHALL 是**确定性纯函数**（同 provider + 同一步 ⇒ 同文本）；SHALL NOT 依赖运行时状态
- [x] 2.4 单测：四家 × 四条命令 = 16 个形态逐字断言；同一输入两次调用结果相同；命令名与 args 在四家之间**只差前缀**
- [x] 2.5 验收 grep：`grep -rn "cadence-init:" src/` ⇒ 命中处 SHALL 仅在**前缀生成**处；SHALL NOT 出现硬编码的完整命令串

## 3. root recipe turn 零变更必须显式失败

映射 requirement：REQ-BOOT-17

- [x] 3.1（`77640c99`）复用**既有** receipt auditor 的零变更判据（SHALL NOT 新增第二套「有无变更」判定），在 root recipe 的 step 结束时把「零动作」判为该 step 失败
- [x] 3.2 失败事实 SHALL 记录：provider、step、零变更；SHALL NOT 继续推进后续 step
- [x] 3.3 单测：零变更 ⇒ step 判失败 + 失败事实含三项；非零变更 ⇒ 照既有语义成功（**既有成功判据不得被放宽**）
- [x] 3.4 回归：`coordinator` / `provider_turn` 相关既有用例 MUST NOT 被删或被放宽断言强度

## 4. 验证矩阵

映射 requirement：REQ-BOOT-03、REQ-BOOT-07、REQ-BOOT-15、REQ-BOOT-16、REQ-BOOT-17

- [x] 4.1（2026-11-05 实测）`flock /tmp/aria-cargo.lock cargo test --locked --lib` ⇒ lib 6001/0
- [x] 4.2（2026-11-05 实测）`cargo test --locked --all-targets` ⇒ 10 个目标全绿
- [x] 4.3（2026-11-05 实测）`cargo clippy --locked --all-targets -- -D warnings` ⇒ exit 0；`cargo fmt --check` ⇒ 干净
- [x] 4.4 `openspec validate --specs --strict` ⇒ **仍 78 passed / 0 failed**（MUST NOT 把现有 spec 改红）
- [x] 4.5（2026-11-05 实测）`openspec validate root-recipe-provider-agnostic --strict` ⇒ valid
- [x] 4.6 归档顺序自查：本 change 的 delta 覆盖 REQ-BOOT-03 / REQ-BOOT-07 ⇒ 归档时 MUST 在 `globalize-provider-capability-verification` **之后**
- [ ] 4.7 真机验收：owner 在 4317 上重跑聚合初始化 ⇒ 选择器选中 Pi ⇒ Pi 收到 `/skill:pre-check …` ⇒ pre_check **真的产生变更**（或**明确失败**并指出 provider + step），SHALL NOT 静默零变更

## 4A. owner 2026-11-05 四点裁决的落地（顺序 / Kimi 例外 / 两处红字）

映射 requirement：REQ-BOOT-03、REQ-BOOT-07、REQ-BOOT-15、REQ-BOOT-18

- [x] 4A.1 契约增补：owner 四点裁决写入 delta（顺序改三家 / Kimi 例外 / 两处红字）—— `d185174c`；Kimi 缺口另立 change `kimi-root-recipe-support`（只记录、不实施，owner 裁定）—— `d185174c` / `3be0ab52`
- [x] 4A.2 候选顺序改三家：`AGGREGATE_PROVIDER_CANDIDATE_ORDER` = Pi > ClaudeCode > Codex（不含 KimiCode）；`PROVIDER_SELECTION_RULE_VERSION` bump 到 `aggregate-provider-owner-order-v2` —— `aeb9ddd2`（红测试）/ `ef19dfa4`（实现）
- [x] 4A.3 连带同步：候选表夹具改为从常量派生、`selection_failure` / `l2_global_capability` / `coordinator_tests_contract` 的家数与期望同步；失败文案「四家 provider …」改为与家数无关措辞 —— `bbf17190`
- [x] 4A.4 两处红字：Provider 管理抽屉（作用域说明之后）+ 逻辑代码库管理抽屉（聚合初始化卡标题行之后）—— `9a939faf`（红测试）/ `bf9db0d9`（实现）/ `b723814d`（去常驻 `role="alert"`）
- [x] 4A.5 独立审查：前端切片 0 阻断、后端切片 0 阻断（4 条注释建议已收进 `e2fe3736`）—— `cadence/reports/2026-11-05_审查_聚合初始化红字前端切片_v1.0.md` / `…_候选顺序改三家后端切片_v1.0.md`
- [x] 4A.6 全量门禁：`cargo test --locked --all-targets` ⇒ lib 6006 / it_core 198 / it_web 602 / entrypoints 49，**0 failed**；clippy / fmt / `openspec validate --specs --strict`（78 passed / 0 failed）全过
- [ ] 4A.7 真机验收（owner，2026-11-05 从零冷启动）：① 候选表只三家、顺序 Pi > Claude Code > Codex；② 两处红字逐字可见；③ 聚合初始化选中 Pi 并真产生根级材料（与 4.7 合并验）

## 5. 与机器级化 change 的关系

- 本 change 与 `globalize-provider-capability-verification` **并行推进但归档有序**（后者先）
- 文件面：`production_policy_resolvers.rs` 与 `types.rs` 是本 change 独占；机器级化的 Task 3/4 也碰 `production_policy_resolvers.rs` ⇒ **两者不可同时改该文件**，须串行

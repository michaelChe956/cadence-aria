# 提案：聚合初始化 root recipe 对 provider 无关（放开 Claude 硬拒 + 按 provider 生成命令 + 零变更即失败）

> 状态：草案 —— owner 2026-11-05 拍板（方案 B）。决策记录见同目录 `design.md` 与 `.openspec.yaml`。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`。

## 背景

owner 2026-11-05 真机验收时撞到聚合初始化失败：

```
问题：provider 的第一步（pre_check）没有完成
机器码：aggregate_pre_check_failed
诊断：stage aggregate_initialization: provider admission preflight denied: Waiting {
  reason_code: "provider_capability_not_satisfied",
  detail: "provider_gateway_capability: root_recipe_requires_fixed_claude_provider: got Pi",
  allowed_actions: [Revalidate, Retry] }
```

**直接原因**：provider 选择器按 owner 顺序 `Pi > KimiCode > ClaudeCode > Codex` 选中 Pi（Pi 的 capability 已 Confirmed），而 root-recipe 相位的准入在 `production_policy_resolvers.rs:366-371` 硬拒非 ClaudeCode ⇒ **第一个 provider turn 在 spawn 前就被拒（零 spawn，命令根本没发出去）**，后两个 turn 与聚合索引构建均未启动。

**更深一层的成因（勘察实测）**：

| 事实 | 位置 |
|---|---|
四条 recipe 命令是**一处写死的裸字面量**，发给四家**完全相同** | `src/product/repository_store/types.rs:140-148`：`/pre-check --no-interrupt --upgrade 用大陆镜像` 等四条 |
prompt **逐字透传**，没有任何按 provider 翻译的层 | `coordinator_provider_turn.inc.rs:196-207` → 各 adapter 原样发出（Claude `:615`、Codex `:480`、Kimi `:472`、Pi `:280`） |
代码里**不存在** skill 调用语法映射 | `src/` 全树 grep `skill:` / `/skill` / `$cadence-init` ⇒ **0 命中**；git 全历史亦无生成痕迹 |
**没有「命令未被 provider 识别」的失败码** | 命令不被识别 ⇒ turn 正常 Completed 但**零文件变更**，或 provider 自报 failure 归入 `provider_status_failed` |

⇒ 只放开准入门（而不改命令形态）会让 Pi 收到 Claude 的 `/pre-check ...` ⇒ **静默零变更的假成功**，比现在的干脆拒绝更危险。

## 目标

| # | 目标 |
|---|---|
1 | **root-recipe 相位对 provider 无关**：准入不再要求 provider 是 ClaudeCode；其余三道检查（凭据重核验通道在场 / 版本钉定 / 凭据对 durable Running operation 重核验）**一条不减** |
2 | **命令按 provider 的 skill 调用语法生成**：Claude `/<skill>`、Pi 与 Kimi Code `/skill:<skill>`、Codex `$cadence-init:<skill>`；**args 逐字不变** |
3 | **零变更即失败**：root recipe turn 未产生任何变更 SHALL 判为明确失败，SHALL NOT 静默 Completed |

## 非目标

- 不改 capability 记录结构、不改 provider 优先级顺序、不改 provider 安装机制
- 不改「不点击不运行探针」契约、不改聚合初始化的五步结构
- 不改 root-recipe 相位对**普通会话**的隔离契约（root recipe 证据不得被普通会话复用）
- 不改其它 per-LC 作用域（manifest / policy / operation / trust / receipt / checkout / index）
- 不在本 change 内做「非 Claude provider 的 recipe 证据 per-provider 化」（那是方案 C，属独立立项）

## 范围

| 能力 | 变更 |
|---|---|
`non-interrupt-repository-bootstrap` | MODIFIED：REQ-BOOT-03（四条命令的形态按 provider 生成）、REQ-BOOT-07（root-recipe 相位准入不再绑 Claude）；ADDED：root-recipe 相位的 provider 无关准入（REQ-BOOT-15）、命令按 provider skill 语法生成（REQ-BOOT-16）、零变更必须显式失败（REQ-BOOT-17） |

## 关键联动

```
root-recipe 相位准入：provider 必须 == ClaudeCode
        │
        └──► 放开：任何已核验 provider 可进（凭据三道检查不变）
                    │
选择器 owner 顺序 Pi > KimiCode > ClaudeCode > Codex
        │
        └─► 选中 Pi ⇒ Pi 收到【Pi 形态】的命令（/skill:pre-check …）
                    │
                    └─► 命令未被认出来 ⇒ 零变更 ⇒ 【显式失败】（不再静默 Completed）
```

## 风险与取舍

| 风险 | 处置 |
|---|---|
四条命令是**按 Claude 的 skills 命令设计的**，非 Claude provider 能否真的执行**没有既有证据** | 这正是 owner 选择 B 的前提：owner 提供的 skill 调用形态是产品事实。本 change 的价值在于**把形态写对**并**让失败变响**；若某家仍跑不通，失败面会明确指出该 provider 的该 step，而不是静默 |
只放开准入、不改形态 ⇒ **假成功** | 形态生成是本 change 的**强制组成部分**（目标 2），与放开门同批落地 |
「零变更即失败」可能误伤**合法零变更**的 step | 只对 root recipe 的四个 step 生效；判定复用既有 receipt auditor 的零变更判据，不新增判定逻辑；发现误伤则按事实回改 spec，不放宽断言 |
两个 change 同时 MODIFIED 同一 requirement | 本 change **MUST 在** `globalize-provider-capability-verification` **归档后归档**（见 `.openspec.yaml` 的 `archive-order`） |

## owner 决策记录（2026-11-05）

| # | 决策 | 结论 |
|---|---|---|
| 1 | 修法 | **方案 B**：放开 root-recipe 相位的 Claude-only 硬拒 |
| 2 | 命令形态 | B 必须连同「按 provider 生成 skill 调用语法」一起落地（owner 提供的形态为产品事实） |
| 3 | 失败可辨 | root recipe turn 零变更 SHALL 显式失败 |
| 4 | 归档顺序 | 本 change 在 `globalize-provider-capability-verification` 之后归档 |

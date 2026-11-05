## MODIFIED Requirements

### Requirement: 聚合根 recipe 与 readiness 事实闭环（REQ-BOOT-03）

逻辑代码库的 root-cwd recipe SHALL 复用四条无中断命令的取消、超时、输出摘要与步骤恢复语义，并将其映射到既有五步聚合初始化 operation。

**recipe provider SHALL NOT 固定为 Claude Code。** 聚合初始化 SHALL 在 operation 创建之前按 owner 顺序（`Pi > ClaudeCode > Codex`）自动选择一个本机可用的 provider，并把它**冻结**在该 operation 上；执行期与重试期 MUST NOT 改选。选中的 provider SHALL 记入 `operation.input.provider`，候选表 SHALL 记入 `operation.provider_selection`。provider 选择 SHALL NOT 放宽 gateway 的 capability 准入门。

**四条命令的调用形态 SHALL 按所选 provider 的 skill 调用语法生成**（见 REQ-BOOT-16）；四条命令的**顺序**、**参数**、取消/超时/输出摘要语义 SHALL 与既有实现一致。

#### Scenario: 配方提供者固定

- **WHEN** 逻辑代码库根 recipe 选择 provider
- **THEN** 系统 SHALL 按 `Pi > ClaudeCode > Codex` 的顺序逐个探测，选中第一个命令可用且其**机器级**能力已核验、且通过 root-recipe 相位准入的 provider；recipe provider MUST NOT 固定为 Claude Code；选中结果 SHALL 冻结在该 operation 上（`operation.input.provider`），执行期与重试期 MUST NOT 改选

#### Scenario: 信任登记不入步骤

- **WHEN** LC 根准入冻结 canonical root 后、所选 provider 的 recipe operation 启动前完成 trust 前置登记
- **THEN** LC 根准入冻结 canonical root 后、所选 provider 的 recipe operation 启动前 SHALL 完成所选 Codex/Kimi trust 的独立前置登记与核验。该登记不属于五步 operation、不插入步骤之间且不新增第六步。

#### Scenario: 信任未就绪阻断五步

- **WHEN** 任一 trust 未 Ready，或全部 trust Ready
- **THEN** 它是五步 recipe 的硬前置门，任一 trust 未 Ready 时五步 operation 不创建/不启动并进入可重试 fail-closed waiting。全部 trust Ready 后才启动五步 recipe。

#### Scenario: 根级事实独立记录

- **WHEN** 命令成功并产出根级产物 receipt 与投影
- **THEN** 命令成功、根级产物 receipt、根 policy/rule 摘要和 bootstrap 投影 SHALL 独立记录。

#### Scenario: 投影齐备才算就绪

- **WHEN** 判定 recipe 完成是否达到 `planning_ready`
- **THEN** recipe 完成本身 MUST NOT 被视为 `planning_ready`，直到规则/policy、成员索引和聚合索引 active 等投影事实均完成。

#### Scenario: 四命令按根顺序执行

- **WHEN** LC 根准入已确定并冻结 canonical root，所选 Codex/Kimi trust gate 已登记并核验后启动新 LC 的五步聚合初始化 recipe
- **THEN** SHALL 按 MachineSkills→AggregatePreflight→PreCheck→RuleAndMcpConfig→OpenspecAndExamples 原样推进；四条 `/pre-check --no-interrupt --upgrade 用大陆镜像`、`/rule-config --no-interrupt`、`/mcp-configuration --no-interrupt`、`/project-rules-examples --no-interrupt` SHALL 按固定顺序在根执行，且其**命令文本 SHALL 按所选 provider 的 skill 调用语法生成**（见 REQ-BOOT-16，参数部分逐字不变）；任一 trust 登记失败时五步 recipe 不启动并保留可重试等待事实，任一命令失败、取消或输出不可审计时 SHALL 停止 recipe/后续命令并保留失败事实。

#### Scenario: recipe 成功但 readiness 材料未齐

- **WHEN** 四条命令已成功但根 rule/policy receipt、MemberIndex 或 AggregateIndexActive 投影缺失或摘要漂移
- **THEN** operation MAY 为 Completed，但 `planning_ready` SHALL 保持非就绪并返回可操作的核验/继续等待项；普通 provider session SHALL 不得仅凭 recipe Completed 放行

#### Scenario: 根 recipe 不产生单仓 Git 副作用

- **WHEN** root-cwd recipe 为包含多个独立 Git 成员的 LC 执行
- **THEN** 系统 SHALL NOT 调用单仓登记持久化、成员 GitFinalize、成员主 checkout 写入或成员初始化 provider；成员 Git 身份与状态 SHALL 保持不变

### Requirement: 聚合初始化的 provider 自动选择与冻结（REQ-BOOT-07）

聚合初始化 SHALL 提供 provider 选择器：按 owner 顺序 `Pi > ClaudeCode > Codex` 逐个探测，在 **operation 创建之前**选中第一个可用的 provider 并**冻结**（执行期与重试期 MUST NOT 改选）。

判定分两层：**L1** 命令可用（取自 provider health 快照）；**L2** 该 provider 在**机器级** capability 记录中的 `PlanningReadOnly.launch` 为 `Confirmed` 且 snapshot 引用一致 —— L2 SHALL 与 gateway 共用**同一个**只读判定函数，MUST NOT 另写一套。

选择器 SHALL NOT 自动触发 capability 核验。无可用候选或 provider health degraded 时 SHALL **不创建 operation**。provider SHALL 走 request，MUST NOT 存进 driver 字段；未知 provider 名 SHALL fail-closed。

被选中的 provider 若要执行 root recipe，SHALL 另行通过 root-recipe 相位准入（见 REQ-BOOT-15）：该准入**不再要求** provider 是 Claude Code，但其凭据三道检查（重核验通道在场 / 版本钉定 / 凭据对 durable Running operation 重核验）SHALL 一条不减。

#### Scenario: 候选皆可用且已核验时选中顺序第一家

- **WHEN** 候选序列中的 provider 命令均在本机可用，且各自的**机器级** `PlanningReadOnly` 能力均为 `Confirmed`
- **THEN** 选择器 SHALL 选中 `Pi`；operation SHALL 以 `pi` 作为 `input.provider` 冻结下来；该 operation 的四条 recipe 命令 SHALL 按 **Pi 的 skill 调用语法**生成（见 REQ-BOOT-16）

#### Scenario: 命令可用性以健康快照为准

- **WHEN** 选择器判定某候选的命令是否可用
- **THEN** 判定 SHALL 取自 provider health 快照的 `available` 与其 `reason_code`（`command_missing` / `timeout` / `non_zero_exit` / `version_unparseable` / `version_too_low` / `io_error` 六值）；MUST NOT 另起一套探测口径

#### Scenario: 命令缺失时跳过该家并记录原因

- **WHEN** 首选候选 `Pi` 的命令不在本机（health `available=false`，`reason_code=command_missing`），而下一家 `ClaudeCode` 命令可用且其**机器级**能力已核验
- **THEN** 选择器 SHALL 跳过 `Pi` 并在候选表里记下 `cli_unavailable` 类原因；选中 `ClaudeCode`；失败面与候选表 MUST NOT 把「未安装」说成「未核验」

#### Scenario: 候选命令可用但都未核验时不创建 operation

- **WHEN** 候选序列中的 provider 命令均在本机可用，但没有任何一家在**机器级**核验过能力
- **THEN** 系统 SHALL **不创建 operation**（不产生任何 provider turn），并返回「需要核验」的失败码，失败面 SHALL 建议核验「顺序中第一家命令可用的 provider」，并携带候选表与可点击的核验入口

#### Scenario: 已核验真实拒绝与未核验可分辨

- **WHEN** 首选候选的能力记录为 `Denied{reason}`，次选候选的能力为未核验
- **THEN** 选择器 SHALL 继续按顺序探测到次选候选；候选表 SHALL 分别如实标出「实测拒绝」与「未核验」两种状态，MUST NOT 把两者压平成同一句

#### Scenario: 重试不换人

- **WHEN** 某 operation 已冻结 provider `pi`，执行期把 provider health 改成只有 `claude_code` 可用，然后对该 operation 执行重试
- **THEN** 该 operation SHALL 继续使用 `pi`；选择器 MUST NOT 在重试路径上重跑并改选

#### Scenario: 能力判定与 gateway 同源

- **WHEN** 选择器与 gateway 对同一 capability store 的同一输入分别做 L2 判定
- **THEN** 两者的结论 SHALL 完全一致（同一 `launch_cell_state` 结果，且读的是**同一份机器级**记录）；MUST NOT 出现选择器放行而 gateway 拒绝（或反之）的情形

#### Scenario: health degraded 时不创建 operation

- **WHEN** 发起聚合初始化时 provider health 处于 degraded 状态
- **THEN** 系统 SHALL **不创建 operation**，并返回指向「重试」与「产品缺陷」档的失败码；MUST NOT 先创建 operation 再在 spawn 前被可用性门拦下

#### Scenario: 未知 provider 名 fail-closed

- **WHEN** durable operation 记录里的 provider 名不是四种真实 provider 之一
- **THEN** provider turn SHALL fail-closed，MUST NOT 回退到任何默认 provider

#### Scenario: 候选表可解释

- **WHEN** 查询任意一次聚合初始化的 operation 记录
- **THEN** `provider_selection.candidates` SHALL 按 owner 顺序列出候选 provider，每家 SHALL 带有判定依据与可执行动作提示（未安装 / 未核验 / 实测拒绝等）；候选顺序 MUST NOT 按结果重排

## ADDED Requirements

### Requirement: root-recipe 相位的 provider 无关准入（REQ-BOOT-15）

root-recipe 相位的 provider 准入 SHALL **不要求** provider 是 Claude Code：任何通过普通 capability 准入（`Confirmed` + snapshot 一致）的 provider SHALL 能进入 root-recipe 相位。

**例外（owner 2026-11-05 裁定）**：`KimiCode` 暂**不参与**聚合初始化的 provider 候选序列 —— 其 LC 策略会话启动层拒绝一切非 `ReviewerVerify` 的通用策略（bootstrap executor marker 在该层被拒），结构上无法承载 root-recipe 相位。该缺口已记录在独立 change `kimi-root-recipe-support`（方案 a：marker 通道放行 + 宿主侧授权等价表达），后续另开 change 解决；**本例外 SHALL NOT 被当作永久形态**。

准入的其余检查 SHALL **一条不减**，且 SHALL 继续 fail-closed：

1. 凭据重核验通道 SHALL 在场（缺失 ⇒ 拒绝，不放行）；
2. 已交付 recipe 证据的版本 SHALL 与记录版本钉定一致（漂移 ⇒ 拒绝）；
3. 凭据 SHALL 每次对**该逻辑代码库的** durable Running operation 重核验（operation 终态 / step 推进 / digest 或 root 漂移 ⇒ 拒绝）。

本 requirement MUST NOT 放宽普通会话的准入强度，也 MUST NOT 让 root recipe 的能力证据被普通会话复用（隔离契约不变）。

#### Scenario: Kimi 不进入聚合初始化候选

- **WHEN** 系统构建聚合初始化的 provider 候选序列
- **THEN** 候选序列 SHALL 为 `Pi` > `ClaudeCode` > `Codex`（**不含** `KimiCode`）；SHALL NOT 把 KimiCode 列为候选、SHALL NOT 在选中后才发现它无法承载 root-recipe 相位

#### Scenario: 非 Claude provider 通过 root-recipe 准入

- **WHEN** 所选 provider 不是 Claude Code，但其普通 capability 准入为 `Confirmed`、凭据三道检查全部通过
- **THEN** root-recipe 相位 SHALL 放行；SHALL NOT 再以「provider 不是 Claude Code」为由拒绝

#### Scenario: 不再出现固定 Claude 的拒绝

- **WHEN** 所选 provider 不是 Claude Code
- **THEN** 准入失败时 SHALL NOT 返回 `root_recipe_requires_fixed_claude_provider`；SHALL 返回与实际失败原因相符的稳定码（凭据通道缺失 / 版本漂移 / 凭据重核验被拒）

#### Scenario: 凭据失效仍 fail-closed

- **WHEN** 某逻辑代码库的 durable Running operation 已终结、或 digest / root 漂移导致凭据重核验失败
- **THEN** root-recipe 准入 SHALL 继续 fail-closed 并给出可重试的失败事实；SHALL NOT 因 provider 不是 Claude Code 而放行、SHALL NOT 因放宽 provider 而跳过凭据检查

#### Scenario: 普通会话不得复用 root recipe 能力

- **WHEN** 普通会话的 Normal capability 不满足，但存在 root recipe 相位的准入事实
- **THEN** 普通会话 SHALL 仍被 fail-closed 拒绝；root-recipe 相位的准入事实 SHALL NOT 被普通会话消费

### Requirement: recipe 命令按 provider 的 skill 调用语法生成（REQ-BOOT-16）

四条 recipe 命令（`pre-check` / `rule-config` / `mcp-configuration` / `project-rules-examples`）的**调用前缀** SHALL 按所选 provider 的 skill 调用语法生成；**命令名**与**参数** SHALL 逐字不变。

| provider | skill 调用前缀 |
|---|---|
Claude Code | `/<skill>` |
Pi | `/skill:<skill>` |
Kimi Code | `/skill:<skill>` |
Codex | `$cadence-init:<skill>` |

生成 SHALL 是**确定性的纯函数**（同一 provider + 同一步 ⇒ 同一命令文本），SHALL NOT 依赖运行时状态；四条命令之外不得新增或改写命令文本。

#### Scenario: Claude 形态保持不变

- **WHEN** 所选 provider 是 Claude Code
- **THEN** 命令文本 SHALL 逐字等于 `/pre-check --no-interrupt --upgrade 用大陆镜像`（及其余三条的既有形态）

#### Scenario: Pi 与 Kimi 形态

- **WHEN** 所选 provider 是 Pi 或 Kimi Code
- **THEN** 命令文本 SHALL 为 `/skill:pre-check --no-interrupt --upgrade 用大陆镜像`（及其余三条同形），args 部分 SHALL 与 Claude 形态逐字相同

#### Scenario: Codex 形态

- **WHEN** 所选 provider 是 Codex
- **THEN** 命令文本 SHALL 为 `$cadence-init:pre-check --no-interrupt --upgrade 用大陆镜像`（及其余三条同形），args 部分 SHALL 与 Claude 形态逐字相同

#### Scenario: 四条命令以外不改文本

- **WHEN** 生成任一步的命令文本
- **THEN** 除这四条命令外，系统 SHALL NOT 生成任何其它 skill 调用文本；SHALL NOT 改写既有四条的命令名或参数

### Requirement: root recipe turn 零变更必须显式失败（REQ-BOOT-17）

root recipe 的四个 step（`PreCheck` / `RuleAndMcpConfig` / `OpenspecAndExamples`）中，任一 turn 结束时若**未产生任何工作区变更**（receipt 判为零动作），系统 SHALL 判该 step **失败**并保留可诊断失败事实；SHALL NOT 把该 turn 记为成功。

理由：provider 侧对「命令未被识别」没有任何专属错误码，其表现是 turn 正常结束但零动作。若不把零动作判成失败，语法错误、skill 缺失、CLI 行为变化都会伪装成成功。

本 requirement SHALL 复用既有 receipt auditor 的零变更判据，SHALL NOT 新增第二套「有无变更」的判定逻辑。

#### Scenario: 零变更的 recipe turn 判失败

- **WHEN** root recipe 的某个 step 结束时 receipt 判为零动作
- **THEN** 该 step SHALL 判为失败，operation SHALL 记录可诊断失败事实（provider、step、零变更）；SHALL NOT 把该 step 记为成功、SHALL NOT 继续推进后续 step

#### Scenario: 有变更的 recipe turn 照旧成功

- **WHEN** root recipe 的某个 step 结束时 receipt 判出实际变更
- **THEN** 该 step SHALL 照既有语义成功；本 requirement SHALL NOT 放宽任何既有成功判据

#### Scenario: 既有成功判据不被改写

- **WHEN** 既有 receipt auditor 的动作判定逻辑被复核
- **THEN** SHALL 复用它；MUST NOT 出现第二套「有无变更」判定

### Requirement: Kimi 能力边界与候选顺序的用户面告知（REQ-BOOT-18）

系统 SHALL 在**两个用户可见入口**用**红字**如实告知聚合初始化准备阶段的 provider 边界与候选顺序：

| 入口 | 位置 | 文案 |
|---|---|---|
| **Provider 管理抽屉** | 作用域说明之后、能力卡之前 | 「聚合初始化准备阶段暂不支持 Kimi Code；该阶段 provider 顺序：Pi > Claude Code > Codex。」 |
| **逻辑代码库管理抽屉（聚合初始化卡）** | 标题行状态之后、步骤列表之前 | 同上 |

- 红字 SHALL 使用危险色 token（`--aria-danger`；带框提示用 `--aria-danger-soft` 底 + `--aria-danger` 边与字）；SHALL NOT 用普通灰字或提示黄字掩盖该边界。
- 该告知 SHALL 与**实际候选序列一致**（见 REQ-BOOT-15 的 Kimi 例外与 REQ-BOOT-03/07 的顺序）；SHALL NOT 声称 Kimi Code 可用于该阶段。
- 该告知是**临时形态**：Kimi 支持该阶段的缺口修复后（change `kimi-root-recipe-support`），两处红字 SHALL 随之移除或改写。

#### Scenario: Provider 管理抽屉的红字

- **WHEN** 用户打开 Provider 管理抽屉
- **THEN** 作用域说明下方 SHALL 出现红字「聚合初始化准备阶段暂不支持 Kimi Code；该阶段 provider 顺序：Pi > Claude Code > Codex。」

#### Scenario: 聚合初始化卡的红字

- **WHEN** 用户在逻辑代码库管理抽屉查看聚合初始化卡
- **THEN** 标题行状态之后、步骤列表之前 SHALL 出现红字「聚合初始化准备阶段暂不支持 Kimi Code；该阶段 provider 顺序：Pi > Claude Code > Codex。」

#### Scenario: 红字与实际候选一致

- **WHEN** 聚合初始化的 provider 候选序列发生变化
- **THEN** 两处红字 SHALL 随之更新，SHALL NOT 继续显示已过时的顺序或已支持的 provider

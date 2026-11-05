## MODIFIED Requirements

### Requirement: 聚合根 recipe 与 readiness 事实闭环（REQ-BOOT-03）

逻辑代码库的 root-cwd recipe SHALL 复用四条无中断命令的取消、超时、输出摘要与步骤恢复语义，并将其映射到既有五步聚合初始化 operation。

**recipe provider SHALL NOT 固定为 Claude Code。** 聚合初始化 SHALL 在 operation 创建之前按 owner 顺序（`Pi > KimiCode > ClaudeCode > Codex`）自动选择一个本机可用的 provider，并把它**冻结**在该 operation 上；执行期与重试期 MUST NOT 改选。选中的 provider SHALL 记入 `operation.input.provider`，候选表 SHALL 记入 `operation.provider_selection`。provider 选择 SHALL NOT 放宽 gateway 的 capability 准入门。

#### Scenario: 配方提供者固定

- **WHEN** 逻辑代码库根 recipe 选择 provider
- **THEN** 系统 SHALL 按 `Pi > KimiCode > ClaudeCode > Codex` 的顺序逐个探测，选中第一个命令可用且其**机器级**能力已核验的 provider；recipe provider MUST NOT 固定为 Claude Code；选中结果 SHALL 冻结在该 operation 上（`operation.input.provider`），执行期与重试期 MUST NOT 改选

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
- **THEN** SHALL 按 MachineSkills→AggregatePreflight→PreCheck→RuleAndMcpConfig→OpenspecAndExamples 原样推进；四条 `/pre-check --no-interrupt --upgrade 用大陆镜像`、`/rule-config --no-interrupt`、`/mcp-configuration --no-interrupt`、`/project-rules-examples --no-interrupt` SHALL 按固定顺序在根执行；任一 trust 登记失败时五步 recipe 不启动并保留可重试等待事实，任一命令失败、取消或输出不可审计时 SHALL 停止 recipe/后续命令并保留失败事实。

#### Scenario: recipe 成功但 readiness 材料未齐

- **WHEN** 四条命令已成功但根 rule/policy receipt、MemberIndex 或 AggregateIndexActive 投影缺失或摘要漂移
- **THEN** operation MAY 为 Completed，但 `planning_ready` SHALL 保持非就绪并返回可操作的核验/继续等待项；普通 provider session SHALL 不得仅凭 recipe Completed 放行

#### Scenario: 根 recipe 不产生单仓 Git 副作用

- **WHEN** root-cwd recipe 为包含多个独立 Git 成员的 LC 执行
- **THEN** 系统 SHALL NOT 调用单仓登记持久化、成员 GitFinalize、成员主 checkout 写入或成员初始化 provider；成员 Git 身份与状态 SHALL 保持不变

### Requirement: 聚合初始化的 provider 自动选择与冻结（REQ-BOOT-07）

聚合初始化 SHALL 提供 provider 选择器：按 owner 顺序 `Pi > KimiCode > ClaudeCode > Codex` 逐个探测，在 **operation 创建之前**选中第一个可用的 provider 并**冻结**（执行期与重试期 MUST NOT 改选）。

判定分两层：**L1** 命令可用（取自 provider health 快照）；**L2** 该 provider 在**机器级** capability 记录中的 `PlanningReadOnly.launch` 为 `Confirmed` 且 snapshot 引用一致 —— L2 SHALL 与 gateway 共用**同一个**只读判定函数，MUST NOT 另写一套。

选择器 SHALL NOT 自动触发 capability 核验。无可用候选或 provider health degraded 时 SHALL **不创建 operation**。provider SHALL 走 request，MUST NOT 存进 driver 字段；未知 provider 名 SHALL fail-closed。

#### Scenario: 四家皆可用且已核验时选中顺序第一家

- **WHEN** 四家候选的命令均在本机可用，且各自的**机器级** `PlanningReadOnly` 能力均为 `Confirmed`
- **THEN** 选择器 SHALL 选中 `Pi`；operation SHALL 以 `pi` 作为 `input.provider` 冻结下来

#### Scenario: 命令可用性以健康快照为准

- **WHEN** 选择器判定某候选的命令是否可用
- **THEN** 判定 SHALL 取自 provider health 快照的 `available` 与其 `reason_code`（`command_missing` / `timeout` / `non_zero_exit` / `version_unparseable` / `version_too_low` / `io_error` 六值）；MUST NOT 另起一套探测口径

#### Scenario: 命令缺失时跳过该家并记录原因

- **WHEN** 首选候选 `Pi` 的命令不在本机（health `available=false`，`reason_code=command_missing`），而下一家 `KimiCode` 命令可用且其**机器级**能力已核验
- **THEN** 选择器 SHALL 跳过 `Pi` 并在候选表里记下 `cli_unavailable` 类原因；选中 `KimiCode`；失败面与候选表 MUST NOT 把「未安装」说成「未核验」

#### Scenario: 四家命令可用但都未核验时不创建 operation

- **WHEN** 四家候选的命令均在本机可用，但没有任何一家在**机器级**核验过能力
- **THEN** 系统 SHALL **不创建 operation**（不产生任何 provider turn），并返回「需要核验」的失败码，失败面 SHALL 建议核验「顺序中第一家命令可用的 provider」，并携带四家候选表与可点击的核验入口

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
- **THEN** `provider_selection.candidates` SHALL 按 owner 顺序列出四家候选，每家 SHALL 带有判定依据与可执行动作提示（未安装 / 未核验 / 实测拒绝等）；候选顺序 MUST NOT 按结果重排

## ADDED Requirements

### Requirement: 全局能力核验后聚合初始化自动开始（REQ-BOOT-14）

聚合初始化 SHALL **不**以「该逻辑代码库的能力尚未核验」为由阻断自动开始：当至少一家候选 provider 命令可用且其在**机器级** capability 记录中的能力为 `Confirmed` 时，冷启动推进到聚合初始化 SHALL 自动创建 operation 并开始执行。

聚合根内是否放置 `AGENTS.md` 与 `.claude/rules/language.md` 两个判定文件 SHALL NOT 是聚合初始化能否自动开始的**前提**：这两个判定文件所承载的是**另一条独立路径**（整条链零 provider turn、零额度、零探针），该路径 SHALL 保持原语义，MUST NOT 被取消或改写。

本 requirement MUST NOT 放宽 provider 准入：L1（命令可用）与 L2（能力 `Confirmed`）两层判定本身 SHALL NOT 改变，MUST NOT 因「全局已核验」而跳过 L1 或把 `Denied`/`Unknown` 放行。

#### Scenario: 全局核验一次后聚合初始化自动开始

- **WHEN** 用户对四家 provider 完成一次全局能力核验，随后冷启动推进到聚合初始化，且聚合根内**没有** `AGENTS.md` 与 `.claude/rules/language.md`
- **THEN** provider 选择门的 L2 SHALL 读到机器级记录中的 `Confirmed` 并放行；系统 SHALL 自动创建 operation 并开始执行；SHALL NOT 以「能力未核验」为由拒绝创建 operation

#### Scenario: 全局未核验仍被第二道门阻断

- **WHEN** 机器级 capability 记录中没有任何候选 provider 的能力为 `Confirmed`
- **THEN** 系统 SHALL **不创建 operation**，并返回既有「需要核验」失败码与可点击的核验入口；SHALL NOT 自动发起核验

#### Scenario: 判定文件路径仍是零 provider turn 路径

- **WHEN** 聚合根内具备 `AGENTS.md` 与 `.claude/rules/language.md`
- **THEN** 系统 SHALL 继续走「整条链零 provider turn」路径（免过 provider 选择门、`provider_selection` 为空、不消耗任何 provider 额度）；该路径 SHALL NOT 因本次全局化而被取消或改写

#### Scenario: 真实拒绝不得因全局化而放行

- **WHEN** 某候选 provider 的全局能力记录为 `Denied{reason}`
- **THEN** 选择器 SHALL 继续按 owner 顺序探测下一家；SHALL NOT 把 `Denied` 当作 `Confirmed` 放行

#### Scenario: 机器级核验一次即生效

- **WHEN** 用户完成一次机器级能力核验
- **THEN** 任何 provider 选择门 SHALL 读到**同一份**机器级记录并据此判定；SHALL NOT 要求再核验一次；SHALL NOT 存在按项目或按逻辑代码库区分的记录副本

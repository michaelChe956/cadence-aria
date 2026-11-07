## MODIFIED Requirements

### Requirement: 聚合根 recipe 与 readiness 事实闭环（REQ-BOOT-03）

逻辑代码库的 root-cwd recipe SHALL 复用四条无中断命令的取消、超时、输出摘要与步骤恢复语义，并将其映射到既有五步聚合初始化 operation。

**recipe provider SHALL NOT 固定为 Claude Code。** 聚合初始化 SHALL 在 operation 创建之前按 owner 顺序（`Pi > KimiCode > ClaudeCode > Codex`）自动选择一个本机可用的 provider，并把它**冻结**在该 operation 上；执行期与重试期 MUST NOT 改选。选中的 provider SHALL 记入 `operation.input.provider`，候选表 SHALL 记入 `operation.provider_selection`。provider 选择 SHALL NOT 放宽 gateway 的 capability 准入门。

#### Scenario: 配方提供者固定

- **WHEN** 逻辑代码库根 recipe 选择 provider
- **THEN** 系统 SHALL 按 `Pi > KimiCode > ClaudeCode > Codex` 的顺序逐个探测，选中第一个命令可用且本 LC 能力已核验的 provider；recipe provider MUST NOT 固定为 Claude Code；选中结果 SHALL 冻结在该 operation 上（`operation.input.provider`），执行期与重试期 MUST NOT 改选

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
- **THEN** 系统 SHALL 不调用单仓登记持久化、成员 GitFinalize、成员主 checkout 写入或成员初始化 provider；成员 Git 身份与状态 SHALL 保持不变

## ADDED Requirements

### Requirement: 聚合初始化的 provider 自动选择与冻结（REQ-BOOT-07）

聚合初始化 SHALL 提供 provider 选择器：按 owner 顺序 `Pi > KimiCode > ClaudeCode > Codex` 逐个探测，在 **operation 创建之前**选中第一个可用的 provider 并**冻结**（执行期与重试期 MUST NOT 改选）。

判定分两层：**L1** 命令可用（取自 provider health 快照）；**L2** 本 LC 的 `PlanningReadOnly.launch` 为 `Confirmed` 且 snapshot 引用一致 —— L2 SHALL 与 gateway 共用**同一个**只读判定函数，MUST NOT 另写一套。

选择器 SHALL NOT 自动触发 capability 核验。无可用候选或 provider health degraded 时 SHALL **不创建 operation**。provider SHALL 走 request，MUST NOT 存进 driver 字段；未知 provider 名 SHALL fail-closed。

#### Scenario: 四家皆可用且已核验时选中顺序第一家

- **WHEN** 四家候选的命令均在本机可用，且各自在本 LC 的 `PlanningReadOnly` 能力均为 `Confirmed`
- **THEN** 选择器 SHALL 选中 `Pi`；operation SHALL 以 `pi` 作为 `input.provider` 冻结下来

#### Scenario: 命令可用性以健康快照为准

- **WHEN** 选择器判定某候选的命令是否可用
- **THEN** 判定 SHALL 取自 provider health 快照的 `available` 与其 `reason_code`（`command_missing` / `timeout` / `non_zero_exit` / `version_unparseable` / `version_too_low` / `io_error` 六值）；MUST NOT 另起一套探测口径

#### Scenario: 命令缺失时跳过该家并记录原因

- **WHEN** 首选候选 `Pi` 的命令不在本机（health `available=false`，`reason_code=command_missing`），而下一家 `KimiCode` 命令可用且能力已核验
- **THEN** 选择器 SHALL 跳过 `Pi` 并在候选表里记下 `cli_unavailable` 类原因；选中 `KimiCode`；失败面与候选表 MUST NOT 把「未安装」说成「未核验」

#### Scenario: 四家命令可用但都未核验时不创建 operation

- **WHEN** 四家候选的命令均在本机可用，但没有任何一家在本 LC 核验过能力
- **THEN** 系统 SHALL **不创建 operation**（不产生任何 provider turn），并返回「需要核验」的失败码，失败面 SHALL 建议核验「顺序中第一家命令可用的 provider」，并携带四家候选表与可点击的核验入口

#### Scenario: 已核验真实拒绝与未核验可分辨

- **WHEN** 首选候选的能力记录为 `Denied{reason}`，次选候选的能力为未核验
- **THEN** 选择器 SHALL 继续按顺序探测到次选候选；候选表 SHALL 分别如实标出「实测拒绝」与「未核验」两种状态，MUST NOT 把两者压平成同一句

#### Scenario: 重试不换人

- **WHEN** 某 operation 已冻结 provider `pi`，执行期把 provider health 改成只有 `claude_code` 可用，然后对该 operation 执行重试
- **THEN** 该 operation SHALL 继续使用 `pi`；选择器 MUST NOT 在重试路径上重跑并改选

#### Scenario: 能力判定与 gateway 同源

- **WHEN** 选择器与 gateway 对同一 capability store 的同一输入分别做 L2 判定
- **THEN** 两者的结论 SHALL 完全一致（同一 `launch_cell_state` 结果）；MUST NOT 出现选择器放行而 gateway 拒绝（或反之）的情形

#### Scenario: health degraded 时不创建 operation

- **WHEN** 发起聚合初始化时 provider health 处于 degraded 状态
- **THEN** 系统 SHALL **不创建 operation**，并返回指向「重试」与「产品缺陷」档的失败码；MUST NOT 先创建 operation 再在 spawn 前被可用性门拦下

#### Scenario: 未知 provider 名 fail-closed

- **WHEN** durable operation 记录里的 provider 名不是四种真实 provider 之一
- **THEN** provider turn SHALL fail-closed，MUST NOT 回退到任何默认 provider

#### Scenario: 候选表可解释

- **WHEN** 查询任意一次聚合初始化的 operation 记录
- **THEN** `provider_selection.candidates` SHALL 按 owner 顺序列出四家候选，每家 SHALL 带有判定依据与可执行动作提示（未安装 / 未核验 / 实测拒绝等）；候选顺序 MUST NOT 按结果重排

### Requirement: 聚合初始化与聚合索引失败面的字段化与文案单一来源（REQ-BOOT-08）

聚合初始化与聚合索引的失败面，以及 `logical-codebase-layout-guidance` 引用的登记布局失败面，SHALL 由同一个失败面文案来源产出；不同业务记录类型 MAY 通过只读事实适配输入复用该来源。系统 MUST NOT 在投影层 / DTO / 通知 / 布局引导侧分别维护同义文案。

每条失败面 SHALL 至少包含：**中文主文案**（说清是什么问题并点名对象）、**可执行动作**（点哪里 / 跑哪条命令 / 改哪个文件 / 要等多久）、**分诊标签**（`Environment` / `ProductDefect` / `RetryOnly` 三类之一）与**机器码**（原始 detail 永远保留，但只能作为次要行）。机器码 MUST NOT 作为主文案呈现。

#### Scenario: 失败面给出中文主文案与可执行动作

- **WHEN** 聚合初始化、聚合索引或上述登记布局检查产生失败事实
- **THEN** 失败面 SHALL 渲染中文主文案 + 可执行动作 + 分诊标签；原始机器码 SHALL 降级为次要行或折叠区；MUST NOT 出现「no stderr summary」；布局失败同时呈现同源原因段

#### Scenario: 三类分诊给出不同下一步

- **WHEN** 失败成因分别属于「用户环境要处理」「产品缺陷」「重试即可」三类
- **THEN** 三类 SHALL 给出各自的下一步指引：环境类指向安装/配置动作，产品缺陷类给出错误原文与「重试」（连续两次仍如此则报 issue 并附机器码），重试类给出「重试」按钮；MUST NOT 把三类混成同一句

### Requirement: 布局规则与七个对外稳定码的唯一登记（REQ-BOOT-10）

该来源 MUST 登记 `logical-codebase-layout-guidance` 的六条布局规则与以下七个**对外稳定码**：`aggregate_root_is_git` / `aggregate_root_member_outside_root` / `aggregate_root_member_symlink_escape` / `aggregate_root_nested_worktree` / `aggregate_root_overlap` / `aggregate_index_layout_unsupported` / `aggregate_index_member_coverage_failed`；MUST NOT 省略该登记。内部码 SHALL 仅作对应对外码的反查字段，不替代上述契约键。

#### Scenario: 无 action 的布局失败同源

- **WHEN** 索引/登记布局事实不带 action
- **THEN** 七码命中已知模板，三段完整，不降级为通用重试

#### Scenario: 单点修改覆盖两出口

- **WHEN** 修改一个布局模板
- **THEN** Bootstrap 投影/通知和登记 HTTP copy 同步消费，不改块3中文字面量

### Requirement: FailureCopy 五字段合同（REQ-BOOT-11）

该来源产出的 `FailureCopy` MUST 包含五个必填、非Option字段：`title_zh: String`（问题）、`reason_zh: String`（原因）、`action_zh: String`（怎么做）、`triage: FailureTriage`（Environment / ProductDefect / RetryOnly）、`machine_line: String`（原始机器码的次要展示位）。码表每一条 MUST 给出非空的三个中文字段；上述七个布局码 MUST 给出具体规则依据与可执行处置动作。无 `action` 字段 MUST NOT 成为另建来源、漏登记或缺失任一段的例外。DTO及消费链 MUST 完整透传五字段，不把reason_zh改为可选。

#### Scenario: 字段完整且跨链同构

- **WHEN** 两条链产出任一已知布局失败
- **THEN** title_zh/reason_zh/action_zh/triage/machine_line全部存在，前三字段非空，Rust DTO、HTTP details.copy与TS字段合同同构；缺reason_zh不得按缺省降级判为通过

### Requirement: 未知码的中性兜底（REQ-BOOT-12）

码表外未知码 SHALL 由同一来源给出中性中文问题、诚实的未知原因与可执行下一步；MUST NOT 编造具体成因。已知布局码及其别名 MUST NOT 因漏登记或字段缺失被当作未知码；未知码兜底不替代七个布局码的完整三段验收。

#### Scenario: 未知码中性兜底不伪装布局

- **WHEN** 输入真正码表外未知码
- **THEN** 同一来源返回中性问题/原因未确定/可执行下一步及原机器码；MUST NOT 将该中性兜底用于已登记的七个布局码

### Requirement: 失败面原始输出的脱敏、截断与成因标注（REQ-BOOT-13）

失败面 SHALL NOT 出现「no stderr summary」这类无信息文案。进入失败面的原始输出 SHALL 先脱敏：凭据类内容 SHALL 被替换为 `[REDACTED]`，身份类路径 SHALL 把 home 前缀替换为 `~`、聚合根绝对前缀替换为 `<聚合根>`；超长输出 SHALL 标注 `…[truncated]`。可判定的成因 SHALL 记为结构化 `cause`，不可判定时 SHALL 诚实标为 `unknown`，MUST NOT 编造成因。

#### Scenario: 原始输出被截断时如实标注

- **WHEN** 失败记录携带的原始输出超过截断上限（字节上限与行数上限）
- **THEN** 展示的输出 SHALL 以截断标记结尾且长度不超过上限 + 标记长度；MUST NOT 静默丢弃或展示超限全文

#### Scenario: 凭据与身份路径被脱敏

- **WHEN** 失败记录携带含 `Bearer` 令牌、`Authorization: *`、`sk-` 前缀密钥、`ghp_*` / `xoxb-*` / `AKIA…`、`--api-key` / `--token` 的值、或 key 大写后含 `KEY` / `TOKEN` / `SECRET` / `PASSWORD` 的 `k=v`，以及 home 绝对路径的原始输出
- **THEN** 展示内容 MUST NOT 包含令牌明文与密钥明文；home 前缀路径 SHALL 被替换为 `~`；MUST NOT 因脱敏而丢失命令名、文件名与错误码

#### Scenario: 最早期的失败记录也有兜底文案

- **WHEN** 失败记录既无 `stderr_summary` 也无 `action`（早期记录）
- **THEN** 系统 SHALL 给出兜底中文文案；MUST NOT 回落到「no stderr summary」这类无信息文案

#### Scenario: 成因不可判定时诚实标注

- **WHEN** 失败记录既不匹配任何已知成因判据，也无法由正则确认
- **THEN** 系统 SHALL 把成因标为 `unknown` 并列出可能的候选供用户自查；MUST NOT 把候选当成确定成因呈现

#### Scenario: 确定性成因被正确归因

- **WHEN** 失败发生在确定性步骤（skills 准备、聚合根可写性探测、skills 命令链接层校验）而非 provider turn
- **THEN** 系统 SHALL 把成因记为该步骤的结构化子码；MUST NOT 把这类失败归因为「provider 未登录」

### Requirement: codegraph 前置探测与单次报错（REQ-BOOT-09）

聚合冷启动 SHALL 在**执行入口**（machine skills 之前、第一个 provider turn 之前）对 codegraph CLI 做前置探测：命令缺失或版本不在支持区间时 operation SHALL 直接失败，`reason_code` SHALL 为 `codegraph_missing`（版本越界为 `codegraph_version_mismatch`），且 MUST NOT 把任何步骤标为 Failed。

探测 SHALL 与索引构建使用**同一份** CLI 配置，探测结果 SHALL 落 durable 证据。同一根因 SHALL **只报一次**：探测失败时 MUST NOT 产生下游 provider 步骤错误，索引构建 MUST NOT 被触发，索引相关步骤 SHALL 投影为「未开始」（无可执行动作），通知 SHALL 只产出一条。

#### Scenario: 缺 codegraph 时一条中文提示且步骤全为 Pending

- **WHEN** 冷启动环境没有 codegraph CLI
- **THEN** operation SHALL 失败且 `error.reason_code` 为 `codegraph_missing`；所有步骤 SHALL 保持 `Pending`（MUST NOT 有任何步骤被标 Failed）；失败面 SHALL 给出中文提示 + 安装命令 + 「装好后点重试」

#### Scenario: codegraph 版本越界同理

- **WHEN** codegraph CLI 存在但版本不在 `>=1.6.0 && <2.0.0` 区间
- **THEN** operation SHALL 失败且 `error.reason_code` 为 `codegraph_version_mismatch`；所有步骤 SHALL 保持 `Pending`

#### Scenario: 探测通过则正常推进

- **WHEN** codegraph CLI 可用且版本在支持区间
- **THEN** 五步 SHALL 正常推进；探测结果 SHALL 记入 durable 预检快照（`codegraph_available=true` 与版本号）

#### Scenario: 探测失败时索引构建不触发

- **WHEN** codegraph 前置探测失败
- **THEN** 聚合索引构建 MUST NOT 被启动；成员索引步与聚合索引步 SHALL 投影为「未开始」且无可执行动作

#### Scenario: 历史残留只留一张红卡

- **WHEN** 某次失败记录的 `reason_code` 为 `codegraph_missing` 或 `codegraph_version_mismatch`
- **THEN** 成员索引步与聚合索引步 SHALL 投影为「未开始」且失败详情为空；页面 SHALL 只出现一条与 codegraph 相关的失败提示

# non-interrupt-repository-bootstrap Specification

## Purpose

以无中断模式（`--no-interrupt`）执行 Cadence-skills 的四个 Claude Code 初始化命令，禁止初始化流程等待人工输入。

## Requirements

### Requirement: 无中断 Claude Code 初始化命令（REQ-BOOT-01）

系统 SHALL 使逻辑代码库场景下四个无中断命令在聚合根执行一次（聚合模式），逐仓本地化由确定性程序完成，不逐成员仓库启动 Claude 会话；传统单仓登记保持原契约。

**已初始化的聚合根 SHALL 跳过这四个命令**：当聚合根同时存在 `AGENTS.md` 与 `.claude/rules/language.md`（即 Cadence-skills 初始化已完成）时，聚合初始化 MUST 跳过 `PreCheck` / `RuleAndMcpConfig` / `OpenspecAndExamples` 三个 provider-turn 步骤（对应四条命令），只执行确定性步骤并合成、发布聚合政策。

跳过 MUST NOT 是静默忽略：operation MUST 记录跳过判定（含两个判定文件的摘要），且判定与三个 `skipped` 步骤 MUST 在同一次持久化写入中落盘。**一旦该 operation 已存在任何命令审计事实，判定窗口即关闭，MUST NOT 再改判为跳过。** 跳过式回执与「四命令全部审计通过」的完整回执互斥。

#### Scenario: 逻辑代码库聚合根执行一次

- **WHEN** 逻辑代码库场景下执行聚合初始化（聚合根未初始化）
- **THEN** 四个无中断命令 SHALL 在聚合根执行一次，逐仓本地化由确定性程序完成，不逐成员仓库启动 Claude 会话

#### Scenario: 传统单仓登记保持不变

- **WHEN** 执行传统单仓登记
- **THEN** 现有逐仓四命令与 git_finalize 行为保持原契约不变

#### Scenario: 已初始化的聚合根跳过四命令

- **WHEN** 对已存在 `AGENTS.md` 与 `.claude/rules/language.md` 的聚合根执行聚合初始化
- **THEN** 系统 MUST 不启动任何 provider turn；三个 provider-turn 步骤标记为 `skipped` 并记录判定依据；聚合政策仍 SHALL 被合成与发布，operation 最终到达 Completed

#### Scenario: 已执行过命令的 operation 不改判跳过

- **WHEN** 某 operation 已有命令审计事实（例如旧版本中断后续跑），即便判定文件齐备
- **THEN** 系统 MUST NOT 将其改判为跳过，SHALL 继续执行剩余命令

#### Scenario: 跳过但无政策发布路径

- **WHEN** 判定为跳过，但该环境没有可用的聚合政策发布通道
- **THEN** operation MUST fail-closed 为 Failed，MUST NOT 以"跳过且未发布"收口

### Requirement: 聚合初始化的生产执行

聚合初始化端点创建的 operation MUST 被真正执行，MUST NOT 仅创建可轮询记录。

#### Scenario: 提交即执行
- **WHEN** POST /api/projects/{pid}/logical-codebase/initializations 成功
- **THEN** 服务端后台执行初始化流程（machine skills → 聚合预检 → provider turns → 完成），使用生产级依赖（真实 skills 准备与聚合根预检，非空实现）；operation 最终到达 Completed 或 Failed

#### Scenario: 取消在步骤边界生效
- **WHEN** 初始化执行中调用 cancel
- **THEN** 正在执行的步骤完成后不再推进，operation 进入 Cancelled；取消触发不依赖轮询

#### Scenario: 重启恢复语义
- **WHEN** 服务重启后查询 Running 且无活跃执行租约的 operation
- **THEN** 标记为 Failed 附 interrupted 原因；人工重触发创建新 operation（新幂等键），不复活旧记录

#### Scenario: 执行租约共享
- **WHEN** 任意 handler 查询或取消初始化
- **THEN** 通过进程级共享的执行注册表（含取消令牌）判定活跃状态，不因 handler 实例隔离而误判中断

### Requirement: 根 recipe 政策正文发布与收口（REQ-BOOT-05）

逻辑代码库根 recipe SHALL 在既有五步内将生成的 canonical 根规则确定性聚合为最终政策正文，发布正文、原始 UTF-8 字节 SHA-256 digest 与 revision；同一正文 SHALL 在 canonical root 下的 `policy_id` 安全相对路径可读。最终 receipt SHALL 冻结同一 policy digest 和本次根 rule digest；MUST NOT 用固定自举正文、provider 输出摘要或仅命令成功替代最终发布。

#### Scenario: 全新 LC 无 fixture 完成首建

- **WHEN** 全新 LC 从产品登记与初始化入口执行五步 root recipe，成员与索引事实经既有产品链准备，且没有手工 policy/receipt/index seed 或 fixture 政策物化
- **THEN** 四命令 SHALL 在 canonical root 执行，根规则 SHALL 确定性产生最终政策正文；根下 `policy_id` 文件与权威 artifact.policy_text SHALL 字节一致，其 digest SHALL 等于最终 receipt 的 policy digest，receipt 的 rule digest SHALL 对应本次根规则；根政策、成员索引与聚合索引均就绪后，投影 SHALL 返回 `planning_ready=true`

#### Scenario: 发布在原有步骤收尾内完成

- **WHEN** 根规则、MCP 及规则示例命令在既有 recipe 中成功完成并进入最终收口
- **THEN** recipe SHALL 在 `RuleAndMcpConfig` 产物流或其既有收尾中完成最终政策发布并复验正文与摘要，再冻结最终 receipt；LC SHALL 保持 MachineSkills、AggregatePreflight、PreCheck、RuleAndMcpConfig、OpenspecAndExamples 五步原序，不新增第六步、第五条命令或成员级初始化会话

#### Scenario: 根入口引用规则时仍发布完整政策材料

- **WHEN** 本次生成的 canonical 根入口列出或引用根规则目录，而非包含所有规则全文
- **THEN** 最终政策 SHALL 以确定性来源顺序保留根入口及根规则原文，不仅复制入口引用或 provider 输出摘要；MCP/settings 凭据、成员仓规则、未启用的规则示例模板 SHALL 不因聚合而成为政策约束，CLAUDE/provider 兼容副本 SHALL 不重复激活同一规则

#### Scenario: 根规则缺失或政策发布失败

- **WHEN** 收口时根规则入口缺失、不可读、非有效 UTF-8 或无有效内容，或者政策路径越界、symlink 逃逸、文件冲突、持久化失败、身份/正文/摘要不一致
- **THEN** 系统 SHALL 保留原 operation 的可诊断失败或等待事实和允许的显式重试，MUST NOT 签发最终成功 receipt 或将该操作投影为根规则/政策已完成；MUST NOT 用桩正文补齐，成员仓主 checkout SHALL 不受发布副作用影响

#### Scenario: 同一完成操作重放与显式新配方

- **WHEN** 再次提交已完成 operation 的同一幂等键、查询旧 operation，或用户显式执行具有新操作身份的 recipe
- **THEN** 同 key SHALL 沿用既有终态 conflict，不重跑或发布新 revision，旧 receipt 与 revision SHALL 保持可查询且不变；实际执行的新 recipe SHALL 产生高于当前 artifact 的 revision、新 `policy_id` 正文路径和新 receipt，即使政策正文未变化也不得重用旧 operation；旧 receipt 与历史正文 SHALL 保留

#### Scenario: 命令成功后政策发布中断

- **WHEN** 本 operation 的最后一条命令已经成功且有 Allowed 审计，随后在正文、artifact 或最终 receipt 发布期间中断，并由用户执行既有显式继续
- **THEN** 系统 SHALL 复验同 operation/canonical root 与来源摘要，复用已审计命令及同一候选政策 revision/时间，只补齐未完成发布；MUST NOT 重复启动已成功命令、覆盖旧审计 receipt 或为同 operation 增加 revision；来源/身份漂移 SHALL 冲突停等

#### Scenario: recipe 完成但索引未就绪

- **WHEN** 政策与 receipt 已发布一致，但成员索引或 aggregate index active 尚未就绪
- **THEN** 系统 SHALL 继续按原有投影返回 `planning_ready=false` 与相应索引准备/继续操作，不以 recipe Completed 代替整体 readiness

#### Scenario: 单仓行为不变

- **WHEN** 运行传统单仓初始化
- **THEN** 单仓逐仓四命令、`git_finalize`、policy、cwd 与 operation 契约 SHALL 保持不变，不要求 canonical LC root 的政策发布

### Requirement: 自举桩识别与存量政策显式重冻结（REQ-BOOT-06）

readiness SHALL 仅以当前权威 artifact 正文等于已知固定自举桩（或其已校验 digest）识别未迁移政策，命中时 SHALL 返回 `planning_ready=false` 与 rules/policy 等待原因，即使旧 receipt 有效。allowed actions SHALL 保留 `Retry`，detail/notice SHALL 指向既有初始化产品 API，以新 `idempotency_key` 发起根 recipe；非桩政策 SHALL 沿用原有全部判定，MUST NOT 自动重跑、直接改写旧 receipt 或静默迁移。

#### Scenario: 桩正文加旧有效 receipt 仍等待

- **WHEN** 当前政策正文为固定自举桩，旧 root recipe operation 已完成、receipt 与 policy/rule digest 一致，其余四个 readiness 步骤已完成
- **THEN** rules/policy SHALL 为等待，`planning_ready` SHALL 为 false，allowed actions SHALL 包含 `Retry`，detail/notice SHALL 明示 `POST /api/projects/{pid}/logical-codebases/{lcid}/initializations` 与新的 `idempotency_key`；通用 bootstrap action dispatcher 与 UI 路由 SHALL 保持原状，不声明 Completed 态有可见启动按钮或 Retry 按钮会直接创建 recipe；只读查询 SHALL 保留全部旧事实且零写入、零 provider 启动

#### Scenario: 真正文继续按原判定就绪

- **WHEN** 当前政策正文不同于固定自举桩，operation、receipt、root rule digest、成员索引与聚合索引均满足原有一致性判定
- **THEN** rules/policy SHALL 完成且 `planning_ready=true`；系统 SHALL 不以 revision 是否为 1、receipt 年龄或额外迁移标记改变就绪结论

#### Scenario: 用户显式重跑并重冻结

- **WHEN** 存量桩 LC 处于等待，用户调用既有初始化产品 API 并明确携带新 `idempotency_key` 发起新 operation 的根 recipe
- **THEN** 系统 SHALL 保留旧 operation/receipt，按原五步执行、发布真正政策正文及更高 revision、冻结新 receipt，再沿原有 readiness 流程恢复就绪；迁移 E2E SHALL 从该 API 实际完成，而非以通用 bootstrap Retry action 的 accepted/waiting 返回代替；未成功时 SHALL 保留等待或失败，不要求人工改 JSON/物化正文

#### Scenario: 已生成根以旧 receipt 证明归属后重跑

- **WHEN** 用户对 Completed 存量桩 LC 调用初始化 API（新 key），同一 LC/canonical root 的旧完成 receipt 证明四命令全 Allowed，receipt.rule_digest 与现存 `AGENTS.md` 一致，现存 `CLAUDE.md`（若有）与同一命令审计快照摘要一致
- **THEN** recipe 的 AggregatePreflight SHALL 复用该已证明的根入口继续重跑，仍核验成员、canonical/non-Git root、越界、symlink、worktree 与重叠；首次登记的用户文件冲突检查、`.aria` 冲突、admission/gateway 与五步顺序 SHALL 不放宽

#### Scenario: 根归属无证据或摘要不符

- **WHEN** API 重跑遇到已有 AGENTS/CLAUDE，但旧 receipt 缺失、不是同 LC/root、四命令不全 Allowed、现存入口与冻结摘要不符，或存在未证明的用户文件/symlink
- **THEN** 系统 SHALL 保持根所有权冲突等待或失败，不删除或覆盖文件、不手工改 receipt、不启动后续 recipe 命令；首次注册 SHALL 继续按原冲突契约拒绝

#### Scenario: 首次自举临时政策不成为最终事实

- **WHEN** 全新 LC 在根规则生成前使用既有有效 bootstrap 相位执行 recipe
- **THEN** 临时政策 SHALL 只为该受凭据约束的初始化相位提供材料；最终收口 SHALL 发布非桩正文，MUST NOT 让临时政策签发最终成功 receipt 或普通 planning readiness

#### Scenario: 唯一新增消费谓词

- **WHEN** 当前 artifact 未命中桩识别，或下游 resolver、admission、gateway、envelope 消费新发布的 policy 引用
- **THEN** 系统 SHALL 保持已有 operation 生命周期、policy/receipt/rule digest、root identity、索引与 provider capability 判定，不新增消费者旁路或放宽准入

### Requirement: 聚合重新初始化入口

系统 SHALL 提供聚合「重新初始化」入口：对已存在（含已初始化）的逻辑代码库，**无视初始化检测结果**强制执行完整五步（含四条无中断命令），用于 Cadence-skills 更新后重装。

#### Scenario: 已初始化根上的重新初始化

- **WHEN** 对已初始化的聚合根请求重新初始化
- **THEN** 系统 SHALL 执行完整五步并运行全部四条命令，MUST NOT 因检测到已初始化而跳过；最终生成完整 recipe 回执

#### Scenario: 重新初始化与普通初始化通道一致

- **WHEN** 重新初始化请求与普通初始化请求并发
- **THEN** 两者 MUST 遵循同一套通道/幂等/并发仲裁语义，不得出现并行双跑

### Requirement: 聚合根 recipe 与 readiness 事实闭环（REQ-BOOT-03）

逻辑代码库的 root-cwd recipe SHALL 复用四条无中断命令的取消、超时、输出摘要与步骤恢复语义，并将其映射到既有五步聚合初始化 operation。

**recipe provider SHALL NOT 固定为 Claude Code。** 聚合初始化 SHALL 在 operation 创建之前按 owner 顺序（`Pi > ClaudeCode > Codex`）自动选择一个本机可用的 provider，并把它**冻结**在该 operation 上；执行期与重试期 MUST NOT 改选。选中的 provider SHALL 记入 `operation.input.provider`，候选表 SHALL 记入 `operation.provider_selection`。provider 选择 SHALL NOT 放宽 gateway 的 capability 准入门。

#### Scenario: 四条命令按所选 provider 的 skill 调用语法生成

- **WHEN** 聚合初始化选中某个 provider 并执行 root recipe
- **THEN** 四条命令的调用形态 SHALL 按所选 provider 的 skill 调用语法生成（见 REQ-BOOT-16）；四条命令的**顺序**、**参数**、取消/超时/输出摘要语义 SHALL 与既有实现一致

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

### Requirement: 聚合初始化自举相位凭据（REQ-BOOT-04）

系统 SHALL 为唯一的“根规则尚未生成”自举例外使用内部不可伪造的 bootstrap phase credential。凭据 SHALL 由当前 durable aggregate initialization operation 的 Running 状态、当前 step、operation input digest、LC 身份和 canonical root 共同派生，并在 provider spawn 前重新核验；普通 provider session 不得自行声明该相位。

#### Scenario: 新 LC 使用有效自举凭据

- **WHEN** 新 LC 的 Running operation 正处于允许执行根 recipe 的步骤，且凭据的 LC、root、step 与 input digest 均匹配
- **THEN** admission SHALL 仅豁免“根规则尚未生成”检查，同时继续校验 authority、临时 policy、capability、gateway、cwd、target 与可用性后才允许 provider spawn

#### Scenario: 凭据缺失或漂移

- **WHEN** provider 请求缺少凭据，或 operation 已 Completed/Failed/Cancelled，或 step、input digest、LC/root 与当前 durable operation 不一致
- **THEN** provider SHALL 在 spawn 前被拒绝并生成 fail-closed 等待/诊断，系统不得以普通 session 或宽松权限替代自举路径

#### Scenario: 普通会话不可绕过根规则

- **WHEN** 普通 planning、coding 或 review session 在根规则缺失、替换或摘要漂移时启动
- **THEN** 系统 SHALL 零启动 provider，返回根规则/policy 准备或重试操作，且不得自动取得 bootstrap 例外

### Requirement: 聚合初始化的 provider 自动选择与冻结（REQ-BOOT-07）

聚合初始化 SHALL 提供 provider 选择器：按 owner 顺序 `Pi > ClaudeCode > Codex` 逐个探测，在 **operation 创建之前**选中第一个可用的 provider 并**冻结**（执行期与重试期 MUST NOT 改选）。

判定分两层：**L1** 命令可用（取自 provider health 快照）；**L2** 该 provider 在**机器级** capability 记录中的 `PlanningReadOnly.launch` 为 `Confirmed` 且 snapshot 引用一致 —— L2 SHALL 与 gateway 共用**同一个**只读判定函数，MUST NOT 另写一套。

#### Scenario: 选择器 SHALL NOT 自动触发 capability 核验

- **WHEN** 选择器评估候选 provider 的可用性
- **THEN** 选择器 SHALL NOT 自动触发 capability 核验

#### Scenario: 无可用候选或 health degraded 时不创建 operation

- **WHEN** 候选序列中无可用 provider，或 provider health 处于 degraded
- **THEN** 系统 SHALL **不创建 operation**

#### Scenario: provider 的落位与未知 provider 名

- **WHEN** 选中 provider 并写入 operation
- **THEN** provider SHALL 走 request，MUST NOT 存进 driver 字段
- **WHEN** provider 名未知
- **THEN** 系统 SHALL fail-closed

#### Scenario: 被选中的 provider 还须通过 root-recipe 相位准入

- **WHEN** 被选中的 provider 要执行 root recipe
- **THEN** 它 SHALL 另行通过 root-recipe 相位准入（见 REQ-BOOT-15）：该准入**不再要求** provider 是 Claude Code，但其凭据三道检查（重核验通道在场 / 版本钉定 / 凭据对 durable Running operation 重核验）SHALL 一条不减

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

### Requirement: root-recipe 相位的 provider 无关准入（REQ-BOOT-15）

root-recipe 相位的 provider 准入 SHALL **不要求** provider 是 Claude Code：任何通过普通 capability 准入（`Confirmed` + snapshot 一致）的 provider SHALL 能进入 root-recipe 相位。

**例外（owner 2026-11-05 裁定）**：`KimiCode` 暂**不参与**聚合初始化的 provider 候选序列（判定与依据见下方 Scenario「Kimi 不进入聚合初始化候选」）；缺口已记录在独立 change `kimi-root-recipe-support`，后续另开 change 解决；**本例外 SHALL NOT 被当作永久形态**。

准入的其余检查 SHALL **一条不减**，且 SHALL 继续 fail-closed。

#### Scenario: 凭据三道检查一条不减且继续 fail-closed

- **WHEN** provider 申请进入 root-recipe 相位
- **THEN** 凭据重核验通道 SHALL 在场（缺失 ⇒ 拒绝，不放行）；已交付 recipe 证据的版本 SHALL 与记录版本钉定一致（漂移 ⇒ 拒绝）；凭据 SHALL 每次对**该逻辑代码库的** durable Running operation 重核验（operation 终态 / step 推进 / digest 或 root 漂移 ⇒ 拒绝）

#### Scenario: 不放宽普通会话的准入强度

- **WHEN** 本 requirement 生效
- **THEN** 它 MUST NOT 放宽普通会话的准入强度，也 MUST NOT 让 root recipe 的能力证据被普通会话复用（隔离契约不变）

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

零变更失败的诊断 SHALL 同时携带**失败事实**（provider、step、零变更）与 **provider 的真实输出**（该 turn 的输出尾部，含 Cadence-skills 脚本的报告原文）。理由：零变更本身不说明原因——「命令未被识别」「命令执行了但失败」「命令按契约幂等无变更」三种情形的 provider 输出各不相同；诊断只有 `zero_change` 一句时，用户无法分辨是哪一种（本仓 2026-11-12 真机：pre-check 脚本某个 phase 失败 → `--no-interrupt` 中止 → 聚合根零落盘 → 界面只显示 `root recipe turn produced no workspace change (provider=pi, step=pre_check, zero_change=true)`，脚本的 JSON 报告整段丢失）。

provider 没有产出任何输出时，诊断 SHALL 只保留失败事实，MUST NOT 用无信息量的兜底串冒充真实输出。

provider turn 的输出超预算时，系统 SHALL 保留输出的**头部与尾部**（MUST NOT 只留头部）：脚本报告与失败原文都在收尾处，只留头部会把最有诊断价值的事实整段截掉。

**幂等重跑例外**：Cadence-skills 四条命令的契约就是幂等（脚本按契约报告 `all-skipped` / 0 created / 0 updated、产物无变化）。因此当工作区零变更**且**该 turn 的**工具结果**里出现脚本的成功报告（`overall=success`）时，系统 SHALL 判该 step **成功**——命令确实执行了、按契约没有产物变化。命令没被执行（没有脚本报告）或脚本报告失败（`overall=failed`）时，零变更 SHALL 仍判失败。脚本报告结论 MUST 只从**工具结果原文**读，MUST NOT 采信模型自述（模型可能复述或编造报告内容）。

本 requirement SHALL 复用既有 receipt auditor 的零变更判据，SHALL NOT 新增第二套「有无变更」的判定逻辑。

#### Scenario: 零变更的 recipe turn 判失败

- **WHEN** root recipe 的某个 step 结束时 receipt 判为零动作
- **THEN** 该 step SHALL 判为失败，operation SHALL 记录可诊断失败事实（provider、step、零变更）；SHALL NOT 把该 step 记为成功、SHALL NOT 继续推进后续 step

#### Scenario: 零变更但脚本报告成功判成功

- **WHEN** 某个 root recipe step 的 turn 结束时 receipt 判为零动作，且该 turn 的工具结果里出现脚本的成功报告（`overall=success`）
- **THEN** 该 step SHALL 判为成功（命令确实执行且按契约无产物变化），SHALL NOT 以零变更判失败

#### Scenario: 脚本报告失败仍判失败

- **WHEN** 某个 root recipe step 的 turn 结束时 receipt 判为零动作，且该 turn 的工具结果里出现脚本的失败报告（`overall=failed`）或没有脚本报告
- **THEN** 该 step SHALL 判为失败，诊断 SHALL 携带脚本报告原文

#### Scenario: 有变更的 recipe turn 照旧成功

- **WHEN** root recipe 的某个 step 结束时 receipt 判出实际变更
- **THEN** 该 step SHALL 照既有语义成功；本 requirement SHALL NOT 放宽任何既有成功判据

#### Scenario: 既有成功判据不被改写

- **WHEN** 既有 receipt auditor 的动作判定逻辑被复核
- **THEN** SHALL 复用它；MUST NOT 出现第二套「有无变更」判定

#### Scenario: 零变更诊断携带 provider 的真实输出

- **WHEN** 零变更失败的 turn 产出了输出（例如 Cadence-skills 脚本报告某个 phase 失败并给出原文）
- **THEN** 失败诊断 SHALL 同时包含零变更事实与该输出尾部，用户从诊断即可判断命令是否被执行、失败原因是什么；MUST NOT 只显示 `zero_change` 一句

#### Scenario: provider 无输出时诊断只保留失败事实

- **WHEN** 零变更失败的 turn 没有任何输出
- **THEN** 诊断 SHALL 只保留失败事实，MUST NOT 出现无信息量的兜底串

#### Scenario: 超长 turn 输出保留收尾

- **WHEN** provider turn 的输出长度超过诊断预算
- **THEN** 保留下来的输出 SHALL 同时包含头部与尾部，收尾（脚本报告 / 失败原文）MUST NOT 被整段截掉

### Requirement: Kimi 能力边界与候选顺序的用户面告知（REQ-BOOT-18）

系统 SHALL 在**两个用户可见入口**用**红字**如实告知聚合初始化准备阶段的 provider 边界与候选顺序。

- 红字 SHALL 使用危险色 token（`--aria-danger`；带框提示用 `--aria-danger-soft` 底 + `--aria-danger` 边与字）；SHALL NOT 用普通灰字或提示黄字掩盖该边界。
- 该告知 SHALL 与**实际候选序列一致**（见 REQ-BOOT-15 的 Kimi 例外与 REQ-BOOT-03/07 的顺序）；SHALL NOT 声称 Kimi Code 可用于该阶段。
- 该告知是**临时形态**：Kimi 支持该阶段的缺口修复后（change `kimi-root-recipe-support`），两处红字 SHALL 随之移除或改写。

#### Scenario: 两处红字的位置与逐字文案

- **WHEN** 用户打开任一入口
- **THEN** **Provider 管理抽屉** SHALL 在作用域说明之后、能力卡之前显示；**逻辑代码库管理抽屉（聚合初始化卡）** SHALL 在标题行状态之后、步骤列表之前显示
- **THEN** 两处文案 SHALL 逐字为「聚合初始化准备阶段暂不支持 Kimi Code；该阶段 provider 顺序：Pi > Claude Code > Codex。」

#### Scenario: Provider 管理抽屉的红字

- **WHEN** 用户打开 Provider 管理抽屉
- **THEN** 作用域说明下方 SHALL 出现红字「聚合初始化准备阶段暂不支持 Kimi Code；该阶段 provider 顺序：Pi > Claude Code > Codex。」

#### Scenario: 聚合初始化卡的红字

- **WHEN** 用户在逻辑代码库管理抽屉查看聚合初始化卡
- **THEN** 标题行状态之后、步骤列表之前 SHALL 出现红字「聚合初始化准备阶段暂不支持 Kimi Code；该阶段 provider 顺序：Pi > Claude Code > Codex。」

#### Scenario: 红字与实际候选一致

- **WHEN** 聚合初始化的 provider 候选序列发生变化
- **THEN** 两处红字 SHALL 随之更新，SHALL NOT 继续显示已过时的顺序或已支持的 provider

### Requirement: provider run 体积预算与四层门禁（REQ-BOOT-19）

provider run 的每个**未装箱** async 对象 SHALL 受**按归属分层的预算**（超其所属档位硬线即回归、须断言阻断；四档见场景）；入口外壳 SHALL ≤32,768 B。

体积 SHALL 以**未装箱类型**度量，SHALL NOT 以 `Pin<Box<_>>` 指针大小代替（恒绿即假门禁）。

入口 SHALL 只构造**小 capture 外壳**，task body SHALL 首次 poll 内被装箱（构造落堆），SHALL NOT 在 WS inbound 深栈上构造大对象。

门禁 SHALL 由 A（体积）/ B（显式 2 MiB worker + 三守卫与 run 隔离）/ C（真实 WS 深链）/ D（真实制品进程）四层构成，各层 SHALL NOT 互相替代；**C 层的 2 MiB SHALL NOT 被任何栈档位豁免**。

#### Scenario: 入口外壳小且构造落在堆上

- **WHEN** WS handler 发起一次 provider run
- **THEN** 入口只构造 ≤32,768 B 的未装箱外壳；task body SHALL NOT 在 inbound 深栈上构造

#### Scenario: 超硬线的对象被断言阻断

- **WHEN** 某个未装箱 future 的体积超过**其所属档位的硬线**
- **THEN** 体积门禁 SHALL 失败并报出符号、实际字节、**所属档位**与 profile/target/toolchain

#### Scenario: 四层门禁各证其责

- **WHEN** 门禁报告通过
- **THEN** A 层已证未装箱对象体积 ≤ 预算；B 层已证显式 2 MiB worker 上可运行且三守卫与新旧 run 隔离成立；C 层已证真实 axum/socket/inbound 深链 + 可信 provider start + 完整帧 + durable 进展 + 取消后再生成；D 层已证真实制品进程在同一合同下成立

#### Scenario: C 层永不被栈档位豁免

- **WHEN** 生产运行时栈被配置为 8 MiB 或更高
- **THEN** C 层 SHALL 仍以显式 2 MiB worker 执行真实 WS 回归

#### Scenario: 退出码 0 不构成通过

- **WHEN** 动态场景仅以退出码 0 结束
- **THEN** 该场景 SHALL 判失败（未证明服务存活、实际启动、完整帧与 durable 进展）

#### Scenario: debug 数字不得冒充 release

- **WHEN** 体积证据只来自 debug profile
- **THEN** 该证据 SHALL 标注为 debug，且发布前 SHALL 以 release 制品复核

#### Scenario: 四档预算与过渡档归属

- **WHEN** 门禁判定某对象所属的预算档位
- **THEN** 四档为：**P0 核心** 65,536 B · **P0 spawn/准入侧** 32,768 B · **宏体过渡档** 131,072 B · **P1 engine 侧过渡档** 262,144 B
- **AND** 宏体档与 P1 engine 档的对象其源码在写集外（宏文本未迁入 `provider_run/`、engine 侧文件未治理）⇒ SHALL 已登记归属独立 change 与收紧条件
- **AND** P1 engine 档内超过 131,072 B SHALL NOT 判回归，但 SHALL NOT 被当作已收敛

#### Scenario: 过渡档对象必须登记归属

- **WHEN** 某对象按宏体档或 P1 engine 过渡档判绿
- **THEN** 该对象 SHALL 已登记其归属的独立 change 与收紧条件；SHALL NOT 被当作已收敛

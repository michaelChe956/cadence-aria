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

逻辑代码库的 root-cwd recipe SHALL 复用四条无中断命令的取消、超时、输出摘要与步骤恢复语义，并将其映射到既有五步聚合初始化 operation；recipe provider SHALL 固定为 Claude Code。LC 根准入冻结 canonical root 后、Claude Code recipe operation 启动前 SHALL 完成所选 Codex/Kimi trust 的独立前置登记与核验。该登记不属于五步 operation、不插入步骤之间且不新增第六步；它是五步 recipe 的硬前置门，任一 trust 未 Ready 时五步 operation 不创建/不启动并进入可重试 fail-closed waiting。全部 trust Ready 后才启动五步 recipe。命令成功、根级产物 receipt、根 policy/rule 摘要和 bootstrap 投影 SHALL 独立记录。recipe 完成本身 MUST NOT 被视为 `planning_ready`，直到规则/policy、成员索引和聚合索引 active 等投影事实均完成。

#### Scenario: 四命令按根顺序执行

- **WHEN** LC 根准入已确定并冻结 canonical root，所选 Codex/Kimi trust gate 已登记并核验后启动新 LC 的 Claude Code 五步聚合初始化 recipe
- **THEN** SHALL 按 MachineSkills→AggregatePreflight→PreCheck→RuleAndMcpConfig→OpenspecAndExamples 原样推进；四条 `/pre-check --no-interrupt --upgrade 用大陆镜像`、`/rule-config --no-interrupt`、`/mcp-configuration --no-interrupt`、`/project-rules-examples --no-interrupt` SHALL 按固定顺序在根执行；任一 trust 登记失败时五步 recipe 不启动并保留可重试等待事实，任一命令失败、取消或输出不可审计时 SHALL 停止 recipe/后续命令并保留失败事实。

#### Scenario: recipe 成功但 readiness 材料未齐

- **WHEN** 四条命令已成功但根 rule/policy receipt、MemberIndex 或 AggregateIndexActive 投影缺失或摘要漂移
- **THEN** operation MAY 为 Completed，但 `planning_ready` SHALL 保持非就绪并返回可操作的核验/继续等待项；普通 provider session SHALL 不得仅凭 recipe Completed 放行

#### Scenario: 根 recipe 不产生单仓 Git 副作用

- **WHEN** root-cwd recipe 为包含多个独立 Git 成员的 LC 执行
- **THEN** 系统 SHALL 不调用单仓登记持久化、成员 GitFinalize、成员主 checkout 写入或成员初始化 provider；成员 Git 身份与状态 SHALL 保持不变

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

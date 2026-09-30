# Design

## Context

本设计承接 `proposal.md`，将方案 v1.2（`cadence/designs/2026-09-28_方案设计_真实链缺口修复总方案_v1.2.md`）§2 的 S20–S23、§3.1–§3.3、§4.1 的 GAP-A–D、§5.3、§5.5、§6.2 的 A03/A04 与 §8 取舍落为 C4 架构边界。现有实现已经有 `RepositoryRouting`、逻辑代码库的 selection/manifest/policy/index store、registration/initialization operation、identity migration journal 以及 provider capability 检查；现有 OpenSpec 主规格分别定义了成员登记的零 Git 副作用与批次状态、聚合索引的 single-writer/快照/last-known-good、代码库 kind 路由和 policy envelope。缺口在于这些事实没有共享唯一解析入口，首建准入依赖尚未存在的 active index，Failed journal 没有普通列表之外的修复面，且规则准备与真实 provider 准入不一致。

本 change 只定义 LC 冷启动/repair 的契约和实现边界。单仓自动化入口、plan/coding 全链、Pi 初始化 recipe 仍分别属于 C5 或延期边界；C2 只能消费本设计提供的 authority/policy 引用，不能另猜路径。

## Goals / Non-Goals

**Goals:**

- 以一个 resolver 归一单仓与 LC 的 authority 解析，产出稳定目标、policy/selection/index 引用和冲突诊断。
- 以既有 registration/index operation 和步骤事实表达可重入的 identity→manifest/checkout→rules/policy→member index→aggregate active 冷启动，并保持 GET 纯投影。
- 提供不依赖普通仓库列表的 Failed identity journal 诊断、digest 核验、mapping 提交与安全继续操作。
- 在 provider/索引准入前验证实际规则材料、policy digest 与 capability/gateway 条件；缺失时以通知和受限产品操作停等。
- 让所有等待项复用现有持久化事实与通知投影，用户操作后回到原编排链，不引入通用 operation 状态机。

**Non-Goals:**

- 不建设全局 durable operation、owner/fence/epoch、自动 takeover 或第二套索引/身份状态机。
- 不在只读查询中启动 provider、checkout、registration、index 或修复写入；不自动吞并 mapping 冲突。
- 不改变登记阶段零成员仓 Git 写入、不放宽 LC 单 target/gateway 约束，不用 capability 记录替代真实能力。
- 不实现单仓自动化、C2 的 coding 断连/验证分诊、C5 的 role-chain 入口或延期的 Pi recipe。

## Decisions

### 1. 唯一 authority resolver 与 fail-closed 路由

在既有 `RepositoryRouting` 及 LC stores 之上建立一个唯一解析合同，输入显式 codebase kind、logical codebase/member 或 repository identity，输出 authority root、canonical target、manifest/checkout、policy、selection、member index/aggregate index 引用及 revision/digest。LC 身份只允许读 LC 子树；单仓身份只允许读合法物理仓布局。解析发现历史项目级布局、多个候选、路径越界、canonical/git-dir/source identity 不一致时返回结构化冲突，产品只展示迁移/核验动作，不提供按“最新”或路径存在性猜测的 fallback。

**取舍：** 选择共享解析合同而非在各 store 逐一加条件分支，避免 selection/policy/index 再次分叉；代价是历史布局无法自动兼容，必须由用户核验。`RepositoryRouting::Legacy` 的源码枚举名不在本 change 重命名，产品文案仍使用单仓。

### 2. 冷启动按依赖步骤、checkpoint 和幂等操作推进

冷启动复用现有 registration/initialization/index operation 的 durable 记录，不创建新的全局工作流。按依赖顺序创建或读取步骤事实：

1. **identity**：确认 LC、成员来源和 authority identity；冲突直接进入等待。
2. **manifest/checkout**：原子创建/校验 manifest 与可用 checkout，保留登记零 Git 副作用。
3. **rules/policy**：生成/校验实际 provider 消费的聚合 policy、成员引用和 capability 预检。
4. **member index**：对已确认成员建立/刷新索引材料，记录 revision/dirty 快照。
5. **aggregate index active**：在前置材料满足后取得 single-writer，构建并验证覆盖/排除/代表性查询，成功才转 active/PlanningReady。

每步使用既有稳定 operation/command identity、expected revision 和 checkpoint；安全本地步骤可重试，外部副作用未知时停等确认/核验。用户重复同一 command key 返回原结果，错误对象或过期 revision fail-closed。GET 只投影这些 durable 事实和允许动作，绝不隐式执行步骤。

**解除鸡生蛋：** active aggregate index 不是成员准入的前置条件；缺失时显示 member index/aggregate index 缺口并由显式首建动作推进。首建失败保留失败事实；有 last-known-good 时以 degraded 只读，首次失败无 active 则为 missing/failed，不伪造 active。

### 3. identity journal repair 独立于普通列表

repair/diagnostic 读取 identity journal、registry 和 authority metadata 的低层事实，不经失败的普通仓列表或会触发 fallback 的解析路径。投影展示 source digest、read mode、completed keys、候选 mapping、冲突和影响范围。digest 可证明一致的安全 prefix 可由用户确认继续；冲突/不一致必须提交明确 mapping 后重新核验，核验前不写当前映射、不切 read authority。成功操作追加 repair 事实并调用原迁移/登记继续路径，原 Failed journal、历史 JSON 和审计不可删除或覆盖。

**取舍：** 选择可操作停等而非后台自动选择 mapping；在恢复速度与身份安全之间优先避免错误仓读取，代价是冲突必须由人裁决。

### 4. rules/policy/capability 预检作为真实准入门

rules/policy 解析和 capability/gateway 检查必须针对实际 provider 将消费的材料和角色链执行。resolver 输出的 policy reference 携带 authority root、revision/digest；SessionPolicyEnvelope 在 spawn 前冻结 target、policy 和 capability snapshot。缺失/非法/过期/不一致进入等待投影，操作面只能是准备、核验或安全重试；不得写一条 capability 记录假装支持、复制一次成员文件作为永久事实或绕过 gateway。准备成功后由原 registration/index/provider 编排器继续，禁止额外的 fallback authority。

### 5. 事实先落盘、通知后投影

步骤开始/完成/失败、checkpoint、快照、journal repair 和操作结果先写入现有 operation/journal/store，再生成 inbox/系统通知。通知包含 LC/member/step/object identity、已完成步骤、失败原因、外部副作用、按钮语义和下一阶段；投递失败时只影响观察面，详情查询仍可补读同一事实。REST 与页面按钮调用同一应用服务，不能通过直接编辑 JSON、删除 journal、修改 consumed/findings 或重启服务恢复。

### 6. 与 C1/C2/C5 的边界

C4 只交接稳定 authority/policy/revision/digest 和可查询的冷启动/repair 结果。C1/C2/C5 不得重新实现路径猜测；C2 的政策读取消费 C4 resolver 的引用，C5 保持单仓入口及 Claude recipe，不把单仓自动化塞回 LC bootstrap。跨 change 的共享字段必须扩展既有 DTO/journal/枚举而非新增平行状态机。

## Failure Handling and Migration

- **旧布局/authority 冲突：** fail-closed，保留当前事实，展示迁移/核验；确认前不合并、不切读、不启动 provider。
- **步骤失败：** 保留失败原因和已完成 checkpoint；仅允许与对象/expected revision 匹配的准备、继续或安全重试。未知 provider 副作用转人工确认/重新绑定，不盲重。
- **首次 index 失败：** 无 active 时保持 missing/failed；有 last-known-good 时保持 degraded 只读，规划带告警；只有完整验证成功才转 active。
- **identity repair 冲突：** 安全 prefix 需用户确认；mapping 冲突必须提交映射并重新核验；不删 journal、不直接改权威 JSON。
- **规则/capability 缺失：** provider spawn 前阻塞并通知；补齐后复用原链。gateway 不支持不回落、不伪造能力。
- **旧数据迁移：** 只读现有记录并通过产品 repair/核验补充新事实；可明确核验的记录复用其稳定 identity，无法核验的进入等待，不批量猜测、不静默变更成员/target。

## Contract Traceability

| 方案 v1.2 依据 | 本设计落点 |
|---|---|
| §2 S20（selection/manifest/policy/index） | 决策 1/2 的唯一 resolver 与五步冷启动 |
| §2 S21（identity migration journal） | 决策 3 的独立诊断/repair 与 digest/mapping 核验 |
| §2 S22（规则硬依赖/capability bootstrap） | 决策 4 的真实材料预检、等待和禁止伪能力 |
| §2 S23（registration/initializer） | 决策 2/4 的复用既有 operation，保留 Claude 正常路径和 Pi defer |
| §3.1–§3.3（停等、人到点、操作安全） | 决策 2/5 的 checkpoint、稳定命令键、通知和 fail-closed |
| §4.1 GAP-A–D | 决策 1–4；旧布局、index 鸡生蛋、Failed journal、规则缺失逐项覆盖 |
| §5.3 C4 与 §5.5 共享边界 | Goals、决策 6 及非目标，C4 只交接 resolver/policy，不扩单仓/C2 |
| §6.2 A03/A04 与 §6.3 零绕行 | 冷启动无 seed、注入中断可继续、mapping 未确认不切读、零手工铺底/直改数据 |
| §8 durable operation vs 步骤操作面 | 不引入新通用平台，复用 registration/index/journal 与产品动作 |

## Risks / Trade-offs

- **历史布局兼容性下降：** fail-closed 会让未核验的旧项目暂时等待，但避免再次产生双轨解析和错误仓读取；通过明确迁移/核验操作提供恢复路径。
- **冷启动步骤依赖较多：** durable checkpoint 与只做安全重试增加状态展示复杂度，但能避免 active index 鸡生蛋和重复 provider turn。
- **repair 需要人工 mapping：** 无法保证无人值守恢复，但身份冲突自动吞并的损害不可逆；保留 digest/候选/影响范围使裁决可审计。
- **规则预检可能提前阻塞：** 真实材料门会暴露当前手工铺底依赖，短期看似减少自动通过率，却把运行时零提示失败转成可操作准备。
- **多 change 交接漂移：** 若 C2/C5 各自猜 policy 路径会重建双轨；因此实施时应将 resolver 输出的 authority/revision/digest 作为唯一跨 change 输入，并在联验前检查 A03/A04。

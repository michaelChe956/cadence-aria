# Design

## Context

见 `proposal.md`（Why）与 `specs/logical-codebase-knowledge-base/spec.md`（行为契约）。这里只记录**塑造方案形状**的现状与约束：

- 聚合初始化的 provider 选择器（`src/product/logical_codebase/provider_selection.rs`）是**唯一权威**：候选顺序 `Pi / Claude Code / Codex` 冻结、判定与 gateway 同源（L1 命令可用 + L2 能力已核验）、选择器零写入、结论四枚失败码。写权限分格按 action 分行存在能力矩阵里；一次「核验」同时确认 `CodingTargetWrite` / `PlanningReadOnly` / `ReviewReadOnly` 三行。
- provider turn 的两条既有通道：① 聚合初始化走 `AggregateBootstrap` 相位 + `BootstrapExecutorMarker`，其凭据与重核验**硬编码** `AggregateInitializationOperationStore`；② 普通会话走 Normal 相位 + `SessionPolicyAction::*`，聚合根 target 只做 canonicalize（不做 git 身份复验）。
- 聚合初始化的五步配方与四条 root recipe 命令是**冻结契约**（`root_recipe_command_index()` 被测试锁死），聚合根写入面审计窗口（前后快照 + 允许清单）已存在。
- Cadence-skills 侧：7 个 `knowledge-base-*` skill；bootstrap 需要人写的六领域输入（插件自带 9 份输入模板）；update 需要人写的五文件变更包（插件自带模板），且**禁止**扫描 updates 自动挑最新；两个 skill 都有敏感信息硬约束与 Manifest 4.0 不变量门禁。
- 前端：`monaco-editor` 已是依赖且有本地化 loader，但既有 `MonacoViewer` 写死只读；`MarkdownContent` 内嵌在聊天条目里；**无**可写编辑器、无文件树、无 YAML 解析依赖、无通用确认弹窗（沿用 `window.confirm`）。
- 平台侧没有任何 HTTP 文件写入端点；失败文案单一来源是 `failure_copy_table.inc.rs`，HTTP 状态映射在 `src/web/error.rs`（未登记的码默认 500）。

## Goals / Non-Goals

**Goals:**

- 知识库能力作为**独立 operation 模块**落地，复用选择器 / gateway / trust 门 / 单飞登记 / 取消语义，**不触碰**聚合初始化的冻结配方与凭据链。
- 平台侧的模板创建与门禁**只做机械可判定部分**，语义一致性一律交回 skill 判定，平台不成为绕过 skill 门禁的后门。
- 失败面全部有中文成品文案与正确 HTTP 状态；不留「通用兜底 + 500」的洞。
- 写侧安全：路径授权、白名单、乐观并发、运行期禁写四道约束齐备。

**Non-Goals:**

- 不做知识库与多仓规划/编码链路的自动注入（`knowledge-base-context` 接入是后续独立需求）。
- 不做知识库产物的 git 提交/发布策略、不做全文检索。
- 不改聚合初始化的任何既有语义（五步、四条命令、凭据链、审计窗口一律不动）。
- 不引入新的前端依赖；不引入 YAML 解析到前端。

## Decisions

### D1 知识库 provider turn 走 Normal 相位 + `CodingTargetWrite`（target = canonical 聚合根）

**决定**：知识库 operation 的 provider turn 用 `SessionPolicyAction::CodingTargetWrite`、`PolicyTarget::aggregate_root(canonical_root)`、cwd = 聚合根、可写根 = 聚合根；知识库的 provider 选择也按**同一分格**判定。

**理由**：

- 替代方案 (A) 走 `AggregateBootstrap` 相位：凭据派生与重核验写死在 `AggregateInitializationOperationStore` 上，知识库 operation 造不出凭据，必须泛化一条**被大量测试锁定**的冻结链；且知识库命令不在 `root_recipe_command_index()` 里，receipt 上下文也无从生成。改动面与回归风险都最大。
- 替代方案 (C) 走 Normal + `PlanningReadOnly`：provider 只读边界，知识库 skill 写不出产物，初始化失去意义。
- (B) 的形状成立：聚合根分支只 canonicalize、不要求 git 身份（git 身份复验只作用于 checkout 分支）；`CodingTargetWrite` 要求「可写根恰为 canonical target」，聚合根 target 天然满足。
- **选择期与起跑期落在同一行能力矩阵**是关键：若选择按 `PlanningReadOnly` 判、起跑按 `CodingTargetWrite` 判，就会出现「选择通过、起跑被拒」，且失败发生在 operation 已创建之后。统一分格后，该风险前移到创建 operation 之前的预检。
- 一次「核验」本来就确认三行，用户**不需要**多核验一次。

**代价与处置**：Normal 相位要求根规则与已发布政策存在（既有语义），故知识库操作的前置条件 = **聚合初始化已完成**；不满足时失败面直接指向「先完成聚合初始化」。这也是物理事实：Cadence-skills 由聚合初始化的 `machine_skills` 步骤安装。

### D2 变更包由用户撰写，平台只提供模板与门禁

**决定**：平台**不生成**变更包正文，provider **不代写**五份文档。平台提供：变更包模板骨架创建（标识由用户填写）、逐字段门禁、以用户指定路径调用 `/knowledge-base-update <目录>`。

**理由**：Cadence-skills 的 skill 契约把变更包定义为**用户显式指定的输入**，并明文「Git Diff、代码扫描只能验证变更包，不能替代变更包或补写用户声明」，且「无变更」声明必须给可审计依据（"Git 没看到""暂不清楚"不合规）。平台代填在 skill 眼里与用户代填无异，绕不过去；而一旦代填，门禁事实上被架空。

**代价**：owner 要多一步「写变更包」。用「模板骨架 + 页内就地编辑 + 逐字段未填标记」把这一步的成本压到最低。

### D3 平台门禁只覆盖机械可判定的子集

**决定**：平台卡 P1–P9 九项（路径形态、五份齐全、字段非空、枚举占位、原因、七行矩阵、敏感扫描、提交可解析、Manifest 不变量）；语义一致性（Diff 与声明是否相符、DDL 冲突、快照指纹、影响链、幂等、关系图等值）**不重复实现**，交回 skill。

**理由**：重复实现 skill 的语义门禁会造出两套口径，且平台永远追不上 skill 的演进；平台的价值在于「快速失败 + 可执行提示」，而不是替代 skill。**平台门禁通过 ≠ skill 通过**，两者都要过。

**边界**：敏感信息扫描用保守正则，宁可误报（用户自行处置，代价小）也不漏报（漏报代价大）；命中即拦，**不自动改写、不回显命中值**。

### D4 知识库 operation 独立模块 + per-LC 落盘 + per-LC 全局单飞

**决定**：新建 `src/product/logical_codebase/knowledge_base/`（operation / store / command / coordinator / templates / change_package / manifest / files）；operation 落 `lc_scope_root(...)/knowledge-base-operations/`；`operation_id` 派生含 `lc_id`；新增 `InitializationOperationKind::KnowledgeBase`；同一 LC 的 bootstrap 与 update 互斥。

**理由**：

- 不复用聚合初始化 coordinator：它的五步与四条命令是冻结契约，塞进知识库步骤会破坏既有断言与审计窗口。
- per-LC 落盘沿用仓内唯一约定（`lc_scope_root`），否则多 LC 的知识库 operation 挤在同一目录、无法按 LC 隔离。
- 单飞必须落在 **durable 检查**（照 `ensure_lc_initialization_started` 的 `other_recipe_running` 形状），不能只靠前端按钮禁用——operation 标识含前端随机 UUID，内存登记天然拦不住重复点击；且 bootstrap 与 update 写同一目录，并发会互相覆盖。

### D5 命令文本：前缀唯一来源 + 统一传目录路径

**决定**：命令 = `RepositoryInitializationStepKind::skill_command_prefix(provider)` + 固定命令名（`knowledge-base-bootstrap` / `knowledge-base-update`）；update 的参数统一传**变更包目录路径**（相对聚合根），不传变更标识。

**理由**：前缀映射是仓库既有契约（Claude `/`、Pi `/skill:`、Codex `$cadence-init:`），已被生产运行的四条 recipe 命令验证；另写一份映射就是第二套口径。上游 Cadence-skills 文档对 Codex 调用形态表述不一，**以仓库既有契约为准**——若真机不认，只改 `skill_command_prefix` 一处。skill 正文要求的是**目录路径**，传路径同时满足「显式指定唯一变更包目录」与「禁止扫描 updates 自动挑最新」。

### D6 知识库文件读写：产品面收敛 + 路径授权四道约束

**决定**：平台文件 API 的根 = 聚合根 + `cadence/knowledge-base`；写入白名单 = 全域 `*.md` + `user-input/**` 下 `*.sql`；乐观并发（客户端带读时摘要，不符则拒）；知识库 operation 运行期间拒写该目录。

**理由**：

- 平台 API 暴露 KB 子树是**产品面收敛**，**不是** provider 的写面限制——D1 下 provider 的写面是整个 canonical 聚合根（含成员仓子目录）。实现注释与文档必须写清这一点，避免实施者误以为「限制到 KB 子树就能约束 provider」。
- provider 侧靠**写入面审计**兜底：命令前后对聚合根做快照差分，允许面 = 知识库目录 + 项目规则目录 + 聚合根的代理规则入口文件（与既有 root recipe 允许清单同源口径），越界即 operation 失败。
- `.sql` 进白名单是因为六领域输入含 `database-ddl.sql`、变更包附件可含 DDL 差异。
- 乐观并发同时解决「两标签页同时编辑」与「用户与 provider turn 同时写」，比文件锁更可靠（跨进程不可靠）。
- 清单与派生关系文件（`manifest.yaml`、`relation-graph.yaml`）天然不可写（扩展名规则），符合「机器管理文件」语义。

### D7 状态投影在服务端解析，前端零 YAML

**决定**：新增 `GET .../knowledge-base/status`，服务端解析 Manifest 并投影结构化 DTO（存在性、版本、初始化状态、全局校验、阶段清单、skill 可用性）。

**理由**：前端无 YAML 依赖，让前端解析 Manifest 只能做脆弱的字符串匹配；且同一个投影同时供卡片（更新按钮禁用）与详情页使用。

### D8 失败面分流：请求期产物 vs operation 失败事实

**决定**：前置检查失败**不产生 operation**，失败明细以请求期响应返回（`code` / `message` / `details.missing[]` / `details.violations[]`），卡片维护「最近一次预检失败」本地态；已开始执行的失败走 operation 的失败事实。新稳定码 MUST 同时在 `failure_copy_table.inc.rs` 登记中文文案、在 `src/web/error.rs` 登记 4xx/422。

**理由**：没有 operation 就没有 durable 载体，失败面只能是请求期产物；而漏登记共享表的后果是「文案退化成通用兜底 + HTTP 500」，中文失败面根本出不来。

### D9 前端组件复用既有件，不新增依赖

**决定**：`MonacoEditor`（可写）复用 `monaco-local-setup.ts` 与 `useSystemMonacoTheme`；`MarkdownContent` 提取到 `components/shared/`，原处保留 re-export（保住既有 import 路径）；provider 摘要组件从 `AggregateInitializationCard.tsx` 提升为公共组件，两个来源装配同一 DTO；二次确认沿用 `window.confirm`；未保存提示复用 `hooks/useUnloadGuard.ts`。

**理由**：项目已有 Monaco（只读）与 `marked` 渲染，扩一个可写变体是零新依赖的最小改动；提取时保留 re-export 把改动面压到最小；provider 摘要若各写一份，卡片会自造一套候选表渲染，与聚合侧不同形。

## Risks / Trade-offs

- **[Normal 相位要求根规则存在 → 知识库按钮在未做聚合初始化时不可用]** → 失败面直接写明「先完成聚合初始化」；owner 已确认接受该前置。
- **[provider 写面 = 整个聚合根（含成员仓）]** → 写入面审计兜底（越界即失败）；平台 API 的 KB 子树收敛是产品面而非安全边界，文档与注释写清。
- **[平台门禁与 skill 门禁口径漂移]** → 平台只做机械子集并写明边界；skill 拒绝时失败面原样透传，不二次解释成平台口径。
- **[敏感信息扫描误报]** → 命中即拦但不自动改写、不回显值；假阳性由用户自行处置。
- **[Codex 的 `$cadence-init:` 形态在上游文档里表述不一]** → 沿用仓库既有契约；真机不认时只改 `skill_command_prefix` 一处映射。
- **[知识库 skill 面向 Java/Vue/React 存量项目，其它技术栈成员仓可能被 skill 判为范围不足]** → 平台不做技术栈拦截（不猜），失败面透传 skill 原文。
- **[单次 provider turn 耗时可达十几分钟]** → 卡片显示步骤进行中 + 可取消；二次确认写明耗时与额度消耗。
- **[知识库 operation 运行期间用户改文档会与 turn 抢写]** → 运行期禁写 + 乐观并发双保险。
- **[模板骨架复制可能覆盖用户内容]** → 只补缺失文件；变更包目录已存在时拒绝新建。

## Migration Plan

- 纯新增能力，无既有数据迁移；不改既有 spec 级行为。
- 既有 spec 与实现零改动（聚合初始化五步/四命令/凭据链/审计窗口不动）。
- 回滚策略：删除新增模块与路由即可；知识库产物是用户目录下的文件，平台不负责清理。

## Open Questions

- Codex 真机上 `$cadence-init:knowledge-base-bootstrap` 的实际可调用性，需在实施期用真机验证一次；不成立时按 D5 的兜底只改前缀映射一处。

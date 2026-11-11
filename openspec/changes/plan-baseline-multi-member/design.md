# Design: plan-baseline-multi-member

> 权威来源：`cadence/designs/2026-11-11_技术方案_WIG三缺陷与多仓事实供给修复_v1.0.md` §4.5、§5.4、§6.1、§7.3、§8、§10.2。分面迁移边界按本次裁定：本 change 只收敛 author 基线上下文与 AC 核对，coder fork/advance 只登记遗留缺口、不改实现。

## Context

动机与证据分级见 `proposal.md` 的 Why。以下是决定接口与迁移范围的已核实事实，不将构造风险写成真实 run 归因。

| 当前事实 | 代码锚点 | 对设计的约束 |
|---|---|---|
| loader 只取 `issue.repo_id`，返回单仓 `BTreeSet<String>` | `src/product/workspace_engine/plan_preflight.rs:132-178`，`plan_baseline_tree` | 多成员基线必须改模型与加载源，不能只扩 prompt 文本 |
| 校验器直接 `baseline_tree.contains(path)`，最近候选也使用该树 | `src/product/work_item_plan_compiler/validate.rs:70-118`，`validate_plan_candidate_ir` | 归属解析与候选建议必须一起切换 |
| 写范围教学已有成员前缀纪律 | `src/product/work_item_split_engine/prompts/sc_author_teaching.rs:238`，`work_item_plan_confirmed_member_order_teaching`；教学正文 `:281` 附近 | 事实面改为聚合根相对域，不反向改变写范围纪律 |
| 结构摘要直接消费当前单树 | `src/product/workspace_engine/plan_projection.rs:901`、`:936-941`，`build_single_candidate_structure_context` | 全量成员路径及归属变化必须进入摘要 |
| 多仓已有逐仓 HEAD 分支解析函数 | `src/product/issue_baseline.rs:128`，`resolve_member_head_branch` | 复用它，不重造分支默认链 |
| C1、advance 与 coding fork 目前均调用单仓分支解析 | `plan_preflight.rs::plan_baseline_tree`、`advance.rs::resolve_advance_base_branch`、`src/web/handlers/coding.rs::fork_base_branch_from_issue` | 本 change 迁移前两面涉及的基线加载；后两入口只登记，不能假称全链一致 |

`resolve_member_head_branch` 当前由逐仓实况使用；实况渲染允许单仓失败降级，不代表基线校验可照搬该降级。基线 loader 对必需成员必须 fail-closed。`resolve_effective_base_branch` 当前保留显式 `issue.base_branch` 与 main/master 默认链；它继续服务单仓及本 change 不改的 fork/advance 面。

## Goals / Non-Goals

**Goals**

- 计划授权成员是事实范围唯一权威，加载、校验、最近候选、D5 显示与结构摘要使用同一共享基线结构。
- author 上下文与 AC 核对在多仓下逐成员同源，事实路径域与既有写范围教学一致。
- 模糊归属或必需基线缺失显式失败；显示预算不足不影响完整校验事实。
- 明文登记多仓分支语义变化及 fork/advance 的已知遗留不一致。

**Non-Goals**

- 不新增成员发现器、检索工具、模型调用、重试、运行期开关或全仓内容快照。
- 不改写范围权限，不把跨成员读引用转成跨成员写授权。
- 不改 rework 的 D5 装配位置/配额算法，不接管 REQ-WSC-06，不改 criterion 命名域或指纹代际。
- 不改 coding fork/advance 实现；不提高 prompt 预算或任何 `large_file_guard` 登记上限。

## Decisions

### D1 · 共享 `PlanBaseline`，成员范围复用计划授权事实

**决定：** 用只读 `PlanBaseline { member_prefixes, trees }` 替代单仓平面集合。结构的语义契约如下；具体模块布局与签名由后续 Plan 展开，不在 prompt 侧另立模型。

| 组成 | 必须承载的事实 |
|---|---|
| 成员身份 | 物理成员仓 ID，用于与 WI 的 `target_repository_id` 对账 |
| `member_prefixes` | 该成员 checkout 相对聚合根的前缀，作为明确成员路径的归属权威 |
| `trees` | 按同一物理成员身份索引的仓内相对路径完整集合 |

成员范围为**计划已授权的有效成员**，复用 `plan_compile_member_targets`、`resolve_logical_work_item_plan_repository_targets`、`plan_member_checkout_prefixes` 产生的 manifest + active + selection 事实。磁盘上的其它 Git 目录不因此变成成员；`issue.repo_id` 也不再把范围压成一个仓。

前缀映射与树的成员身份必须对齐：缺 checkout、前缀重复或无法唯一确定归属，均为错误，不能静默丢成员。沿用目标成员在前、其余按 alias 的既有成员展示口径，避免与 rework 的成员清单/逐仓实况段顺序冲突。

**权衡：** 成本是每成员一次 Git 树读取、内存随路径数增长；收益是归属不再依赖路径猜测，并使四端消费有同一事实接口。模型调用零新增。

### D2 · 多仓逐成员 HEAD 分支；分面登记单仓语义差异

**决定：** 多仓 loader 对每个授权成员 checkout 调用 `resolve_member_head_branch(checkout)`，按其返回的本地分支读 Git 树。该函数执行逐仓 HEAD 分支解析，不猜 main/master；detached HEAD、空仓或 Git 故障返回错误。多仓基线加载不得调用 `resolve_effective_base_branch` 的单仓默认链替代它。

单仓继续使用锁定的 `issue.base_branch` 与既有默认链，保留仓内相对路径语义。Issue 创建与字段锁定规则不因本 change 改写。

**语义变更的精确登记：** 多仓下 `issue.base_branch` 对**author 基线上下文面、AC 基线核对面**不再生效，改用各仓自身 HEAD 分支；但它在**coder fork 面、advance 面**仍按单仓规则生效。后两面继续调用 `resolve_effective_base_branch`，本 change 不改 `coding.rs` 或 `advance.rs`。

因此，本 change 只使 author 与 AC 两面逐成员同源，**不能宣称三面同源已达成**。多仓 fork 基线可能与核对基线不一致，这是明确保留的已知遗留缺口，须后续独立 change 收敛。REQ-PIB-01 delta 登记分面语义，REQ-PIB-03 的三面同源场景显式区分单仓保证与多仓边界。

### D3 · F3：明确成员前缀优先，裸路径只绑定 WI 所属仓

**决定：** 多仓新产出用聚合根相对成员前缀路径，如 `frontend/src/App.vue`。解析遵循以下唯一口径：

1. 命中已授权成员 checkout 前缀时，路径归属于该明确成员，只查该成员仓内路径集合，不再做全仓同名搜索。
2. 无成员前缀的仓内相对路径仅作兼容输入，按当前 WI 的 `target_repository_id` 解析；其它成员存在同名文件不能让它通过。
3. 重复前缀、归属不唯一或不能确定绑定成员时显式失败，不降为 `None` 或任意选仓。
4. 明确跨成员引用是**读引用事实**；acceptance 命中不得消除跨成员 exclusive 越界，写权限仍由原有校验独立判定。
5. 单仓继续按仓内相对路径匹配，不强迫增加成员前缀。

`baseline_candidates` 与校验器共享解析结果：最近候选限制在已确定的成员及正确路径域；候选必须属于完整基线，不从 busi 给 frontend 拼出误导性建议，无候选则如实说明。

### D4 · 四端与全部调用方一批迁移，不接受仅修输入

| 端/调用方 | 已有落点 | 本 change 的职责 |
|---|---|---|
| 加载与共享类型 | `src/product/workspace_engine/plan_preflight.rs::plan_baseline_tree` / `plan_baseline_repository_path` | 加载全部授权成员，提供共享基线结构，严格传播错误 |
| 校验上下文 | `src/product/work_item_plan_compiler/types.rs::PlanCandidateValidationContext` | 携带共享完整结构，替换平面集合接口 |
| 校验与最近候选 | `src/product/work_item_plan_compiler/validate.rs::validate_plan_candidate_ir`、`baseline_candidates.rs` | 使用同一成员归属解析与候选域 |
| D5 渲染 | `src/product/work_item_split_engine/prompts/sc_baseline_tree.rs`、`src/web/workspace_ws_handler/run/single_candidate_baseline_tree.rs` | 标签来自真实成员归属，F3 路径来自完整基线，不从目录首段猜仓 |
| 结构摘要 | `src/product/workspace_engine/plan_projection.rs::build_single_candidate_structure_context` | 摘要覆盖全量成员路径与映射 |
| 调用链 | `src/web/workspace_ws_handler/run/single_candidate.rs` 三条校验调用、`src/product/workspace_engine/conversational_gate_parts/revision_turn.inc.rs`、`plan_projection.rs` | 全部迁移，不遗留旧平面集合消费或旁路 |

rework 的 D5 装载调用点与装配职责保持不变：本 change 替换**被装载的树结构及其消费者接口**，不重新安排何时加载、何处注入，不另造第二个 loader；D5 因数据契约切换获得多成员事实，不需要再改装载机制。

### D5 · 显示截断不缩小事实；预算仍归 rework

D5 只按真实成员归属渲染路径，不能把路径首段 `src` 当作仓名。渲染出的每条路径必须能回溯到完整 `PlanBaseline` 的成员与树。

清单继续占上下文面，保留 rework 的「先算固定开销、剩余额度给清单、上限上下文预算的四分之一、额度为零整段缺席」口径。显示可按既有机制截断，**validator 与摘要始终消费完整结构**，不得把显示子集回写为校验事实。不得以「动态段」名义逃避预算。

下表仅为主方案的**基线对照值 / 实施后须复测**，不是已达成结论：

| 项 | 对照值与实施约束 |
|---|---|
| F3 三成员基线段 | 2,407 B：busi 1,258 / gateway 787 / frontend 362 |
| 相对原基线段 | 1,308 → 2,407 B，增加 1,099 B |
| 上下文面 | 25,518 → 26,617 B，余 5,383 B；实施后实测必须 ≤32,000 B，成员清单等其它段仍需合计计量 |
| 固定规范面 | 23,500 B 预算零新增，既有固定 fixture 逐字节不变 |
| 大文件守卫 | 默认 1,200 行；既有登记 `workspace_engine/prompts.rs`=1,681、`review/routing.rs`=1,391、`tests/prompt_contract.rs`=1,667、`run/single_candidate.rs`=1,666 均不得抬高 |

### D6 · 结构摘要覆盖成员路径与归属，不承诺内容快照

`build_single_candidate_structure_context` 的基线摘要必须由完整成员结构派生。仅非默认成员增加/删除基线路径，或路径字符串集合相同但成员/前缀归属映射变化，都必须改变结构上下文 digest。

当前摘要是**文件路径集合摘要，不是文件内容摘要**。不把同路径内容提交导致的 HEAD 变化写成已被保护的事实，不增加全仓内容快照或新的模型调用。

### D7 · 备选与否决

以下数字均为主方案的格式测量**基线对照值，实施后须复测**。

| 方案 | 代价/缺口 | 结论 |
|---|---|---|
| **F3 聚合根相对 + 成员前缀** | 2,407 B，基线样本零碰撞，归属可见且与写范围同域 | **选定** |
| F1 成员内相对分组 | 2,112 B，但有 6 处跨仓同名，且路径域与写范围相反 | 否决 |
| F2 同域无标签 | 2,069 B，省 338 B，但丢失归属可视化 | 本 change 不选 |
| 仅按需检索/逐仓概览 | 已有渲染器，但不含文件清单，不能替代基线树 | 否决为替代方案；概览接线仍归 rework |
| 新增检索工具 | 已有 Read/Glob/Grep/Bash/MCP-codegraph，cwd 为聚合根；77 个 author 节点中 59 个零工具调用，属基线对照事实 | 否决，不能依赖模型自愿检索补齐事实 |

## Failure Handling

- 任一必需成员 checkout/分支/树不可解析，或成员归属映射有冲突：loader 返回显式 `Err`，不产出不完整成功结构，不将错误吞成 `None`。
- 真正无仓 issue 保留既有 `Ok(None)` 语义；它与「授权成员加载失败」不同，不可混用。
- 作者轮前的 D5 装载失败仍按 rework 既有口径记录诊断并使显示段缺席，不新增作者启动阻断点；Evaluate 等原有核对面继续传播真实加载错误、fail-closed，不因 prompt 显示降级而跳过校验。
- 其它分支、未提交文件、兄弟 worktree 独有文件及新建未入基线脚本不进入树；不得退化成读当前磁盘兜底。
- 路径缺失仍产生 `acceptance_path_not_in_baseline` Error 与既有三种修复建议；只修成员判定域，不改变受限路径提取或写权限规则。

## 共享面与排序

本 change 为**全局串行序 7 之后，最后落地**，前置是既有 change 的共享接口稳定，尤其 rework 的 D5 装配已就位。

| 共享写面 | 排序理由 |
|---|---|
| `compiler/validate.rs`、`compiler/types.rs` | 等待 mechanicalize 的诊断接口与 rework 的 D5 事后候选落地，再切共享基线消费 |
| `plan_projection.rs` | 等待既有结构上下文/归属与指纹接口稳定，不重写它们的契约 |
| `sc_baseline_tree.rs`、`run/single_candidate_baseline_tree.rs` | 等待 rework 的 D5 显示与预算口径稳定，再替换树结构 |
| `run/single_candidate.rs`、`conversational_gate_parts/revision_turn.inc.rs` | 已被序 1/3/4/5/6 多方共写；本 change 必须排在全部既有 change 之后，不得并发修改 |

rework 的 D5 **装载点不改**与本 change 排最后不冲突：装配在先，树结构替换在后。REQ-WSC-06 仍由 rework 独占；本 change 仅 MODIFIED REQ-PIB-01、REQ-PIB-03、REQ-WSC-02。写 delta 前已检索全部 active change 的 `specs/**/spec.md`，三者均未出现第二处 MODIFIED。

## Migration Plan

- 按 B1 → B2 串行承接：B1 建立共享基线与严格 loader，B2 在同一完整切换中迁移校验/最近候选、D5、摘要及全部调用方。工作包可分别跟踪，但**不接受只修输入或混用新旧事实结构上线**。
- 单仓基线选择、路径语义与无仓缺席语义保留；Issue 字段无需批量迁移，既有字段不清空。多仓字段的分面生效变化由 delta 明示。
- 不保留旧平面集合的生产旁路、兼容别名或第二套成员发现机制；所有消费者切到共享契约。
- 前置纯拆分负责消除既有大文件超限，本 change 不提高登记；精确布局、命令、测试步骤及提交建议留给契约获批后的 `writing-plans`。

## Acceptance Mapping

| 组 | 可执行边界 | requirement 与已有验证落点 |
|---|---|---|
| 基线加载 | 真实三仓 Git 夹具，各仓提交独有文件，`issue.repo_id=busi` 仍覆盖全部授权成员；排除脏文件与兄弟分支文件；任一必需成员失败得到 `Err` | REQ-PIB-01/03；`plan_preflight.rs` 现有基线测试或按既有模式拆出的独立测试模块 |
| 校验器 | frontend 明确路径通过；busi/frontend 同名 `package.json` 归属确定；仅 frontend 存在的裸路径对 busi WI 仍 Error；AC 命中不消除跨仓 exclusive 越界 | REQ-WSC-02；`src/product/work_item_plan_compiler/tests/preflight_baseline_paths.rs`（已核实存在） |
| D5 | 标签来自真实归属，显示路径属于完整结构，截断不影响全量校验；上下文面 ≤32,000 B、固定面不变 | REQ-PIB-03 / REQ-WSC-02；`src/product/work_item_split_engine/tests/prompt_contract_baseline_tree.rs`（已核实存在） |
| 摘要 | 只改非默认成员路径或同路径归属映射，digest 必须变化；不声称检测内容变化 | REQ-WSC-02；`plan_projection.rs` 的结构上下文摘要验证 |

全部为确定性机制验收，不依赖新增模型调用；真实 run 收益与其它修复联合复测，不把构造通过写成 confirmed 率提升。

## Risks / Trade-offs

| 风险/残余 | 处置与边界 |
|---|---|
| **多仓 coder fork/advance 与 author/核对仍可能不同源** | 本 change 只收敛 author+核对两面；后两面仍用 `issue.base_branch` 单仓语义，属已知遗留缺口，需后续独立 change，禁止声称三面已同源 |
| 三仓中任一必需基线失败会拒绝不完整事实 | 保持 fail-closed；不可用其它成员成功或 prompt 降级冒充全量可用 |
| 授权前缀重叠或归属不唯一 | 显式失败，不任选成员、不做全仓同名兜底 |
| 每仓 Git IO 与全量路径内存增加 | 仅每成员一次树读取、内存随路径数增长；不增加模型调用，不用显示截断裁 validator 树 |
| 成员清单与基线树共同占上下文面 | 按面合计复测，32,000 B 不上调；D5 配额机制不改 |
| 同路径内容改变不改变路径集合摘要 | 如实保留边界，本 change 不扩成内容快照 |
| 66 个 IR 实测误判率为零，构造修复不能证明真实 run 收益 | proposal 分列证据，所有数字标为基线对照、实施后须复测 |

## Open Questions

无须本 change 再裁定的架构选项。coder fork/advance 的后续收敛是明确排除的遗留缺口，不是本 change 的未完成实现工作包。

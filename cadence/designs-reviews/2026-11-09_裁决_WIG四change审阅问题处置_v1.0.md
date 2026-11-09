# 裁决 · WIG 四个 OpenSpec change 审阅问题处置 · v1.0

> 裁决者：controller（owner 授权「修，该修的都修复好」）｜日期：2026-11-09
> 权威上游：`cadence/designs-reviews/2026-11-08_决策_WIG六项owner裁决_v1.0.md`（下称「**裁决**」）
> 审阅输入：`cadence/designs-reviews/2026-11-09_评审_WIG四个OpenSpecChange_*.md`（4 份，共 61 项）
> 作用：**本文件是四个 change 与主方案 v1.3 的修订口径**。与「裁决」冲突时以「裁决」为准；本文件只补「裁决」未逐字展开处。

## 1. 总则

1. **只改规划产物，不改实现**。本次修订范围＝`openspec/changes/{bound-sc-generation-teaching,isolate-plan-criterion-namespace,close-contract-findings-before-pass,explicit-sc-author-effort}/` 下的 `proposal.md` / `design.md` / `tasks.md` / `specs/**`，以及主方案 `cadence/designs/2026-11-08_技术方案_WIG生成加速与收敛性加固_v1.0.md`。**禁止**改 `openspec/specs/**`、禁止改 `src/**`、禁止改其它 change 目录、禁止 `git add` 越界。
2. **MODIFIED 保全纪律不变**：delta 标题与主规格逐字相同；主规格该 requirement 的**原有场景标题与正文逐字保留**，只允许追加场景与追加/改写描述段。原文不得删改、不得改写既有场景。
3. **不得为了「看起来干净」放宽契约**：任何「删掉义务」「改写 MUST 强度」「承认 pass」「推迟二期」「弱化归档门禁」的改法一律禁止。
4. **不得夹带**：本文件未点名的行为变更不得引入；新增需求只能服务于本文件点名的缺口。
5. **禁止预写实现细节与哈希数值**：沿用「裁决」§5.6 的纪律（不预写哈希、不声称实测）。
6. **验收口径见 §8**。每个工作包完成后必须自跑 §8 的命令并把真实输出写进报告。

## 2. 三条全局口径（四个 change 共同遵守）

### 2.1 归档后主规格 500 字符门禁 —— 压缩，不接受变红

`openspec validate --specs --strict` 对**主规格**每个 requirement 的**描述段**（不含标题、不含场景）有 500 字符上限；change 级对 `MODIFIED` 块豁免，**归档后不豁免**。实测：

| change | requirement | 主规格现值 | delta 现值 | 必须砍掉 |
|---|---|---|---|---|
| bound | `workspace-structural-governance` / REQ-SG-05 | 370 | 780 | **≥280** |
| close | `work-item-plan-single-candidate` / REQ-WSC-17 | 254 | 883 | **≥383** |
| effort | `workspace-session-resume-recovery` / 明确 fresh 只换 native 会话 | 218 | 649 | **≥149** |
| isolate | TOP-04 / TOP-05 | 177 / 128 | 333 / 281 | 无需压缩 ✅ |

**处置**：把描述段里属于**举例/边界/细则**的文字下沉到**新增场景**（场景无长度上限），描述段只保留规范性主干。**压缩后描述段必须 ≤500 字符**（含标点）。原场景逐字保留不受影响。**禁止**用「主规格校验改成非 strict」「登记豁免」等方式绕过。

### 2.2 共享面全局串行顺序（唯一权威，各 change 的 design 必须互相对齐）

| 序 | 包 | 承担的共享面 | 前置 |
|---|---|---|---|
| 0 | **决策六：拆分大文件**（纯移动） | `run/single_candidate.rs`(1660/1666)、`workspace_engine/prompts.rs`(1667/1681)、`contract_autorepair.rs` | 无。**必须最先**——handler 仅剩 6 行余量，任何实质新增前先拆 |
| 1 | `mechanicalize-plan-compile-repair` | MODIFIED **REQ-WSC-09**（typed IR + structure 资格清单） | 决策六 |
| 2 | `fix-author-rewrite-quota-pool` | MODIFIED **REQ-WSC-21**（门段 `final_compile_*` 触发域） | 无 |
| 3 | `bound-sc-generation-teaching` | WSC-28 族 + MODIFIED **REQ-SG-05** + MODIFIED **REQ-WSC-25** | 1（资格清单）、2（触发域口径）、0 |
| 4 | `explicit-sc-author-effort` | WSC-29 + MODIFIED「明确 fresh 只换 native 会话」 | 3（窗口冻结输入摘要字段）、0 |
| 5 | `isolate-plan-criterion-namespace` | MODIFIED **REQ-TOP-04/05** + 新增 WSC-30/31 | 0 |
| 6 | `close-contract-findings-before-pass` | MODIFIED **REQ-WSC-17** + ADDED **REQ-TOP-07** | 5（消费 isolate 的版本化 scope）、1 |

**文件级归属（同一文件被多包写时按此序串行，后序不得覆盖前序语义）**：

| 文件 | 写入包（按序） |
|---|---|
| `run/single_candidate.rs`（handler） | 0 拆分 → 1 mechanicalize(typed 判据) → 3 bound(claim/教学) → 4 effort(env 装配) → 5 isolate(`forged_done_when` 删除与教学面) |
| `workspace_engine/prompts.rs` | 0 拆分 → 4 effort（SC author 出口）→ 5 isolate（spec/plan 域教学） |
| `lifecycle_store/workspace.rs` | 3 bound（窗口/ledger 段）→ 4 effort（创建/接管逐字段）→ 5 isolate（`run_history` 创建点与代际） |
| `work_item_plan_policy/classify.rs` | 5 isolate（版本传递）→ 6 close（闭合谓词） |
| `work_item_plan_policy/types.rs` | 3 bound（ledger entry）→ 5 isolate（代际字段） |
| `models/workspace.rs` | 3 bound（窗口字段）→ 4 effort（SC 冻结字段） |
| `review/routing*.rs`、`review/closure_policy.rs` | 1 mechanicalize → 6 close |

**对称登记义务**：每个 change 的 design「共享面与排序」小节**必须**列出与它共写同一文件的**其它包的包名**，并写明相对次序。单向登记（只写自己、不写对方）视为不合格。

### 2.3 命名域与跨 change 归属的唯一性

- `work_item_plan_policy/*` **不由 isolate 独占**。isolate 只独占「指纹代际 + 命名策略元数据」相关面；`classify.rs` 由 **isolate → close** 串行共写。
- `forged_done_when` 的删除归属 **isolate**（裁决 §5.4 明列 `handler single_candidate.rs:404-437` 删除 `forged_done_when`），**bound 与 close 均不得认领**。
- MODIFIED **REQ-WSC-06**（方案 E 投影裁剪，X2）**当前无任何 active change 承载**。处置见 §7.2：**不得由 isolate 夹带**。

## 3. bound-sc-generation-teaching 的处置（I-01 ~ I-14）

### 3.1 I-01【阻断】窗口语义与「裁决」相反 —— 必改

裁决 §1.5 原文：「**窗口保存历史，关闭不删除、不通过新 source 重新计数**」；§1.5「接管只能引用原窗口的关闭/消耗事实，MUST NOT 凭新的 session ID 重新铸造 C 机会，原 session ledger 保留权威」。delta 现有「MUST NOT 保留已关闭的窗口」把「不重新铸造」误写成「不保留」，与 design D7、tasks 6.2 直接矛盾。

**改法**：`specs/workspace-structural-governance/spec.md` 的 SG-05 描述段，把该句改为：
> 已关闭窗口 MUST 保留其关闭与消耗事实供审计与接管引用；MUST NOT 被删除、清空、复用或以新 source 重新计数，接管 MUST NOT 凭新 session ID 重新铸造机会。

并按 §2.1 把字段清单/行为细节下沉场景（净砍 ≥280 字符）。**原有 4 个场景逐字保留**，新增场景承载「关闭后仍可被接管引用」与「关闭窗口不重计数」。

### 3.2 I-02【阻断】28E 与 REQ-WSC-21 字面冲突 —— 必改（加作用域限定，不得改 WSC-21）

裁决 §1.1(3) 原文：「**WSC-21 的触发域是门段 `final_compile_*`** …… 不能通过 C 的 handler 复用同一个 provider 函数就推断其属于 A/B」。裁决 §1.5 亦写「结构失败耗尽禁止落到另一自动通道」。

**二者不是矛盾，是段落不同**。处置：在 28E 描述段加作用域限定句（原文保留，仅追加）：
> 本条仅适用于 **generate 段**原生成链的第二次交付；**门段** `final_compile_*` 候选编译类失败不属于本条，其自动重写按 REQ-WSC-21 既有合同处理，REQ-WSC-21 的触发域不因本条收窄或扩张。

并新增一条场景：「门段候选编译失败仍按 REQ-WSC-21 驱动重写」。**禁止**修改主规格 REQ-WSC-21、**禁止**修改 `fix-author-rewrite-quota-pool/`（裁决 §1.4 明文「不修改」）。

### 3.3 I-03 —— 并入 §2.1（SG-05 压缩 ≥280 字符）。

### 3.4 I-04【重要】WSC-09 资格清单无 typed 扩充 —— 登记为硬门禁

裁决 §1.5 明定四类 typed IR 诊断（`unknown_done_when_ref`、`unknown_reviewer_check_ref`、`acceptance_criterion_without_reviewer_check`、`unknown_requirement_ref`）与 producer 落点 `work_item_contract/validation.rs:245-312` + 新 `work_item_plan_compiler/ir_fault.rs`。

**处置**：bound 的 design 必须写明**归档硬门禁**——「mechanicalize 的 MODIFIED REQ-WSC-09 完成 typed IR + structure 资格扩充后，本 change 方可归档；在此之前 28 的『资格以 WSC-09 为唯一清单』不得实施」。tasks 增一条对应验收项。**不在 bound 内重复 MODIFIED WSC-09**（裁决 §1.4 明令）。

### 3.5 I-05【重要】上限写死 max=2 —— 改为 {1,2}

裁决 §1.3 原文：「只有已批准且耐久绑定的 RepairOnce 才可使用原调用之外的一个物理机会，原调用在内最多两次；**无批准时最多一次**」。design D1/D6 现有 `max=2` 写死。

**改法**：D1 窗口字段清单补齐裁决 §1.5 明列项——「RepairOnce 批准状态」「批准规则版本」「max ∈ {1,2}（由批准状态决定）」「冻结输入摘要」；D6 与 tasks 的上限表述同步改为 `{1,2}`。

### 3.6 I-06【重要】冻结执行策略摘要字段悬空 —— 补通用字段

裁决 §3.4 第 4 行把「窗口冻结输入摘要」列为硬依赖，effort 侧 design 声明「由 bound 承接」。

**改法**：bound 的 28B 字段清单 + D1 + tasks 1.1 增通用字段「**冻结执行策略摘要**」（只定义字段与「由 effort 填值」，**不在 bound 定义 effort 语义**）。bound 的 Non-Goals 中「effort 不在本 change 范围」一句保留，但补「effort 语义不在本 change；冻结执行策略摘要字段由本 change 提供」。

### 3.7 I-07【重要】共享面登记不对称 —— 按 §2.2 重写

bound 的 design「共享面与排序」必须按 §2.2 表列出：与 **effort** 共享 `models/mod.rs`、`models/workspace.rs`、`lifecycle_store/workspace.rs`、`workspace_engine/types.rs`、`run/single_candidate.rs`、`workspace_engine/prompts.rs`（次序：bound → effort）；与 **isolate** 共享 `work_item_plan_policy/types.rs`（bound → isolate）；与 **mechanicalize / quota-pool** 的 WSC-09 / WSC-21 依赖；并声明「**决策六拆分先落**」为前置。同时澄清 effort #6「compile 教学块迁出 prompts.rs」的归属＝**决策六**（`prompts/sc_compile_teaching.rs`），**不是 bound**——bound 的 tasks 不承接该项。

### 3.8 I-08【重要】D2 漏耦合点 —— 补齐

裁决 §1.5 明列必须迁移的读写点：`workspace_engine/single_candidate.rs:138-167,244-256`（含 `:244-256` 取「ledger 中最后一个前缀键」当 reservation）、`review/routing_scope.rs:20-46`（以 `ledger.len()` 预占 A 路径键），以及 durable→runtime 同步与 policy CAS 不得覆盖已更新 ledger。

**改法**：D2 点名补全上述三处；tasks 1.2 与 8.3 增验收项「**记 C 后 A 路径键不变**」。

### 3.9 I-09【重要】attempt 键与占位值 —— 按裁决 §1.5 定案

裁决 §1.5 原文：「新 C key 应从原 generation 身份和显式 attempt 序号构造，例如 `sc_generation:{session}:{generation}:attempt:{1|2}`；**provider 技术运行使用独立 launch 身份关联该 attempt**」；「**先占位 `started=false`**，同 key 的真正 observation 原位补全，不另插一条『同一次』记录」。

**定案**（写入 D2/D4）：
- attempt1 与 attempt2 **同用**新键式 `sc_generation:{session}:{generation}:attempt:{1|2}`；旧键 `single_candidate_author:{id}:{n}` 是 provider launch 身份，**保留为 ledger 自身的键**，不与 attempt 键混用。
- provider_start_ledger 条目**以其 launch 身份为键**，并**新增 attempt 键关联字段**；「同一 attempt key 写入既有 ledger」据此改写为「该 attempt 的实际启动 MUST 写入既有 provider_start_ledger（以 launch 身份为键）并携带 attempt 键关联；MUST NOT 为同一次启动另插第二条记录」。
- 占位值 MUST 为 `started=false`（现实现为 `started:true, provider:None`，须在 tasks 列明改动点）。

### 3.10 I-10【次要】越权触发源 —— 标注归属

「修复范围越权」的新码 `repair_scope_violation` 归**裁决 §7 的 C 包**（机械修复实施侧）。bound 的 28E 与 tasks 4.3 注明该判据由 C 包交付，并补一条场景占位说明其在 C 包落地前不可判定。

### 3.11 I-11【次要】缺场景承载的规则 —— 补场景

裁决 §1.5 明列但只写在 design/tasks 的四条，各补一个短场景：① 接管不铸造新机会；② 窗口只在初轮新授权建立、A/B 接力不重建；③ 不具备合法观察/准入的 provider 路径不得获 C 机会（裁决 §1.5「未具备合法观察/准入的provider路径不得获C额外机会，按SG05显式回显」）；④ 历史样本不得补造 grant/actual-start。

### 3.12 I-12【次要】WSC-25 attempt 关联 —— 放宽为条件式

现 `attempt_seq/attempt_key` 是 store 分配的会话内流水号，非 provider 启动。**改法**：「每条记录 SHALL 关联本次触发的 provider attempt（attempt 序号与 attempt key）；**若本次未触发 provider 启动则 SHALL 显式记录「无 provider 启动」，MUST NOT 伪造关联**」。

### 3.13 I-13【次要】行号偏差 —— 更正

`models/workspace.rs:843`→844；`workspace_single_candidate.rs:129`→127（占位写入实为 197-203）；`gateway_start.rs:585`→583；「七个 caller」改为引用 `gateway_start.rs:594` 注释而非断言（实为 11 处非测试调用点）；handler `:1307` 亦置位 `compile_ir_rerun_used`，design 须一并列 1227。

### 3.14 I-14【次要】流程项混入验收 —— 归位

tasks 7.2 / 7.3 / 7.4 / 8.5 为流程/协调项，移入「Workflow follow-up / 授权后」小节，不再映射到 REQ-SG-05 / WSC-28 / 28A 的可验收产出。design 映射表 §1.3 末段(fail-closed) 订正为 ¶4 末句；补 §1.6 场景映射。

## 4. explicit-sc-author-effort 的处置（I-01 ~ I-19）

### 4.1 I-01【重要】与主规格 REQ-ENV-HOST-01 冲突 —— 走 OpenSpec 显式裁决，不得静默违反

主规格 `session-policy-envelope/spec.md:506`：「平台不构造、不覆盖子进程环境（**调用方显式 env map 仅作覆盖层**）」；`:514`「平台不注入」。裁决 §3.1(2)/§3.2 已判定：「由平台 SC 入口冻结」「caller 注入有可达路径，不用改全局 adapter 默认」。

**处置**：本 change **新增**一份 delta `specs/session-policy-envelope/spec.md`，以 `## MODIFIED Requirements` 修改 REQ-ENV-HOST-01，**追加**一句显式例外（原文与既有场景逐字保留）：

> 受信执行策略变量（provider 官方声明的单一档位变量，如 `CLAUDE_CODE_EFFORT_LEVEL`）MAY 由平台在**调用方显式 env map 覆盖层**内设定为该 provider 的受控值；该例外 MUST NOT 构造或覆盖其余子进程环境、MUST NOT 涉及凭据、隔离或网络命名空间、且 MUST NOT 依模型标签或宿主设置重算。原「平台不构造、不覆盖子进程环境」的义务对本变量以外的环境保持不变。

并新增一条场景（承载「平台设定受信执行策略变量不得影响其余环境继承」）。design 必须新增一节说明这是对主规格 REQ-ENV-HOST-01 的**显式修订**，并引用裁决 §3.1(2)/§3.2 的可达性依据。**禁止**直接改 `openspec/specs/**`；**禁止**把该例外做成「仅本 change 的实现细节」而不进规格。

### 4.2 I-02 —— 并入 §2.1（压缩 ≥149 字符）。

### 4.3 I-03【重要】接管继承父冻结事实缺场景 —— 补场景 + 明确复制口径

裁决 §3.4：「创建/恢复/接管逐字段贯穿；**接管继承原策略，不以新 session ID 换档**；server 创建事实决定新默认，读旧 record 不写入」。

**改法**：design 明写「接管 MUST 原样复制父会话的冻结字段，父为 None 时保持 None，MUST NOT 走「创建时写新默认」路径」；resume delta 增场景「自动→交互接管沿用父冻结档位（含缺省）」。**代码依据**：接管走 `create_workspace_session_with_id` 后逐字段复制（`lifecycle_store/workspace.rs:455-470` 创建、`:485-499` 复制），故 design 须点名这两处并写明「接管路径 MUST NOT 触发新默认写入」。

### 4.4 I-04【重要】冻结点与 provider 可变 —— 按裁决 §3.4 定案

裁决 §3.4 明列落点 `models/workspace.rs:843-850` 邻近 SC 字段，创建/恢复/接管逐字段贯穿；「**server 创建事实决定新默认**」。故**创建时写入**是裁决口径，不改。

**补强**（处理 `author_provider` 创建后仍可被 `set_provider` 改写的问题）：加一致性条款——「若创建时写入的 provider 与真实 SC 首次生成准入时的 author_provider 不一致，MUST NOT 静默沿用该档位：MUST 按『无冻结事实』处理（披露未知、不注入平台覆盖），MUST NOT 以创建时值覆盖准入时值」。delta 增对应场景。

### 4.5 I-05【重要】「原运行」与「真实新生成」边界 —— 定义

**定义**（写入 design 与 delta）：
- **原运行**＝同一 session record 承载的同一冻结事实；其上的一切运行（重臂、resume、native fresh、compile/IR 教学 attempt）**均属同一原运行**，MUST 继续使用同一冻结策略。
- **真实新生成**＝server 创建事实为**新建 session record** 的首次生成；只有它适用当前部署默认（`high`）。
- 读旧 record（含旧 record 上的重臂）MUST NOT 写入新默认，**保持 None（继承宿主）并披露未知**。

依据：裁决 §3.1(3)「旧已开始运行保原配置；不存在耐久 effort 证据的旧运行不得在 resume 时被补写成『原本 high』」+ §3.4「server 创建事实决定新默认，**读旧 record 不写入**」+ §3.5「存量继承模式的实际 effort 仍可能受宿主变化影响，不能宣称旧运行的 effective 已被追溯冻结」。delta 增场景「同一 session 重臂仍属原运行」「新建 session 适用新默认」。

### 4.6 I-06【重要】fail-closed 无 requirement 承载 —— 补场景或降级映射

裁决 §3.5 明文：「未知 policy_version 零启动 + typed 错误 fail-closed」为「未识别 policy_version」的安全要求，但裁决 §3.4 未逐字给出该条。

**处置**：在 REQ-WSC-29 描述段或场景中补一条场景：「冻结策略引用的 policy_version 不可识别时 MUST 零 provider 启动、保留原记录、MUST NOT 回落当前默认」，并在 design 标注该条为**依据裁决 §3.5 推导的安全补充**（不冒充裁决原文）；若因 §2.1 字符预算无法并入描述段，则该场景 MUST 挂在 REQ-WSC-29（其 472 字符，尚有余量）而非 resume MODIFIED。tasks 3.3 的 requirement 映射改为指向该场景。

### 4.7 I-07【重要】冻结输入摘要依赖只在单侧 —— 按 §3.6 在 bound 侧补字段，本侧改写为「已由 bound 承接」

**改法**：effort 的 design 132/135-139 与 tasks 4.6 保留声明，但补一句「该字段由 `bound-sc-generation-teaching` 的 REQ-WSC-28B / D1 提供（次序 bound → effort），本 change 只负责填值」，并把 tasks 的「由集成 owner 接上、本 change 不改其文件」改为指向具体 change 名与条目。

### 4.8 I-08【重要】共享面声明不互认 —— 按 §2.2 重写

effort 的 design 共享面表必须列出与 **bound**（8 项，bound → effort）、与 **isolate**（`lifecycle_store/workspace.rs` 创建/接管段，effort → isolate）、与 **close**（无直接共享写面，显式写「无」）的关系；并更正 proposal.md:62「与 mechanicalize 无共享写面」——mechanicalize 的代码面含 `run/single_candidate.rs`，两者经**决策六拆分后的模块**才分离，改为「经决策六拆分后不共享写面」。

### 4.9 I-09【重要】attempt 同名异义 —— 改写

「MUST NOT 增删 attempt 事实」与 fresh 的真实启动追加候选尝试流水（bound 的 MODIFIED WSC-25）冲突。

**改法**：改为「fresh MUST NOT 新增**授权来源**、MUST NOT 重置预算/预留/授权事实；其真实启动照常按 REQ-WSC-25 记流水。本条的 attempt 指 gate/授权层 attempt，不指候选尝试流水与 provider_start_ledger 条目。」

### 4.10 I-10 ~ I-15【次要】

- **I-10** 从作用域矩阵移除「基线教学」（`baseline_teaching.rs:337` 起全在 `#[cfg(test)]`），改为「builder 内基线注入不受影响」。
- **I-11** 生产创建恒 SC 的证据改引 `web/handlers/lifecycle/plan_preparation.rs:134`（`flow_kind = SingleCandidate`）；`:210,235` 补入。
- **I-12** 理由改写：「**共用 builder 无法按 provider 区分**」（分流实际在 `ProviderRunKind`，`provider_run.rs:286-293`）；结论不变。
- **I-13** 「本期不改」清单补 `prompts/revision.rs:96`（Story/Design 修订出口），并写明 `provider_drive/artifact_retry.rs:248-263` 的 retry 继承原出口。
- **I-14** REQ-WSC-29 正文/场景补「含 SC 人工修订出口」（`sc_manual_revision.rs:356-366`）。
- **I-15** fresh 场景写明「**仅 effort/执行策略表达被忽略；provider/权限变更仍按既有场景拒绝**」，删「等价执行策略」措辞。

### 4.11 I-16【次要】5.4/5.5 需另授权 —— 移入 follow-up

tasks 5.4/5.5 移入「Workflow follow-up / 需 owner 另授权后」小节并注明**不阻归档**；4.x/6.x 合并或降为说明项。

### 4.12 I-17【次要】缺回退段 —— 补

按裁决 §3.5「回退：新生成策略可经受控版本改回原档位/旧继承，新旧运行各按原冻结策略结束；不热切在途 provider、不清预算。」补 design 回退段，并补「新增字段对 `WorkspaceSessionRecord`（无 `deny_unknown_fields`）旧二进制回滚影响」与「旧二进制**不得直接回滚**」（裁决 §5.6 同口径）。

### 4.13 I-18【次要】归属混乱句 —— 删

删去 design「边界 1 的推论」一句，或移入跨 change 一节并写明归属包名。

### 4.14 I-19【重要·横向】主方案 v1.3 残留 —— 见 §7.1

## 5. isolate-plan-criterion-namespace 的处置（R-01 ~ R-15）

### 5.1 R-01【重要】`lifecycle_store/workspace.rs` 登记缺失 —— 补

按 §2.2：本 change 改创建点 `run_history: Default::default()`（`lifecycle_store/workspace.rs:325`），接管 `:485` 的 `run_history.clone()` 已覆盖、**不改**。design 的共享面表补该文件与行号，并登记与 effort / bound 的次序（bound → effort → isolate）。

### 5.2 R-02【重要】`forged_done_when` 无归属 —— 归 isolate

裁决 §5.4 明列：`handler single_candidate.rs:404-437`「保留真实 spec 域 `forged` 检查，**删除 `forged_done_when` 及混合错误文案**」；`:458-487` 与 `workspace_engine/prompts.rs:244-249`「两处教学均分清 spec 与 plan 域」；`work_item_split_engine/prompts.rs:185,306`「ID 纪律加入 WIC」。

**处置**：
- 本 change 的 design 共享面表**必须**登记 `run/single_candidate.rs`（含上述行段）与 `workspace_engine/prompts.rs`、并声明次序（决策六 → mechanicalize → bound → effort → isolate）。
- tasks 增工作包：**删除 `forged_done_when` 与混合错误文案**、两处教学分清 spec/plan 域、`prompts.rs:185,306` 加入 WIC 纪律、`:306` 的 spec Traceability AC 示例**不替换**。
- 验收项：LegacyPreserved 的 plan 域 `AC-001` 若不在 spec 注册集，**不再被判伪造**；真实 spec 域伪造仍拒。

### 5.3 R-03【重要】预检/复验次序未定 —— 补

裁决 §5.2：「接在 `workspace_engine/single_candidate.rs:470-518` 与 `conversational_gate_parts/revision_turn.inc.rs:1158-1216` 的新候选编译/提升边界；**handler 预检用同一规则提前诊断，权威 completion 仍复验**」。裁决 §6.2：「C 的硬检置于新交付 compile 成功后、预检/refs 提升前」。

**改法**：D3/D9 写明三处位置与相对次序——① handler 预检（同规则、提前诊断、不吞 operational 错误、不授教学）→ ② C 硬检（compile 成功后、预检/refs 提升前）→ ③ 权威 completion 复验。tasks 2.2 增对应验收。

### 5.4 R-04【重要】守卫诊断的教学资格未定 —— 明确

裁决 §5.2/§5.3 要求「新增类别必须显式进入规格/typed 白名单，不因 `code` 前缀或一句『缺少』自动获权」（§1.5 同口径）。`AC-` 在双前缀下语法合法，**不触发** `invalid_work_item_id`。

**改法**：design 明确——命名域守卫诊断**新增独立 typed fault**，其教学重驱资格**不**通过 WSC-09 的 parse 语法类清单自动获得；该资格**由 mechanicalize 的 MODIFIED WSC-09（typed 资格清单，序 1）承载**，本 change 只提 typed 诊断并引用该清单，**不**自行扩展 WSC-09。WSC-31 增场景「WicV1 交付含 plan 域 `AC-` 被拒，且其教学资格按 WSC-09 typed 清单判定」。grammar.rs 的 `DIAGNOSTIC_CODES` 定长 4 数组需扩容一事写入 design（**只提，不改**）。

### 5.5 R-05【重要】「策略面独占」不成立 —— 改写

`identity_for_finding` 实有三处调用：`classify.rs:141`、`routing_platform_rescue.rs:178`、`routing_scope.rs:651`（决策 §5.3 另列 `routing.rs:468`）。这些文件在 close 的串行表内。

**改法**：design 删掉「`work_item_plan_policy/*` 本 change 独占」，改为「本 change 独占指纹代际与命名策略元数据面；`work_item_plan_policy/classify.rs` 由 isolate → close 串行共写，`identity_for_finding` 的全部调用点（4 处）由本 change 先行落地版本传递，close 后续只加闭合谓词不改版本语义」。另补：`evaluate.rs:184` 以**全字段字面量**重建 `RunHistory`，**新增代际字段 MUST 逐字段拷贝，MUST NOT 用 `..Default::default()`**（否则静默回落 v2）——写入 D6 与验收项。

### 5.6 R-06【重要】REQ-WSC-31 冻结点与三态未闭合 —— 补全

裁决 §5.2 明文：「在**真实 SC 首次生成准入以 CAS 冻结**：无历史 source/IR/评审/发布**且有新生事实**才为 `WicV1`，存量为 `LegacyPreserved`；未知历史不能伪装新生」。

**改法**：
- 场景 1 补「**且有真实新生事实**」合取条件（无 source/IR/评审/发布）。
- 场景 2 的触发「存量 lineage 首次被本平台识别」按 D3 明确为「首次准入时判定为存量」。
- 场景 4「未知」明确：**不冻结**、MUST NOT 伪装新生、MUST 按未知处理（待既有等待/人工面）；并写明「已冻结不可改」只约束已判定的 WicV1/LegacyPreserved。
- **CAS 载体必须钉死**：`IssueWorkItemPlan` 无 revision 字段（`models/lifecycle.rs:299`），仅 `plan.rs` 有 `membership_revision`；裁决 §5.2 要求「`lifecycle_store/plan.rs:36-125,132-187` 创建/ensure 与 SC 准入共同保持该元数据」。**定案**：命名策略元数据落在 **plan 记录（`IssueWorkItemPlan` 可缺省字段）+ plan 文件锁内 RMW**，CAS 载体＝plan 的 `membership_revision`（若不足则新增独立 revision 字段），**不得**依赖 session record 的 CAS。D3 写明载体与「重入/接管不覆盖已有策略」（裁决 §5.2 同口径）。
- 澄清同名异物：本文的「lineage」＝**命名策略代际**（`WicV1`/`LegacyPreserved`），与 `work-item-revisions/{plan}/lineage.json` 的 revision lineage **不是同一物**，须在 design 显式区分并统一措辞。

### 5.7 R-07【重要】回退风险只覆盖 RunHistory —— 补两项

补：`IssueWorkItemPlan` 无 `deny_unknown_fields`，旧二进制读改写会**静默丢弃**新命名策略字段（使「已冻结不可改」在回滚后失效）；`ReviewInvocationScope`/Raw DTO 带 `deny_unknown_fields`（`types.rs:259,360`），加版本字段同样令旧二进制拒收。发布顺序注明与新二进制同批；**旧二进制不得直接回滚**（裁决 §5.6）。

### 5.8 R-08【重要】不 bump compiler / IR 不漂移无场景 —— 补

裁决 §5.5：「本轮不 bump compiler version」「新增验收证明同旧 source 的 IR/内容寻址不漂移」「不同 compiler_version 仍按 freshness 拒绝，不能顺手关掉版本门」。

**改法**：WSC-30 增场景「WIC/AC 双前缀下同一旧 source 的 IR 与内容寻址不漂移」；WSC-30 或 WSC-31 增场景「不同 compiler_version 仍按 freshness 拒绝」。tasks 7.3 补对应映射。

### 5.9 R-09【重要】token 边界无契约场景 —— 补

**改法**：TOP-04 增场景「新代际稳定 ID 扫描：`WAC-001` 不被抽成 `AC-001`；`WIC-001 ≠ AC-001`；取与删规则完全一致」；design D5 补「v2 对 `WIC-001` 抽不到任何稳定 ID（`WI-`/`AC-` 皆非子串）→ 进 unstable 域强制人工，**这才是需要 v3 的直接理由**」，并保留原「不得断言 WAC 必然完整 hash 碰撞」的收窄（裁决 §5.3 末段）。

### 5.10 R-10【重要】授权例外被硬化 —— 改写

裁决 §5.2 原文：「旧 baseline 的 ID 改变/消失按**修复闭包与业务授权**处理，**不允许『仅前缀升级』替换整组身份**」。

**改法**：delta 与 D4 改为「**未经修复闭包与业务授权**，MUST NOT 删除、重编号或替换旧 baseline 的 criterion ID；经授权的删除走修复闭包既有合同。『仅前缀升级』替换整组身份在任何情况下 MUST NOT 发生。」

### 5.11 R-11【重要】大文件门禁未提 —— 补前置

裁决 §6.3 明列不得涨线的文件：`lifecycle_store/plan.rs`(1683)、`lifecycle_store/workspace.rs`(1730)、`revision_turn.inc.rs`(1505)。本 change 触及前两者。

**改法**：tasks 增**前置工作包**：「命名域模块放新文件；被改文件的完整内聚方法/类型先纯移动到相邻新文件再接线；纯移动不得顺手改无关行为」（裁决 §6.3 原口径）；同时登记 §2.2 的「决策六先落」。

### 5.12 R-12 ~ R-15【次要】

- **R-12** 改写为「摘要输入含代际标识；Verification 保持既有 `original_fingerprints` 逐项绑定」；并写明新代际**是否保留 `review_scope_v1:` 前缀**——**定案：保留前缀不变**，只在其摘要输入中纳入代际标识（裁决 §5.3「v2 旧 scope 哈希配方不变；v3 新 scope 绑定版本及 original 集合」；前缀属既有场景写死项，不动）。
- **R-13** 定义「受控会话」＝ `flow_kind=SingleCandidate` 创建路径；Legacy 保持缺省 v2。
- **R-14** 钉 `fingerprint_version` 的键名与取值域（`{"v2","v3"}`），未知值按**状态损坏**拒收（裁决 §5.3「未知算法版本按状态损坏拒收」）；并说明 `RunHistory` 带 `deny_unknown_fields` 需自定义反序列化，**不得**用 serde default 回落。
- **R-15** tasks 组 1/6/7 补 REQ 标注；7.4 的字面命令删（与首段自相矛盾）；行号订正（`workspace.rs:326`→325、`STABLE_ID_PREFIXES` 实为 `:8`、`remove_stable_ids` 实为 `:174`）；与 mechanicalize 的共享面登记改为**双边**（本 change 补记 `sc_author_teaching.rs`/fixtures/`prompt_contract` 由 mechanicalize 先行）。

## 6. close-contract-findings-before-pass 的处置（P1-01 ~ P3-13）

### 6.1 P1-01【P1】D4 判据自相矛盾 + 与现码/golden 冲突 —— 必改

**问题**（三方互斥）：
1. 描述段用「缺定位**或**缺许可 ⇒ 人工」，场景用「无定位**且**不存在修复证据」；两个混合格（有定位无证据 / 无定位有证据）无定义。
2. 「既有合法机械修复证据」在 `work_item_plan_policy/` 内**无对应代码概念**：`machine_repair` 只存在于 `contract_prerevision.rs`/`plan_preflight.rs`/`parsers.rs`（恒 None），`classify.rs`/`evaluate.rs` 不读它。
3. 现码 `fallback_class`（`classify.rs:174-183`）对 `Revise + 契约类 + contract_field 非空` 直接给 `Repairable`；`classify_finding`（`:134-140`）让 `class_hint` **优先于** `fallback_class`。若按「契约类不得被 advisory hint 判成 Advisory」改掉 hint 优先，则带 `contract_field` 的 suggestion 契约类会落到 `Repairable` ⇒ `is_automatic_repair_candidate`（`evaluate.rs:411-415`）为真 ⇒ **扣 `repairs_used`（A 池）** ⇒ 违反裁决 §4.4「**不增加自动重写次数**」。

**裁决 §4.3 的准确口径**（原文）：「disposition 非 actionable 仍先 Advisory；**未排除契约类不能被 advisory hint/rawPass 判成 Advisory**。**仅此标签不足自动 Repairable：无可靠定位/服务端授权证据的 #1 落 HumanRequired；已有合法机械修复证据的沿既有许可，不凭 category 或 severity 新造 L1**」。

**定案——两级判定（闭合资格 / 自动修复许可分离）**：

```
requires_closure      = effective_disposition == actionable
                     && category ∈ {contract_gap, self_contradiction}
                     && 未被 owner 显式排除          ← 纯谓词，无副作用

auto_repair_permitted = requires_closure
                     && 存在「服务端授权的既有合法机械修复许可」证据
                     && 该 finding 具备「可验证定位」证据
```

**分类落点**：
- `requires_closure` 为真且 `auto_repair_permitted` 为真 ⇒ `Repairable`（**沿既有许可**，不新造 L1）。
- `requires_closure` 为真且 `auto_repair_permitted` 为假 ⇒ **`HumanRequired`**（不得 `Advisory`，也不得 `Repairable`）。
- `requires_closure` 为假 ⇒ 维持既有行为（范围外 advisory 不变）。

**「可验证定位」与「既有合法机械修复许可」的落地口径**（必须落到现码可判定项，不得造新概念）：
- **可验证定位** ＝ 该 finding 能指到具体契约面：`contract_field` 非空，**或** 携带可定位的 typed 事实引用（机械侧 typed IR/structure 诊断）。裁决 §4.3 的「不凭 category 或 severity 新造 L1」意味着：**category/severity 本身不构成定位证据**。
- **既有合法机械修复许可** ＝ 服务端既有 L1 机械修复授权链路的在场证据（`RepairReservation`/机械修复指令链），**不是** `class_hint`、**不是** category、**不是** severity。

**因此 `fallback_class` 必须改**：`Revise + 契约类 + contract_field 非空 ⇒ Repairable` 这条**仅凭 category+field** 的路径 MUST NOT 单独构成 `Repairable`；按新判定式，无服务端许可证据时落 `HumanRequired`。`class_hint=repairable` 亦 MUST NOT 单独构成自动调用许可（裁决 §4.3 routing 行「模型 hint 不能单独给自动调用许可」）。

**tasks 必须显式承接 golden 影响**（不得假装不变）：`work_item_plan_policy/tests_classify.rs:24-114,248-270` 与 `review/disposition_override.rs:347-363` 中依赖「hint wins」或「Revise+field ⇒ Repairable」的既有断言，按裁决 §4.4 的矩阵（category 两类 × suggestion/must_fix/blocking × disposition actionable/advisory/缺省 × hint 三类+**缺省** × Pass/Revise/NeedsHuman）更新；**原真实 golden findings 不改写成新造输入**（裁决 §4.4）。

### 6.2 P2-02【P2】谓词事实如何送达 policy —— 定义为纯函数 + 可信入口传 scope

`classify_finding(raw_verdict, finding)` 无 scope 参数，有 6 个调用点。裁决 §4.3 明写「作用域由**可信 session/解析入口**传入，不由模型标签授予」；`structured_output.rs:251` 以 `workspace_type == WorkItemPlan` 分流到 `parse_work_item_plan_review_value`，该事实非模型可控（真实可信入口）。

**改法**：
- 判定式实现为**纯函数** `requires_closure(finding, scope) -> bool`，`scope` 为**枚举**（由 `WorkspaceType`/`WorkItemPlanReviewScope` 等既有类型派生，非 wire 字段）。
- `classify_finding` 增 `scope` 参数；**6 个调用点逐一列名并写明各自 scope 取值来源**：`classify.rs:70`（自身）、`routing_scope.rs:88`、`routing_platform_rescue.rs:71,267`、`prompts/review_context.rs:557`、`routing.rs:468`（经 `classify_review`）。
- 裁决 §4.3 的 `review/closure_policy.rs` 为**唯一**谓词实现处，其他面 MUST NOT 各自复制判定式。

### 6.3 P2-03【P2】「raw verdict」语义 —— 明确

`routing.rs:344,444` 取的是**已被 `effective_work_item_plan_review_verdict` 重开后**的 verdict 序列化值，`finalize_review_verdict` 注释称「provider 原始 verdict」实为 post-parse 值。

**改法**：delta 写明「本条的 raw verdict 指**未经有效必关闭重开**的原始判定」；并把不可达场景改写为可达表述——「当某路径上未发生重开时（如读持久化旧 verdict、`routing_scope.rs:88`、`routing_platform_rescue.rs:71`、`review_context.rs:557`），原始 `pass` 同样 MUST NOT 使未排除契约类自动放行」。**禁止**保留 live 路径不可达的场景而不说明。

### 6.4 P2-04【P2】消费面清单不完整 —— 补齐

补入遗漏两面并写明同口径：`sc_revision.rs:147`（向作者声明 severity=suggestion 条目为「建议项」，重开后的契约缺口在返修轮仍显示可选 ⇒ 必须改为按**有效必关闭**呈现）、`plan_outline/revision.rs:627-638`（`requires_impact_closure` 仅认 blocking/must_fix ⇒ 必须改认有效必关闭）、`routing_scope.rs:84-94`（在状态表但不在 D3）。design 的「当前状态」表行数与「四套」措辞对齐实际面数。

### 6.5 P2-05【P2】作用域边界未定义 —— 补取值表

**改法**：design 增「WorkItemPlan 作用域取值表」：`WorkItemPlanReviewScope::{Outline,Item,Batch}`（`parsers.rs:304-308`）与非 SingleCandidate 的 legacy flow **各自是否启用例外**，逐项写明。delta 明确「本 Work Item」＝ 该 finding 的 `target_work_item_id` 指向的 Work Item，并写明与 `with_platform_disposition_rules` 现行行为的差异。同时**必须**声明：平台 disposition 硬化已在通用 strict/lenient 解析器**无条件生效**（`parsers.rs:750`、`lenient_findings.rs:136`），**只有 severity 豁免是 WorkItemPlan 专属**——避免实施者把硬化也收窄。

### 6.6 P2-06【P2】共享面登记不一致 —— 按 §2.2 重写

- 删「`work_item_plan_policy/*` 本 change 独占」类表述（见 §5.5）。
- 与 isolate 的次序登记（isolate → close），并写明 close 消费 isolate 的版本化 scope。
- 与 quota-pool / mechanicalize 的次序写**双边**（在对方目录登记本 change，或在 design 注明「对方目录登记由集成 owner 统一补齐」并列入 tasks 的协调项）。
- 删除「与 quota-pool 共享 `tests_classify.rs`」的无依据表述（quota-pool 全目录无 `classify` 引用）。
- 裁决 §1.1(3) 与 §1.3 的「第 C 章绑定」：裁决 §1.1(3) 指出 C 的 handler 复用 provider 函数**不等于**属于 A/B；故本 change 不得把 bound 的 C 纳入自身共享面。裁决 §1.4「不修改 `fix-author-rewrite-quota-pool/` 的 C 范围」。

### 6.7 P2-11【P2】裁决 §4.3 两条落地条款缺失 —— 补

① 「原始模型文本保留用于审计，但 **policy 不得重新解析 raw 后绕过 owner 排除**」——design/proposal/delta/tasks 全部补写该禁止（含「历史重建路径同样禁止」）。② 「**模型 hint 不能单独给自动调用许可**」——delta 的禁止清单由「不得凭 `category` 或 `severity`」扩为「不得凭 `category`、`severity` **或 `class_hint`**」；现码 `class_hint=repairable` 即直接 `Repairable`（`classify.rs:135-138`，`tests_classify.rs:58-69`「hint wins」），与 §6.1 同源，一并修。③ 裁决 §4.3 列出的 `review/closure_policy.rs`、`closure_policy_tests.rs` 落点必须进 design 与 tasks。

### 6.8 P3-07 ~ P3-13【次要】

- **P3-07** 删去 `routing_platform_rescue.rs` 的「平台救回路径需用同一谓词」论证（它只处理 `candidate_compile_failure:` 前缀的平台 finding，不涉模型契约类 finding）；若要保留则补可复现反例。
- **P3-08** tasks 矩阵维度改为 `class_hint（repairable/human_required/advisory/**缺省**）`。
- **P3-09** 6.2 前置改为「**mechanicalize 9.1 结论文档存在且已评审**」；5.2 注明「同一指纹复评停止」专项 fixture **必须构造有稳定 ID 的 finding**（`contract_field=None` 且无稳定 ID 者走 `UnstableFindingIdentity`，`evaluate.rs:335-347`，不参与该闸门）。
- **P3-10** historical 入口枚举两类并各自写明：① 重新解析型 `parse_historical_review_json` / `parse_historical_work_item_plan_review_json`（`routing.rs:1279,1286`、`owner_guidance.rs:335`）；② 纯 serde 反序列化型 `deserialize_historical_review_verdict`（`historical_review.inc.rs:38`、`lifecycle_recovery.rs:45`、`review_context.rs:546`、`routing_scope.rs:843`）。
- **P3-12** 补两处场景→任务映射：WSC「缺可验证定位时转人工且不自动调用 author」→ 3.1；TOP「分类优先级与原始 verdict 来源不冲突」→ 2.4。
- **P3-13** D4 补该类人工门卡的 owner 可读呈现口径：**沿用平台渲染的技术细节呈现机制**，并说明 REQ-WSC-11/15 的 `owner_question`/`owner_options` 只在模型提供时强制（`parsers.rs:395-401`），平台渲染不替代 owner 提问。

## 7. 主方案 v1.3 的处置（effort I-19 + X2 归属）

### 7.1 I-19 —— G-1 / G-2 / G-3 / G-6 / §8.0 E 行

主方案 `cadence/designs/2026-11-08_技术方案_WIG生成加速与收敛性加固_v1.0.md` 的 v1.3 残留，须逐条改为裁决口径：

| 位置 | 现状 | 改为 |
|---|---|---|
| `:795` G-1 | 「`claude_code_provider/mod.rs:311-344`（direct）/:351-389（LC）**加 argv**，或 `workspace_engine/prompts.rs:545` 注入」；「44 分钟 → 预计 10-15 分钟（需 A/B 复测）」；「**需 A1 裁决**」 | 删「加 argv / adapter 硬编码」路径（裁决 §3.3 明文否决「adapter 硬编码」）；改为「由 `explicit-sc-author-effort` 的 SC author 出口注入（新增 `prompts/sc_author_input.rs` + `prompts/sc_launch_policy.rs`），冻结策略随原运行保存」；删「44 分钟 → 10-15 分钟」（裁决 §3.2：「不承诺 44 分钟→10–15 分钟」）；「需 A1 裁决」改为「已由裁决 §3.1 定案：默认 high」 |
| `:796` G-2 / `:800` G-6 | 注入点写 `workspace_engine/prompts.rs:545` | 注入点改为 SC author 出口模块（`prompts/sc_author_input.rs`），并注明「G-2/G-6 是否在本期实施由裁决 §3.1(1) 的范围限定裁定（本期只控制 Claude SC author 生成/教学链）」 |
| G-3 等 | 指向 `prompts.rs:545` | 同上 |
| `:922` §8.0 E 行 | 「＋ `isolate-plan-criterion-namespace/` 的 **WSC-06 delta**」 | 见 §7.2 |
| `:87` §5 行 G | 「`claude_code_provider/mod.rs` / `workspace_engine/prompts.rs:545`」；「**中-高（G-1 需 owner 裁决）**」 | 文件与注入点同上；「G-1 需 owner 裁决」改为「G-1 已由裁决 §3 定案」 |
| `:951` §6 行 G | 「需先裁决 §7.1」 | 改为「G-1 已由裁决 §3 定案；G-2/G-3/G-4 可先行」 |

### 7.2 X2：MODIFIED REQ-WSC-06 无 owner —— 订正 §8.0，不夹带

主方案 `:446-462`（X2）已自述：「方案 E 的裁剪**必须带 MODIFIED REQ-WSC-06 delta**……⇒ 方案 E 不是『可先行』包：E-3 可先行，**E-1/E-4 需 delta**」。而 `:922` 却称该 delta 由 isolate 承载——**isolate 全目录零命中 WSC-06**，属虚假归属。

**定案**：
- **隔离 change 不夹带**。isolate 的 scope 是命名域 + 指纹代际（裁决 §5），与「reviewer 投影裁剪」不同物；夹带会违反裁决 §1.4「去重纪律」与本文件 §1.4。
- 主方案 `:922` 与 `:937` 改为：「E-1/E-4 的 MODIFIED REQ-WSC-06 delta **当前无 owner**；方案 E 的**裁剪部分**在另立 change 承载该 delta 前**不可先行**；E-3（字段白名单对齐 schema）仍可先行。」
- 主方案新增一行登记该待办（**不得**新建 change 目录，本次只订正归属与门禁）。
- 同时订正 `:950` 中「E-1/E-4 另需 MODIFIED REQ-WSC-06」的排序结论：明确「E-1/E-4 排在 WSC-06 delta 承载方之后」。

## 8. 验收口径（每个工作包必跑，真实输出写入报告）

```bash
cd /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-wig-optimize-11-08
# 1) change 级严格校验
openspec validate <change-name> --strict
# 2) MODIFIED 场景保全（主规格原场景标题逐字仍在）
python3 - <<'EOF'
import re,glob,os
def scen(p):
    return re.findall(r'^#### Scenario: (.+)$', p, flags=re.M)
def blocks(path):
    txt=open(path,encoding='utf-8').read()
    out={}
    for p in re.split(r'(?=^### Requirement: )', txt, flags=re.M):
        if p.startswith('### Requirement: '):
            out[p.split('\n')[0].strip()]=(scen(p), '\n'.join(p.split('#### Scenario')[0].split('\n')[1:]).strip())
    return out
EOF
# 3) 归档后主规格 strict 校验（关键门禁）
cp -r openspec /tmp/arch-<change>/ && cd /tmp/arch-<change>/openspec \
  && openspec archive <change-name> --yes >/dev/null 2>&1 \
  && openspec validate --specs --strict   # 必须 79 passed, 0 failed
```

**硬性通过条件**：
1. `openspec validate <change> --strict` **exit 0**。
2. MODIFIED requirement 的**主规格原有场景标题与正文逐字保留**（不得删改、不得改写）。
3. **归档后** `openspec validate --specs --strict` 全绿（**79 passed, 0 failed**）；描述段 ≤500 字符。
4. 本文件点名的每一条缺口，在对应 change 的 `design.md`/`tasks.md`/`specs/**` 中**有对应文字**，且可被 grep 到。
5. `git status` 显示**只有本工作包负责的文件**被修改。
6. **禁止**改 `openspec/specs/**`、`src/**`、其它 change 目录。
7. 报告须含：改了哪些文件、每条编号的处置位置（文件:行）、真实命令输出、未处理的项及原因。

# Spec Delta: dangling-reference-governance

## Purpose

为「已删实体留下的旁支/记账类悬空引用」建立三层治理契约：读侧只对旁支引用容错（跳过该条目并告警，主记录缺失语义与真实错误一律不变）、全量巡检只报不改、按类型幂等销账（留痕并在清理后自证归零）。

> **r2 修订说明（2026-10-28）**：本 delta 经两位独立审查者裁定「需修」后修订。修订要点：
> ① 「已删」证据的 L1 收紧为**四条同时成立**（墓碑存在 + `phase == deleted` + `deleted_at` 有值 + 无 pending failure）；
> ② 销账证据契约统一为「**L1 可删；L2 仅对规则表明示的类型且其专属可算条件成立时可删；其余 L2 一律只报不删**」，并新增**目标类型 → 存在性来源 → 唯一可达上游 → L2 专属可算条件 → 可删证据等级**表（REQ-DRG-02）；
> ③ attempt 目标类型的存在性核对**拆成两路**（coding attempt / 规划面 workspace session），两者都不存在才算悬空（REQ-DRG-21）；
> ④ intent 销账新增**四道前置条件**（enrollment 文件锁内重读 + 只认 L1 + generation phase 限制，REQ-DRG-20）；
> ⑤ 确认令牌改为 **findings 规范化摘要**，`--apply` 入口内部重跑巡检比对（REQ-DRG-22）；
> ⑥ 巡检新增**「不可判定」隔离**与**「只报不删残留」栏**，归零口径收窄（REQ-DRG-17、REQ-DRG-23）。
> 全部修订**未放宽**任何既有边界：只吞「目标已删」、主记录缺失语义不变、墓碑永不删、未归类只报不删、默认 dry-run、幂等 + 留痕 + 自证。
>
> **r3 修订说明（2026-10-28，闭环复审后）**：本轮只做**文本级互斥消解**，不改 owner 已拍板的任何边界。要点：
> ① **L2 的口径统一为「该类型专属可算条件」**（REQ-DRG-02），证据等级表的**等级列写进本 spec**；「唯一可达上游不可达」的双条件表述仅在 `Plan` 一类上可解析，其余按表降级；
> ② **「不可判定」不再被重载**：拆为「**不可判定（读取失败）**」与「**证据不足 / 删除未完成（只报不删残留）**」两类，全文口径统一（REQ-DRG-02 / 11 / 23）；
> ③ **REQ-DRG-19 的硬约束加限定**：不删的是「不具备 L1 证据**且不满足该类型 L2 专属条件**」的旁支记录（窗口簇按 L2 删，不与本条冲突）；
> ④ **intent 销账条款引用 REQ-DRG-20 四前置**（REQ-DRG-12），REQ-DRG-20 明确「enrollment 状态 / 相位 / L1 证据三项**在锁内重读**」，且「已终止」以 `terminated_at` 有值为准；
> ⑤ **幂等不得免除锁语义**（REQ-DRG-15）：「缺失视为已清」只用于重入；
> ⑥ **REQ-DRG-16 的留痕落点去「既有」**：落在状态根下**单一全局追加式 JSONL 台账**，不进 issue 级墓碑、不扩 `DeletionEntityKind`；落点归属留 owner 拍板；
> ⑦ **REQ-DRG-21 的路二数据源改为「枚举」**：遍历全部 project × issue 建 session id 集合，禁用带写回副作用的单条会话读取，枚举失败归不可判定；
> ⑧ **REQ-DRG-12 的窗口删除粒度改为按 attempt 聚合**：该 attempt 全部窗口都满足才删；
> ⑨ **REQ-DRG-22 写死残留条目是否进摘要**（进），并排除未归类条目；
> ⑩ **REQ-DRG-09 点名 intent 的五个授权引用字段与 enrollment 的全部引用字段**；REQ-DRG-02 补「`deleted_at` 缺失」场景。
>
> **r5 修订说明（2026-10-28，owner 拍板后）**：本轮把**人工触发改为自动执行**、并把①层容错**接到产品读路径**。要点：
> ① **REQ-DRG-13 改为自动模式**：销账在**服务启动时自动跑一次**，删除判据 = 白名单类型 + 证据达标；**删除人工令牌环节**（原「默认 dry-run + 确认令牌」作废）；人工 CLI 入口一并删除；
> ② **REQ-DRG-22 改口径**：清单摘要**仅用于审计关联**（核对两次巡检是否同一现场），**不再是删除凭据**（其计算口径逐字保留）；
> ③ **新增 REQ-DRG-24**：①层容错接入产品读路径——目标**已确认不存在**时跳过该旁支条目 + 记一条①层告警；
> ④ **REQ-DRG-11** 补一句：清单由**服务启动时的巡检自动产出**。
> 本轮**未放宽**任何删除边界：白名单 + L1 墓碑 / W-1..W-4 证据门不变，幂等 + 留痕 + 自证归零不变，主记录缺失语义不变。
>
> **r4 修订说明（2026-10-28，实施审查后）**：本轮把**存在性判定的三态语义**写进契约，并落实此前挂「待核实」的读 API，**不改变任何 owner 边界**。要点：
> ① **REQ-DRG-01 改为三态判定**（`Exists` / `Missing` / `Unknown`）：存在性数据源或所需作用域在本判定面不可用时 MUST 归「不可判定」（REQ-DRG-23），MUST NOT 判为「不存在」；目标类型名统一为 `logical_member`（正文原误写 `logical_repository(member)`，与 REQ-DRG-02 附表、设计 §3.1.2 / §3.3.2 及实现一致）。
> ② **REQ-DRG-02 附表 #7–#10 的存在性来源落实为具体读 API 与作用域**：checkout / aggregate_index / logical_member 的记录落在 `logical-codebases/<lc_id>/`，核对 MUST 先按 `LogicalCodebaseStore::list(project_id)` 枚举 LC 再以 `for_lc` 作用域化读取；enrollment 的来源 `{issue_root}/automation-enrollment.json` 已可读（`IssueAutomationStore::get`），不再挂「待核实」。附表「两类非可删结论」的「证据不足」行补前提「**目标已判定为不存在**」。
> ③ **REQ-DRG-23 的「不可判定」定义扩为「读取失败 **或** 所需存在性数据源 / 作用域不可用」**；REQ-DRG-17 / 22 明确判为 `Unknown` 的条目 MUST NOT 计入悬空条目数、MUST NOT 计入归零、MUST NOT 进入摘要的发现集合。

### 目标行为映射

| 目标行为 | 对应需求 |
|---|---|
| A1 按字段名判定引用目标实体类型 | REQ-DRG-01 |
| A2 「目标已删」的证据分级与可删等级表 | REQ-DRG-02 |
| A3 读侧容错：旁支条目跳过 + 告警 | REQ-DRG-03 |
| A3b ①层容错接入产品读路径 | REQ-DRG-24 |
| A4 主记录缺失语义不变 | REQ-DRG-04 |
| A5 只吞「目标已删」这一种错 | REQ-DRG-05 |
| A6 fail-closed 读路径不接入容错 | REQ-DRG-06 |
| A7 告警形状、去重与上界 | REQ-DRG-07 |
| B1 巡检范围 | REQ-DRG-08 |
| B2 巡检判据与未归类 | REQ-DRG-09 |
| B3 dry-run 只报不改 | REQ-DRG-10 |
| B4 巡检清单字段格式与落点 | REQ-DRG-11 |
| C1 销账规则表 | REQ-DRG-12 |
| C2 闸 1：自动模式（启动时自动销账，无人工令牌） | REQ-DRG-13 |
| C3 闸 2：白名单外不删 | REQ-DRG-14 |
| C4 闸 3：幂等 | REQ-DRG-15 |
| C5 闸 3：留痕 | REQ-DRG-16 |
| C6 闸 3：清理后自证归零 | REQ-DRG-17 |
| D1 承接在途 enrollment 语义 | REQ-DRG-18 |
| D2 硬约束 | REQ-DRG-19 |
| D3 intent 销账的四道前置条件 | REQ-DRG-20 |
| D4 attempt 目标的两路存在性核对 | REQ-DRG-21 |
| D5 清单摘要的计算口径与审计用途 | REQ-DRG-22 |
| D6 巡检的「不可判定」隔离 | REQ-DRG-23 |

## ADDED Requirements

### Requirement: 引用目标类型按字段名判定（REQ-DRG-01）

系统 MUST 以「记录类型 → 引用字段 → 目标实体类型」的静态映射判定引用指向，判定依据 MUST 是字段名而非实体 id 前缀。目标实体类型 MUST 覆盖 story_spec、design_spec、plan、session、attempt、checkout、aggregate_index、logical_member、enrollment 九类。系统 MUST NOT 依赖实体 id 的字符串前缀来推断类型。

存在性判定 MUST 为**三态**：`Exists`（明确命中）/ `Missing`（**明确未命中**）/ `Unknown`（**无法判定**）。存在性数据源或所需作用域在本判定面不可用时，MUST 判 `Unknown` 并归「不可判定」（REQ-DRG-23），MUST NOT 判 `Missing`；读取失败同样 MUST NOT 判 `Missing`，MUST 原样上抛。核对数据源见 REQ-DRG-02 附表「存在性来源」列。

#### Scenario: 无前缀的 enrollment id 仍可判定

- **WHEN** 某记录以 `enrollment_id` 字段引用一个授权记录，且该 id 是不带任何固定前缀的字符串
- **THEN** 系统 MUST 按字段名判定其目标类型为 enrollment，并以 `{issue_root}/automation-enrollment.json`（`IssueAutomationStore::get`，`src/product/issue_automation_store.rs:69`）核对目标是否仍存在，MUST NOT 因 id 无前缀而放弃判定；当判定面不持有该记录所属的 project/issue 作用域、无法读该文件时，该条 MUST 归「不可判定」（REQ-DRG-23），MUST NOT 判为「不存在」

#### Scenario: 强类型 checkout 引用仍可判定

- **WHEN** 某记录以 `checkout_id` 字段引用一个 checkout 记录
- **THEN** 系统 MUST 按字段名判定其目标类型为 checkout，并以 `LogicalCodebaseStore::list_checkouts`（`src/product/logical_codebase/store.rs:792`；单条读 `load_checkout`，`:777`）核对目标是否仍存在；因 checkout 记录落在 `logical-codebases/<lc_id>/`，核对 MUST 先按 `LogicalCodebaseStore::list(project_id)`（`:249`）枚举 LC、再以 `LogicalCodebaseStore::for_lc(paths, lc_id)` 作用域化读取；判定面未持有 LC 上下文时该条 MUST 归「不可判定」（REQ-DRG-23），MUST NOT 判为「不存在」

#### Scenario: 表外记录类型不猜引用语义

- **WHEN** 某条记录的类型不在静态映射表内
- **THEN** 系统 MUST NOT 为它推断引用目标类型，该记录 MUST 被归为未归类

### Requirement: 「目标已删」证据分级与可删等级表（REQ-DRG-02）

系统 MUST 将「目标记录不存在」区分为两级证据。

**L1 直证** MUST 由以下四条**同时成立**构成：① 存在删除墓碑；② 删除阶段为**已完成**；③ 带完成时间；④ **无**待人工处置的删除失败事实。任一条不成立 MUST NOT 判 L1。

**L2 推定** MUST 要求「目标不存在」与「**该类型的专属可算条件**」（见等级表）同时成立，MUST NOT 用统一的「上游不可达」替代。

系统 MUST 维护一张「目标类型 → 存在性来源 → 唯一可达上游 → L2 专属可算条件 → 可删证据等级」的表作为销账白名单的**单一事实源**（表体见末尾）：**L1 可删**；**L2 仅对表中标 L2 且专属可算条件成立的类型可删**；其余一律只报不删；无法定义上游者 **MUST 降级为只报不删**；**墓碑 MUST 永不删除**。

「不可判定」指读取失败**或所需存在性数据源 / 作用域不可用**（REQ-DRG-01 / 23）；已判「不存在」但证据不足者 MUST 归「证据不足 / 删除未完成」，MUST NOT 归「不可判定」。

#### Scenario: 墓碑直证已删

- **WHEN** 被引用的 story_spec 存在对应的删除墓碑记录，且该记录的删除阶段为已完成态、带完成时间、且无待人工处置的删除失败事实
- **THEN** 系统 MUST 判定其证据等级为 L1，且在销账规则允许的目标类型上 MUST 允许执行删除

#### Scenario: 删除进行中的墓碑不算 L1

- **WHEN** 被引用的 plan 存在删除墓碑记录，但该记录的删除阶段仍为进行中态
- **THEN** 系统 MUST NOT 判定该目标为 L1，MUST 归入「证据不足 / 删除未完成」类并只报不删，MUST NOT 归入「不可判定」

#### Scenario: 墓碑缺完成时间不算 L1

- **WHEN** 被引用的 story_spec 存在删除墓碑记录，其删除阶段为已完成态但记录不带完成时间
- **THEN** 系统 MUST NOT 判定该目标为 L1，MUST 归入「证据不足 / 删除未完成」类并只报不删

#### Scenario: 带删除失败事实的墓碑不算 L1

- **WHEN** 被引用的 plan 存在删除墓碑记录，但该实体同时存在待人工处置的删除失败事实
- **THEN** 系统 MUST NOT 判定该目标为 L1，MUST 归入「证据不足 / 删除未完成」类并只报不删

#### Scenario: 推定需专属条件成立

- **WHEN** 某目标类型不在可删等级表的 L2 列中，且其记录不存在
- **THEN** 系统 MUST NOT 判定该目标已删，该条目 MUST 只报不删

#### Scenario: 推定专属条件成立

- **WHEN** 某目标类型在可删等级表中被标为 L2，且其专属可算条件成立
- **THEN** 系统 MUST 判定其证据等级为 L2，并在清单中如实标注为推定而非墓碑

#### Scenario: 证据不足不得冒充不可判定

- **WHEN** 某条引用的目标已被判定为不存在（`Missing`）但证据等级不足（L1 四条缺一、或 L2 专属条件不成立）
- **THEN** 该条目 MUST 计入悬空条目数并归入「只报不删残留」节，MUST NOT 归入「不可判定」节

#### Scenario: 无法定义上游的类型降级

- **WHEN** 某目标类型在可删等级表中被标为「上游无法定义」
- **THEN** 系统 MUST 把它归为只报不删，MUST NOT 在任何销账路径中删除依赖它的记录

#### Scenario: 墓碑本身永不删除

- **WHEN** 销账在状态根下执行
- **THEN** 实体删除墓碑记录 MUST 保持不变

**附：证据等级表（销账白名单的单一事实源，与技术方案 §3.1.2 逐行一致）**

| # | 目标类型 | 存在性来源 | 唯一可达上游 | L2 专属可算条件 | 可删证据等级 |
|---|---|---|---|---|---|
| 1 | `StorySpec` | `list_story_specs` | 无上游（根实体） | 不可用（无上游可解析） | **L1** |
| 2 | `DesignSpec` | `list_design_specs` | 无上游（根实体） | 不可用（无上游可解析） | **L1** |
| 3 | `Plan` | `list_live_plan_ids` | plan → work item group → story/design（须 plan 记录本身才能解析） | 不可用（plan 记录已消失即无法解析） | **L1** |
| 4 | `Session` | `list_workspace_sessions` | session → enrollment 绑定 | 不可用（无独立墓碑，且缺失属正常中间态） | **只报不删** |
| 5 | `Attempt`（路 1 coding attempt） | `get_attempt_by_id` | attempt 记录无 `plan_id`，上游不可从 attempt 解析 | **由窗口记录自身给出的四条**（REQ-DRG-12） | **L2** |
| 6 | `Attempt`（路 2 规划面 workspace session） | `list_workspace_sessions` | 同 #4 | **由窗口记录自身给出的四条**（REQ-DRG-12） | **L2** |
| 7 | `Checkout` | `LogicalCodebaseStore::list_checkouts`（`src/product/logical_codebase/store.rs:792`）/ `load_checkout`（`:777`），须以 `for_lc` 作用域化；LC 由 `LogicalCodebaseStore::list(project_id)`（`:249`）枚举 | 未核实，无法定义 | 不可用（上游无法定义） | **只报不删** |
| 8 | `AggregateIndex` | `AggregateIndexStore::get`（`src/product/logical_codebase/aggregate_index/store.rs:72`），须以 `for_lc` 作用域化 + 同上 LC 枚举 | 未核实，无法定义 | 不可用（上游无法定义） | **只报不删** |
| 9 | `LogicalMember` | `LogicalCodebaseStore::list_members`（`src/product/logical_codebase/store.rs:751`）/ `load_member`（`:736`）＋ `LogicalCodebaseManifest.member_ids`（`store.rs:32`），须以 `for_lc` 作用域化 + 同上 LC 枚举 | 未核实，无法定义 | 不可用（上游无法定义） | **只报不删** |
| 10 | `Enrollment` | `{issue_root}/automation-enrollment.json`（`IssueAutomationStore::get`，`src/product/issue_automation_store.rs:69`） | 无上游（授权根） | 不可用（永不删） | **只报不删（永不删）** |

**附：两类非可删结论的区分（MUST NOT 混用）**

| 类别 | 触发条件 | 归属节 | 计入悬空条目数 | 计入归零 |
|---|---|---|---|---|
| **不可判定（读取失败 / 数据源或作用域不可用）** | 判定目标存在性时因 IO / 解析 / 权限错误**无法完成判定**，**或**该目标所需存在性数据源 / 作用域在本判定面不可用（存在性判为 `Unknown`） | 清单「不可判定」节（REQ-DRG-23） | 否 | 否 |
| **证据不足 / 删除未完成** | **前提：目标已判定为不存在（`Missing`）**；在此前提下证据等级不足 L1（墓碑进行中、墓碑缺完成时间、存在待人工处置的删除失败事实、无墓碑、上游无法定义、降级类型） | 清单「只报不删残留」节（REQ-DRG-11） | 是 | 否 |

### Requirement: 读侧容错只对旁支引用生效（REQ-DRG-03）

系统 MUST 提供旁支/记账类记录的容错列举能力：自动化 intent 清单、reviewer 窗口清单、reviewer 快照清单、实体身份台账清单。遇到指向已删对象的条目时，系统 MUST 跳过该条目并产生一条告警，MUST NOT 使整次列举失败。系统 MUST NOT 改变任何既有 fail-closed 读路径的语义。

自动化 intent 清单 MUST 只枚举两个确定的文件名（自动化 plan intent 与自动化 generation intent），MUST NOT 用通配符枚举该目录下的其它文件——特别是 MUST NOT 把自动化 enrollment 记录纳入「悬空条目」判定。

#### Scenario: 悬空 intent 条目被跳过

- **WHEN** 列举某 issue 的自动化 intent，其中一条 intent 的 source 指向已删的 story_spec
- **THEN** 该 intent 条目 MUST 不出现在列举结果中，MUST 产生一条告警，且该次列举 MUST 正常返回其余条目

#### Scenario: 悬空窗口条目被跳过

- **WHEN** 列举 reviewer 窗口，其中一条窗口记录引用的目标在两路核对下均不存在
- **THEN** 该窗口记录 MUST 不出现在列举结果中，MUST 产生一条告警，且该次列举 MUST 正常返回其余窗口

#### Scenario: 仍被引用的窗口条目被保留

- **WHEN** 列举 reviewer 窗口，其中一条窗口记录引用的目标在两路核对之一命中
- **THEN** 该窗口记录 MUST 出现在列举结果中，MUST NOT 被当作悬空条目跳过

#### Scenario: 悬空台账条目被跳过

- **WHEN** 列举实体身份台账，其中一条绑定指向已删的 work item plan
- **THEN** 该台账条目 MUST 不出现在列举结果中，MUST 产生一条告警，且该次列举 MUST 正常返回其余条目

#### Scenario: intent 清单不枚举 enrollment 记录

- **WHEN** 列举某 issue 的自动化 intent 清单
- **THEN** 该清单 MUST 只包含两个 intent 文件，MUST NOT 包含自动化 enrollment 记录

#### Scenario: 既有列举语义不变

- **WHEN** 对比启用本能力前后的既有 reviewer 窗口列举与实体身份台账按 owner 反查行为
- **THEN** 两者的返回集合与失败语义 MUST 完全一致

### Requirement: 主记录缺失语义一律不变（REQ-DRG-04）

系统 MUST NOT 改变主记录缺失时的错误码与文案。既有 `story_spec_not_found` 等错误码 MUST 逐字不变。系统 MUST NOT 让主记录缺失被降级为「跳过并告警」，也 MUST NOT 让主记录缺失导致整次列举失败。

#### Scenario: 主记录缺失仍按既有错误码拒绝

- **WHEN** 请求一个已被删除的 story_spec
- **THEN** 系统 MUST 仍返回既有错误码 `story_spec_not_found` 与既有文案，MUST NOT 改为跳过或静默返回空

#### Scenario: 主记录缺失不得降级为容错

- **WHEN** 某次列举的主记录部分引用了已删对象
- **THEN** 系统 MUST 按既有语义报错，MUST NOT 把该主记录当作旁支条目跳过

### Requirement: 只吞「目标已删」这一种错（REQ-DRG-05）

读侧容错 MUST 只吞「引用目标记录不存在」这一种错。JSON 解析失败、schema 不符、IO 失败、权限错误 MUST 照旧上抛，MUST NOT 被降级为「跳过该条目」。

**例外（reviewer 窗口面）**：reviewer 窗口清单 MUST 沿用既有 `ReviewerWindowStore::list_windows` 的既有语义——单条窗口记录读失败按既有语义「跳过该条并告警」，其目录级读取失败仍 MUST 照旧上抛；其余三面（intent / 快照 / 实体台账）读失败 MUST 一律以 `Err` 上抛，MUST NOT 被降级为「跳过该条目」。本例外 MUST NOT 扩展到其它面或目录级失败，MUST NOT 改写既有函数行为。

#### Scenario: 解析失败照旧上抛

- **WHEN** 列举自动化 intent / reviewer 快照 / 实体身份台账时遇到一条 JSON 解析失败的旁支记录
- **THEN** 系统 MUST 照旧报告该解析错误，MUST NOT 把它当作悬空条目跳过并告警

#### Scenario: IO 失败照旧上抛

- **WHEN** 列举过程中某个分区目录不可读
- **THEN** 系统 MUST 照旧报告该 IO 错误，MUST NOT 静默返回部分结果

#### Scenario: 目标不存在被吞

- **WHEN** 某条旁支记录引用的目标记录确实不存在
- **THEN** 系统 MUST 跳过该条目并产生告警，MUST NOT 报错

#### Scenario: reviewer 窗口单条坏记录沿用既有跳过语义

- **WHEN** 列举 reviewer 窗口，其中一条窗口记录自身 JSON 解析失败或 schema 不符
- **THEN** 系统 MUST 沿用 `ReviewerWindowStore::list_windows`（`src/product/reviewer_window_store.rs:1160`）的既有语义：跳过该条并留下告警痕迹，MUST NOT 使整次列举失败，MUST NOT 把该条计为悬空条目，MUST NOT 产生 REQ-DRG-07 定义的 `dangling_reference_target_missing` 告警（该条无可判定的引用目标 id）

#### Scenario: reviewer 窗口目录级失败仍上抛

- **WHEN** 列举 reviewer 窗口时窗口分区目录不可读
- **THEN** 系统 MUST 照旧上抛该 IO 错误，MUST NOT 静默返回部分结果

### Requirement: fail-closed 读路径不接入容错（REQ-DRG-06）

系统 MUST NOT 在任何 fail-closed 准入、反查或物理清理读路径上跳过悬空条目。窗口租约列举、reviewer 资源物理清理、实体身份台账按 owner 反查、实体 scope 准入与墓碑复核、intent 的版本化单条读 MUST 保持既有失败语义。

#### Scenario: 租约读取不得 fail-open

- **WHEN** 某条窗口租约引用的 attempt 已不存在
- **THEN** 系统 MUST 仍按既有语义读出该租约，MUST NOT 跳过它使其看起来无租约而放行写入

#### Scenario: 物理清理读取不得漏删

- **WHEN** 物理清理扫描 reviewer 资源时遇到一条记录读取失败
- **THEN** 系统 MUST 照旧报错，MUST NOT 跳过它导致残留漏删

#### Scenario: 按 owner 反查不得静默漏清

- **WHEN** 按 owner 反查会话归属时其中一条绑定指向已删实体
- **THEN** 系统 MUST 仍按既有语义返回该绑定所属的会话 id，MUST NOT 因跳过而使级联删除漏清

### Requirement: 告警形状、去重与上界（REQ-DRG-07）

悬空引用告警 MUST 使用单一稳定码 `dangling_reference_target_missing`，并 MUST 携带记录路径、记录类型、引用字段、引用目标类型、引用目标 id 与处置动作。同一「记录路径 + 引用字段 + 目标 id」的组合 MUST 只产生一条告警。单次列举的告警条数 MUST 设有上界。

#### Scenario: 告警字段完整

- **WHEN** 某条旁支条目因悬空被跳过
- **THEN** 产生的告警 MUST 含稳定码、记录路径、记录类型、引用字段、引用目标类型、引用目标 id 与处置动作

#### Scenario: 重复触碰只告警一次

- **WHEN** 同一条旁支记录被多次列举
- **THEN** 同一「记录路径 + 引用字段 + 目标 id」MUST 只产生一条告警

#### Scenario: 告警有上界

- **WHEN** 单次列举命中大量悬空条目
- **THEN** 告警条数 MUST 不超过既定上界，超出部分 MUST 被丢弃且 MUST NOT 影响列举结果

### Requirement: 巡检范围（REQ-DRG-08）

巡检 MUST 覆盖状态根下全部持久化记录：issue 级分区（`projects/<p>/issues/<i>/**`）、reviewer 窗口分区（`reviewer-windows/**`）、reviewer 快照分区（`reviewer-snapshots/**`）、实体身份台账分区（`state/entity-identity/**`）与逻辑代码库 scope 目录。

#### Scenario: 五个分区全部被覆盖

- **WHEN** 在一个同时含 issue 级记录、三类全局分区记录与逻辑代码库 scope 记录的状态根上执行巡检
- **THEN** 巡检 MUST 遍历全部五个分区，且清单 MUST 给出本次覆盖的记录类型清单

#### Scenario: 逻辑代码库 scope 两路都扫

- **WHEN** 状态根下同时存在新版逻辑代码库子树与旧版别名子树
- **THEN** 巡检 MUST 遍历两路子树，MUST NOT 只扫其一

### Requirement: 巡检判据与未归类（REQ-DRG-09）

巡检 MUST 按「记录类型 → 引用字段 → 目标实体类型」判据核对目标是否仍存在。记录类型不在判据表内时 MUST 归为未归类，MUST NOT 猜测其引用语义，也 MUST NOT 进入销账动作。

判据表 MUST 覆盖自动化 intent 记录中的**全部授权引用字段**（五个：`enrollment_id` / `plan_id` / `session_id` / `source.stories[].id` / `source.designs[].id`），以及自动化 enrollment 记录中的**全部引用字段**（顶层 `plan_id` / `session_id` / `source.*`、当前绑定代际 `binding_history.current.*` 的 plan / session / source、历史代 `binding_history.previous[].*` 的 source）。判据表 MUST NOT 只覆盖其中一部分（逐项清单见本 REQ 末尾）。

#### Scenario: 未归类只报不删

- **WHEN** 巡检遇到一条判据表未覆盖的记录类型
- **THEN** 该记录 MUST 出现在清单的未归类节，MUST NOT 出现在带删除建议的明细节

#### Scenario: 墓碑不判为悬空

- **WHEN** 巡检扫描到实体删除墓碑记录
- **THEN** 该记录 MUST NOT 被判为悬空引用，MUST NOT 出现在清单的悬空明细中

#### Scenario: intent 的授权引用被登记

- **WHEN** 巡检扫描自动化 intent 记录
- **THEN** 该记录的 `enrollment_id`、`plan_id`、`session_id`、`source.stories[].id`、`source.designs[].id` 五个授权引用字段 MUST 均被登记并被核对，MUST NOT 因未登记而漏检

#### Scenario: enrollment 的全部引用被登记

- **WHEN** 巡检扫描自动化 enrollment 记录
- **THEN** 该记录的顶层 `plan_id` / `session_id` / `source.*`、当前绑定代际 `binding_history.current.*` 的 plan / session / source 引用、以及历史代 `binding_history.previous[].*` 的 source 引用 MUST 均被登记并被核对，MUST NOT 因未登记而漏报

**附：判据表 MUST 登记的引用字段逐项清单**

| 记录类型 | 引用字段 |
|---|---|
| `automation_plan_intent` | `enrollment_id`、`plan_id`、`session_id`、`source.stories[].id`、`source.designs[].id` |
| `automation_generation_intent` | `enrollment_id`、`plan_id`、`session_id`、`source.stories[].id`、`source.designs[].id` |
| `automation_enrollment` | 顶层 `plan_id`、`session_id`、`source.stories[].id`、`source.designs[].id`；`binding_history.current.plan_id`、`binding_history.current.session_id`、`binding_history.current.source.stories[].id`、`binding_history.current.source.designs[].id`；`binding_history.previous[].source.stories[].id`、`binding_history.previous[].source.designs[].id` |

### Requirement: dry-run 阶段只报不改（REQ-DRG-10）

巡检 MUST 只具备读取能力，MUST NOT 具备任何删除动作。dry-run 执行前后，状态根下全部文件 MUST 逐文件保持不变。

#### Scenario: 巡检零副作用

- **WHEN** 对一个含悬空记录的状态根执行 dry-run 巡检
- **THEN** 巡检前后状态根内全部文件 MUST 逐文件保持不变，MUST NOT 有任何文件被删除或改写

#### Scenario: 巡检不持删除能力

- **WHEN** 检查巡检能力可用的操作
- **THEN** 它 MUST NOT 提供任何删除文件或删除目录的入口

### Requirement: 巡检清单字段格式与落点（REQ-DRG-11）

巡检 MUST 产出一份清单并落盘到**分析报告目录**（`cadence/analysis-docs/`，按运行日命名的分析报告文件）。清单 MUST 至少包含「汇总」与「明细」两节，明细每行 MUST 含记录路径、记录类型、引用字段、引用目标类型、引用目标 id、已删证据与建议动作。清单 MUST 附带本次巡检发现集合的摘要（**仅用于审计关联**，MUST NOT 作为删除凭据——REQ-DRG-22）。清单由**服务启动时的巡检自动产出**（REQ-DRG-13），MUST NOT 需要人工触发。

清单 MUST 另外分节列出「只报不删残留」与「不可判定」两类条目，二者 MUST NOT 与可删条目混列。「只报不删残留」节 MUST 收录全部**已判定为悬空但按规则不可删**的条目，含「证据不足 / 删除未完成」类（REQ-DRG-02 的两类区分）；「不可判定」节 MUST **只**收录读取失败导致未判定的条目。

#### Scenario: 明细三要素齐备

- **WHEN** 巡检发现一条悬空引用
- **THEN** 清单明细 MUST 同时含该记录的路径、悬空引用类型与目标 id、以及建议动作

#### Scenario: 清单附带发现集合摘要

- **WHEN** 巡检产出清单
- **THEN** 清单 MUST 含一份发现集合的规范化摘要，MUST 可被人用于核对两次巡检是否同一现场

#### Scenario: 清单由启动时的巡检自动产出

- **WHEN** 服务启动并完成本次巡检
- **THEN** 清单 MUST 落盘到分析报告目录，MUST NOT 需要任何人工触发

#### Scenario: 清单落盘位置固定

- **WHEN** 巡检产出清单
- **THEN** 清单 MUST 落盘到分析报告目录下按日期命名的分析报告文件

#### Scenario: 只报不删残留单列一节

- **WHEN** 巡检发现一条命中悬空但按规则不可删除的条目
- **THEN** 该条目 MUST 出现在只报不删残留节，MUST NOT 出现在带删除建议的明细节

#### Scenario: 证据不足归入只报不删残留

- **WHEN** 某条引用已被判定为悬空，但证据等级不足（墓碑进行中、墓碑缺完成时间、存在待人工处置的删除失败事实、无墓碑、上游无法定义或降级类型）
- **THEN** 该条目 MUST 出现在只报不删残留节并计入悬空条目数，MUST NOT 出现在不可判定节

#### Scenario: 不可判定单列一节

- **WHEN** 巡检因读取失败、或所需存在性数据源 / 作用域不可用（存在性判为 `Unknown`）而无法判定某条引用
- **THEN** 该条目 MUST 出现在不可判定节，MUST NOT 被计入悬空条目数，MUST NOT 进入摘要的发现集合（REQ-DRG-22）

### Requirement: 销账规则表（REQ-DRG-12）

系统 MUST 按记录类型执行销账：自动化 intent 在其引用的 story/design/plan 满足 L1 证据**且满足 REQ-DRG-20 的四道前置条件**时删除；reviewer 窗口、租约、违规记录与快照在满足 L2 条件时删除；实体身份台账条目在其引用的 work item plan 满足 L1 证据时删除。墓碑 MUST 永不删除；未归类 MUST 只报不删；session 或 enrollment 缺失 MUST NOT 构成删除依据。

reviewer 窗口簇的 L2 条件 MUST 由窗口记录自身可算的四条构成且 MUST 同时成立：① 终态；② 无租约持有；③ 目标在两路核对下均不存在；④ worktree 路径已不存在。任一条不成立 MUST 只报不删（细则见末尾）。

**删除粒度 MUST 按 attempt 聚合**：该 attempt 名下的**全部**窗口（含归档）都满足上述四条时，才可把该 attempt id 交给物理清理器；任一窗口不满足 → 该 attempt 名下的窗口 MUST **一个都不删**（细则见本 REQ 末尾）。

#### Scenario: intent 悬空即删

- **WHEN** 某 issue 的自动化 plan intent 引用了已删的 story_spec、证据为 L1 墓碑直证、**且满足 REQ-DRG-20 的四道前置条件**
- **THEN** 该 intent 记录 MUST 被删除，且 MUST NOT 产生删除失败

#### Scenario: intent 前置条件不满足则不删

- **WHEN** 某自动化 plan intent 引用了已删的 story_spec 且证据为 L1 墓碑直证，但 REQ-DRG-20 的四道前置条件中任一条不成立
- **THEN** 该 intent 记录 MUST 保持不变，MUST 只报不删

#### Scenario: intent 的 session 缺失不构成删除依据

- **WHEN** 某 issue 的自动化 intent 引用的 session 不存在但没有任何 L1 墓碑
- **THEN** 该 intent 记录 MUST 保持不变，MUST 只报不删

#### Scenario: 窗口簇整体删除

- **WHEN** 某 reviewer 窗口的四条 L2 条件全部成立
- **THEN** 该窗口记录及其租约、违规记录与快照基线 MUST 一并删除，MUST NOT 留下半删状态

#### Scenario: 窗口四条件缺一即不删

- **WHEN** 某 reviewer 窗口仍持有租约，或其窗口状态非终态，或其 worktree 仍存在
- **THEN** 该窗口及其关联记录 MUST 保持不变，MUST 只报不删

#### Scenario: 同一 attempt 下有窗口不满足即整 attempt 不删

- **WHEN** 同一 attempt id 名下有两个 reviewer 窗口，其中一个满足全部四条 L2 条件、另一个仍持有租约
- **THEN** 该 attempt 名下的**两个**窗口及其关联记录 MUST 全部保持不变，MUST 只报不删，MUST NOT 只删满足条件的那个

#### Scenario: 墓碑永不删除

- **WHEN** 销账在状态根下执行
- **THEN** 实体删除墓碑目录 MUST 逐文件保持不变

#### Scenario: 用户业务数据不删

- **WHEN** 销账在状态根下执行
- **THEN** story/design/plan 正文、版本历史与产物目录 MUST 逐文件保持不变

#### Scenario: 无墓碑的台账条目不删

- **WHEN** 某实体身份台账条目引用的 work item plan 已不存在但没有 L1 墓碑
- **THEN** 该台账条目 MUST 保持不变，MUST 只报不删

**附：窗口簇 L2 四条件与 attempt 聚合判定的细则**

| # | L2 条件 | 判定方式 | 归属 |
|---|---|---|---|
| ① | 窗口状态为终态 | 窗口状态枚举的终态判定 | 窗口记录自身 |
| ② | 该窗口无租约持有 | 对应租约记录不存在 | 窗口记录自身 |
| ③ | 窗口引用的目标在两路存在性核对下均不存在 | 路 1 coding attempt 未命中 **且** 路 2 workspace session 未命中（REQ-DRG-21） | 窗口记录自身 + 两路核对 |
| ④ | 窗口记录中的 worktree 路径已不存在 | 记录的 worktree 路径 `!exists()` | 窗口记录自身 |

- **attempt 聚合**：既有物理清理器按 attempt id 批量删除该 attempt 名下的全部窗口（含归档窗口）及其租约、违规记录与快照目录。故销账 MUST 先按 attempt 聚合该 attempt 名下**全部**窗口记录（含 `window-archive` 中的归档窗口），**全部满足**①–④才可把该 attempt id 交给物理清理器；只要该 attempt 下有**任一**窗口不满足（持租约 / 非终态 / 目标仍在 / worktree 仍存在），该 attempt 名下的窗口 MUST **一个都不删**，MUST 只报不删。

### Requirement: 闸 1：自动模式（启动时自动销账，无人工令牌）（REQ-DRG-13）

销账 MUST 在**服务启动时自动执行一次**：巡检后按「白名单类型 + 证据达标」自动销账，MUST NOT 保留人工确认令牌环节，MUST NOT 需要人工触发。删除条件 MUST NOT 比 REQ-DRG-12 更宽：白名单类型 + L1 墓碑证据（intent / 台账条目）或 W-1..W-4 全成立（窗口簇）才可删，其余一律只报不删。清单摘要仅用于审计关联，MUST NOT 作为删除凭据（REQ-DRG-22）。

#### Scenario: 启动时自动销账

- **WHEN** 服务启动并完成本次巡检
- **THEN** 系统 MUST 按白名单与证据门自动销账，MUST NOT 要求任何人工令牌或人工触发

#### Scenario: 无人工令牌环节

- **WHEN** 销账被执行
- **THEN** 系统 MUST NOT 要求任何确认令牌，MUST NOT 因缺少令牌而拒绝执行

#### Scenario: 删除条件不放宽

- **WHEN** 某条旁支记录不在销账白名单内，或其 L1 / W-1..W-4 证据不成立
- **THEN** 该记录 MUST 只报不删，MUST NOT 被自动销账删除

#### Scenario: 启动期失败不阻断服务

- **WHEN** 启动期巡检或销账失败
- **THEN** 系统 MUST 只记录失败，MUST NOT 阻断服务启动

### Requirement: 闸 2：白名单外一律不删（REQ-DRG-14）

销账 MUST 只作用于销账规则表明确列举的记录类型。规则表外的记录类型 MUST NOT 被删除，即使其引用目标已不存在。已知但被降级为只报不删的目标类型（上游无法定义、或目标缺失属正常中间态的类型）同样 MUST NOT 被删除。

#### Scenario: 表外类型不被删

- **WHEN** 状态根下存在一条判据表未覆盖、且其某引用目标已不存在的记录
- **THEN** 该记录 MUST 保持不变

#### Scenario: 规则表与判据表独立

- **WHEN** 某记录类型在巡检判据表内但不在销账规则表内
- **THEN** 该记录 MUST 出现在巡检清单中，MUST NOT 被销账删除

#### Scenario: 降级类型不被删

- **WHEN** 某记录引用了一类上游无法定义的目标类型，且该目标已不存在
- **THEN** 该记录 MUST 保持不变，MUST 只报不删

### Requirement: 闸 3：销账幂等（REQ-DRG-15）

销账的每一步 MUST 幂等：重复执行 MUST NOT 报错，MUST NOT 重复删除，MUST NOT 产生重复的删除事实。幂等语义 MUST 只用于**重入**（同一清理动作重复执行）；MUST NOT 被用来替代 REQ-DRG-20 的锁语义——删除 intent 记录仍 MUST 在自动化 enrollment 的文件锁内进行，「缺失视为已清」MUST NOT 免除该锁。

#### Scenario: 重复执行无副作用

- **WHEN** 连续执行两次销账
- **THEN** 第二次的删除数 MUST 为零，MUST NOT 报错，MUST NOT 产生重复的删除事实

#### Scenario: 目标已不存在视为已清

- **WHEN** 销账遇到一条记录已被先前执行删除
- **THEN** 该条 MUST 被视为已清，MUST NOT 报错

#### Scenario: 幂等不得免除锁语义

- **WHEN** 销账删除一个自动化 intent 记录，而该 intent 记录在此刻已不存在
- **THEN** 销账 MUST 仍按 REQ-DRG-20 在 enrollment 文件锁内完成判定，MUST NOT 以「缺失视为已清」为由在锁外跳过判定

### Requirement: 闸 3：每次清理留痕（REQ-DRG-16）

每次清理 MUST 留一行痕迹，含何时、因哪个实体被删、清了哪条三要素，并 MUST 记录本次删除所依据的证据等级。留痕 MUST 落在**状态根下单一全局的追加式 JSONL 台账**中（当前方案落点 `{state_root}/entity-deletions/dangling-reference/cleanup.jsonl`；该目录由本能力新建，具体落点归属由 owner 裁定）。留痕 MUST NOT 写入 issue 级删除墓碑目录，MUST NOT 扩展既有 `DeletionEntityKind` 枚举形状，MUST NOT 改写既有墓碑记录。留痕行 MUST 沿用既有根级追加式审计行的格式规范（每行带行格式版本字段）。同一清理动作重复执行 MUST NOT 追加重复行。

#### Scenario: 留痕三要素齐备

- **WHEN** 销账删除了一条旁支记录
- **THEN** 留痕 MUST 记录清理时间、被删实体、目标类型与目标 id、以及被清理记录的路径与类型

#### Scenario: 留痕落在状态根下的单一全局台账

- **WHEN** 检查留痕落点
- **THEN** 留痕 MUST 位于状态根下**单一**全局追加式 JSONL 台账中，MUST NOT 分散为每 issue 一份，MUST NOT 写入 issue 级删除墓碑目录，MUST NOT 改写既有墓碑记录或扩展既有删除实体种类枚举

#### Scenario: 留痕记录证据等级

- **WHEN** 销账删除了一条旁支记录
- **THEN** 留痕 MUST 记录本次删除所依据的证据等级

#### Scenario: 重复执行不追加重复行

- **WHEN** 连续执行两次销账
- **THEN** 留痕 MUST 不出现内容重复的行

### Requirement: 闸 3：清理后自证归零（REQ-DRG-17）

销账完成后 MUST 再执行一次巡检。归零计数 MUST **只统计销账白名单内且本次可删的类型**；MUST NOT 把「只报不删残留」或「不可判定」计入归零。存在性判为「无法判定」（`Unknown`，REQ-DRG-01 / 23）的条目 MUST 归「不可判定」，MUST NOT 计入悬空条目数，MUST NOT 计入归零计数。存在未归零的可删条目时，系统 MUST 逐条列出，MUST NOT 宣称悬空引用已治理完成。存在任一「不可判定」条目时，系统 MUST NOT 宣称归零。

#### Scenario: 归零计数逐类给出

- **WHEN** 销账完成后触发自证巡检
- **THEN** 自证结果 MUST 按记录类型给出悬空条目数，且已清理类型的计数 MUST 为零

#### Scenario: 未归零必须列出

- **WHEN** 自证巡检仍发现未归零的可删悬空条目
- **THEN** 系统 MUST 逐条列出这些条目，MUST NOT 宣称治理完成

#### Scenario: 只报不删残留不计入归零

- **WHEN** 自证巡检发现的悬空条目全部属于只报不删残留
- **THEN** 系统 MUST 把这些条目列入只报不删残留节，MUST NOT 以其为零宣称治理完成

#### Scenario: 存在不可判定时不得宣称归零

- **WHEN** 自证巡检存在不可判定条目
- **THEN** 系统 MUST NOT 宣称归零或治理完成，MUST 逐条列出不可判定条目

### Requirement: 承接在途 enrollment 语义（REQ-DRG-18）

自动化 enrollment 记录在 story/design 被删后 source 指向已删实体，属于在途变更覆盖的已知状态。巡检 MUST NOT 将其判为未归类；销账 MUST NOT 删除 enrollment 记录。系统 MUST NOT 重新裁定 enrollment 的重新授权语义。story/design 的悬空判定 MUST 只判存在性：实体仍在但版本或确认态漂移 MUST NOT 判为悬空。

「读取失败按未悬空处理」这条口径 MUST **只约束 enrollment 重新授权判定**，MUST NOT 被当作通用规则用于巡检或销账的证据判定。

#### Scenario: enrollment 不判未归类

- **WHEN** 巡检遇到一个 source 指向已删 story_spec 的自动化 enrollment 记录
- **THEN** 该记录 MUST 被识别为已知的、在途覆盖的状态，MUST NOT 归入未归类节

#### Scenario: enrollment 不被销账

- **WHEN** 销账在状态根下执行
- **THEN** 自动化 enrollment 记录 MUST 保持不变

#### Scenario: 不重裁重新授权语义

- **WHEN** 系统判定 enrollment 的 source 已悬空
- **THEN** 系统 MUST 只记录该事实，MUST NOT 决定如何重新授权，MUST NOT 改变既有精确校验的三条判定

#### Scenario: 悬空只判存在性

- **WHEN** enrollment 的 source 引用的 story 与 design 实体都仍存在，但版本或确认态已漂移
- **THEN** 系统 MUST NOT 判为悬空，MUST 维持既有的逐字段相等拒绝语义

#### Scenario: 重新授权判定的读取失败按未悬空处理

- **WHEN** 判定 enrollment 的 source 是否悬空时读取目标记录失败
- **THEN** 系统 MUST 按未悬空处理，MUST NOT 据此放宽任何拒绝语义

#### Scenario: 读取失败口径不得外推到巡检

- **WHEN** 巡检判定某条旁支记录的目标是否存在时读取失败
- **THEN** 系统 MUST NOT 把该条按未悬空处理，MUST 归入不可判定

#### Scenario: 只有 source 可被放行

- **WHEN** 重新开启时终止记录的 source 已悬空，但 options 或 target 与本次提交不同
- **THEN** 系统 MUST 拒绝该次重新开启，MUST NOT 因 source 悬空而放行 options 或 target 的漂移

### Requirement: 硬约束（REQ-DRG-19）

本能力 MUST NOT 改变既有删除语义与既有级联清理覆盖面，MUST NOT 删除墓碑，MUST NOT 删除用户业务数据，MUST NOT 新增依赖，生产目录操作 MUST 只用标准库文件接口。本能力 MUST NOT 改动 provider 适配层、工具策略、网关信封与流式输入结构，MUST NOT 改动已到行数上限的源码文件。本能力 MUST NOT 删除任何**不具备 L1 证据、且不满足证据等级表中该类型的 L2 专属可算条件**的旁支记录。

#### Scenario: 既有删除语义零回归

- **WHEN** 对比启用本能力前后的既有删除与级联清理行为
- **THEN** 既有删除准入、既有级联覆盖面与既有墓碑形状 MUST 完全一致

#### Scenario: 不新增依赖

- **WHEN** 检视依赖清单
- **THEN** 本能力 MUST NOT 引入任何新的第三方依赖

#### Scenario: provider 侧零改动

- **WHEN** 对比启用本能力前后的工具策略摘要、网关信封与流式输入结构
- **THEN** 三者 MUST 完全一致

#### Scenario: 行数上限文件零改动

- **WHEN** 检视本次改动的文件清单
- **THEN** 已达行数上限的源码文件 MUST NOT 出现在改动清单中

#### Scenario: 无证据的记录不被删除

- **WHEN** 某条旁支记录的目标已不存在，但该记录既不具备 L1 证据、其类型在证据等级表中又未被标为 L2（或已标为 L2 但其专属可算条件不成立）
- **THEN** 该记录 MUST 保持不变，MUST 只报不删

### Requirement: 意图销账的四道前置条件（REQ-DRG-20）

删除一个自动化 intent 记录 MUST 同时满足四条，缺任一条 MUST 只报不删（逐条判据见本 REQ 末尾）：

① **在自动化 enrollment 记录的文件锁内删除**，MUST NOT 与既有取代路径并发删除同一文件；
② **锁内重读三项**——enrollment 记录状态、generation intent 相位、目标墓碑证据——三者 MUST 在**同一把锁内**读取，MUST NOT 使用锁外读到的旧值；
③ **只认 L1 证据**，其 session 引用或授权引用缺失 MUST NOT 构成删除依据；
④ 自动化 generation intent 的相位 MUST 为「已认领且未派发」态，进入「引擎已启动」「已派发」「需人工」态 MUST NOT 删除。

MUST NOT 以「幂等」为由免除锁语义。

#### Scenario: enrollment 仍启用时不删

- **WHEN** 某 issue 的 enrollment 记录仍为启用态，且该 issue 的自动化 intent 引用了已删实体
- **THEN** 该 intent 记录 MUST 保持不变，MUST 只报不删

#### Scenario: enrollment 身份不匹配时不删

- **WHEN** 某 issue 的 enrollment 记录已终止，但其身份与 intent 记录中的授权身份不同
- **THEN** 该 intent 记录 MUST 保持不变，MUST 只报不删

#### Scenario: generation intent 已派发时不删

- **WHEN** 某自动化 generation intent 的相位为已派发态，且其引用的 plan 具备 L1 证据
- **THEN** 该 intent 记录 MUST 保持不变，MUST 只报不删

#### Scenario: 相位在锁内已推进则不删

- **WHEN** 锁外读取该 generation intent 时相位为「已认领且未派发」态，但进入 enrollment 文件锁后重读发现相位已推进为「已派发」态
- **THEN** 该 intent 记录 MUST 保持不变，MUST 只报不删（相位 MUST 以锁内重读值为准）

#### Scenario: 墓碑证据在锁内已变化则不删

- **WHEN** 锁外读取时目标 plan 具备 L1 墓碑直证，但进入 enrollment 文件锁后重读发现该墓碑已变为进行中态或已出现待人工处置的删除失败事实
- **THEN** 该 intent 记录 MUST 保持不变，MUST 只报不删（证据等级 MUST 以锁内重读值为准）

#### Scenario: 仅禁用未终止时不删

- **WHEN** 某 issue 的 enrollment 记录 `enabled == false` 但 `terminated_at` 为空，且该 issue 的自动化 intent 引用了已删实体
- **THEN** 该 intent 记录 MUST 保持不变，MUST 只报不删（「已终止」以 `terminated_at` 有值为准）

#### Scenario: 与取代路径并发不失败

- **WHEN** 销账删除 intent 与既有的自动化取代路径删除同一 intent 恰好并发
- **THEN** 取代路径 MUST NOT 因文件已被销账删除而报失败

#### Scenario: 前置条件齐备才删

- **WHEN** 某自动化 plan intent 的锁内重读判定通过、且其引用的 story 在锁内重读后仍具备 L1 证据
- **THEN** 该 intent 记录 MUST 被删除

**附：四道前置条件的逐条判据**

- **① 在 enrollment 文件锁内删除**：复用既有 enrollment 文件锁原语做临界区，MUST NOT 与既有取代路径并发删除同一文件。
- **② 锁内重读三项**：三项 MUST 在**同一把锁内**读取，MUST NOT 使用锁外读到的旧值。
  - enrollment 记录**不存在** → MAY 继续；
  - enrollment 记录**仍为启用态**（`enabled == true`）→ MUST NOT 删除；
  - enrollment 记录**已终止**（`terminated_at` **有值**）**且**其授权身份（`enrollment_id`）与 intent 一致 → MAY 继续；身份不一致 → MUST NOT 删除。
    > 「已终止」MUST 以 `terminated_at` 有值为准；仅 `enabled == false`（未写 `terminated_at`）**不等价于**已终止，MUST NOT 据此放行删除。
  - generation intent 的相位与目标墓碑证据同样以**锁内重读值**为准（外部先读到 `Claimed`、锁内已是 `ProviderDispatched` 的情形 MUST 判为不删）。
- **③ 只认 L1 证据**：L1 四条 MUST 在同一把锁内重读后仍成立；其 session 引用或授权引用缺失 MUST NOT 构成删除依据。
- **④ generation intent 相位限制**：相位 MUST 为「已认领且未派发」态；「引擎已启动」「已派发」「需人工」态 MUST NOT 删除。

### Requirement: attempt 目标的两路存在性核对（REQ-DRG-21）

对 reviewer 窗口与快照记录中的 attempt 引用，系统 MUST 执行**两路**存在性核对：路一为 coding attempt 记录，路二为规划面的 workspace session 记录。**两路都不存在**时，MUST 判定目标不存在；**任一路命中**时，MUST 判定目标仍在，MUST NOT 将该条目判为悬空。

该口径的理由：规划面的评审窗口把 workspace session id 写在该引用位上；只做单路核对会把活窗口误判为悬空并被物理删除。

**路二的数据来源 MUST 是「枚举」而非「按 id 单点查询」**：巡检 MUST 遍历状态根下**全部 project × issue** 调 workspace session 的**列举**接口建 id 集合，再以集合成员关系判定路二是否命中。巡检 MUST NOT 使用带写回副作用的单条会话读取接口。路二的枚举若因 IO / 解析 / 权限错误**无法完成**，MUST 记入**不可判定**（REQ-DRG-23），MUST NOT 把「未能枚举」当作「目标不存在」（细则见本 REQ 末尾）。

#### Scenario: coding attempt 命中即视为仍在

- **WHEN** 某 reviewer 窗口的 attempt 引用在 coding attempt 记录中命中
- **THEN** 系统 MUST 判定目标仍在，MUST NOT 把该窗口判为悬空

#### Scenario: 规划面 session 命中即视为仍在

- **WHEN** 某 reviewer 窗口的 attempt 引用只在 workspace session 记录中命中
- **THEN** 系统 MUST 判定目标仍在，MUST NOT 把该窗口判为悬空

#### Scenario: 两路皆不命中才算不存在

- **WHEN** 某 reviewer 窗口的 attempt 引用在两路核对下均未命中
- **THEN** 系统 MUST 判定目标不存在，该窗口 MAY 进入 L2 条件判定

#### Scenario: 路二枚举失败记不可判定

- **WHEN** 巡检枚举 workspace session 时某个 project/issue 分区不可读，无法建立完整的 session id 集合
- **THEN** 受影响条目 MUST 记入不可判定，MUST NOT 被判定为「目标不存在」，MUST NOT 计入悬空条目数

#### Scenario: 巡检不得产生写回副作用

- **WHEN** 巡检对某个 reviewer 窗口执行两路存在性核对
- **THEN** 系统 MUST NOT 调用会在读路径上持久化补写字段的单条会话读取接口，巡检前后状态根内全部文件 MUST 逐文件保持不变

**附：路二「枚举」的数据来源细则**

- workspace session 记录落在 **issue 级目录**，而窗口 / 快照记录**不携带** `project_id` / `issue_id`，故列举接口必须两者齐备 → 巡检 MUST **遍历全部 project × issue 组合**并在一次巡检内建一份 session id 集合。
- 巡检 MUST **禁用**带写回副作用的单条会话读取接口（其读路径会持久化补写字段，破坏零副作用契约）。
- 巡检 MUST **禁用**跨模块不可见的按 id 单点路径查找（其可见性仅限既有模块内部，且该文件已达行数上限）。
- 路二枚举若因 IO / 解析 / 权限错误**无法完成**，MUST 记入**不可判定**（REQ-DRG-23），MUST NOT 把「未能枚举」当作「目标不存在」。

### Requirement: 清单摘要的计算口径与审计用途（REQ-DRG-22）

清单摘要 MUST 等于本次巡检**发现集合的规范化序列化**的摘要。发现集合 = 本次巡检**判定为悬空**（目标已判定为不存在）的条目（含其中「只报不删残留」的条目）；**未归类条目与不可判定条目（含 `Unknown`）不属于发现集合**。规范化序列化 MUST：按记录路径升序排序（同记录路径再按引用字段与目标 id 升序）；每条只含记录路径、记录类型、引用字段、引用目标类型、引用目标 id、证据等级与建议动作七个字段；MUST NOT 包含巡检时间、令牌行自身、不可判定条目、未归类条目，或清单中的说明性文字（汇总栏、只报不删残留栏与不可判定栏的栏目标题与说明文字）。**「只报不删残留」条目本身的七个字段值 MUST 参与摘要**——只排除其所在栏目的说明文字，不排除条目本身。清单摘要**仅用于审计关联**（核对两次巡检是否同一现场），MUST NOT 再充当删除凭据（REQ-DRG-13）。

同一现场连续两次巡检的发现集合规范化序列化 MUST 逐字节一致。

#### Scenario: 摘要不随巡检时间变化

- **WHEN** 同一现场连续执行两次巡检
- **THEN** 两次的发现集合规范化序列化 MUST 逐字节一致，且两次的摘要 MUST 相同

#### Scenario: 清单正文不参与摘要

- **WHEN** 清单正文包含巡检时间等随运行变化的行
- **THEN** 这些行 MUST NOT 参与摘要计算

#### Scenario: 只报不删残留条目参与摘要

- **WHEN** 某条「只报不删残留」条目的字段值发生变化（例如其证据等级或建议动作改变），而其余内容不变
- **THEN** 摘要 MUST 随之改变；该条目所在栏目的说明文字 MUST NOT 参与摘要

#### Scenario: 摘要只用于审计关联

- **WHEN** 销账被执行
- **THEN** 系统 MUST NOT 以清单摘要作为删除凭据，MUST NOT 因摘要不匹配而拒绝或放行删除

#### Scenario: 现场变化后摘要随之改变

- **WHEN** 清单生成后现场状态发生变化，导致重算的摘要与清单上的摘要不一致
- **THEN** 摘要 MUST 随之改变，MUST 可被人据此判定两次巡检不是同一现场

### Requirement: 巡检的「不可判定」隔离（REQ-DRG-23）

巡检在某条引用的目标核对上**无法完成判定**时，MUST 记入**不可判定**计数：MUST NOT 计入悬空条目数，MUST NOT 计入归零，MUST NOT 判为未归类。清单 MUST 逐条列出条目与失败原因。存在不可判定条目时 MUST NOT 宣称治理完成。

「无法完成判定」覆盖两种情形：① **读取失败**（IO / 解析 / 权限）；② **所需存在性数据源或作用域不可用**（如缺 LC 上下文、缺记录所属 project/issue 作用域），此时存在性判为 `Unknown`（REQ-DRG-01），MUST NOT 退化为 `Missing`。证据不足（已判 `Missing`）归「证据不足 / 删除未完成」（REQ-DRG-02），MUST NOT 归入不可判定。

巡检自身扫到的坏记录或不可读分区同样 MUST 记入不可判定，MUST NOT 被静默跳过。

**边界**：本条约束**巡检面**（容错列举面的窗口跳过见 REQ-DRG-05）。

#### Scenario: 读失败不计入悬空

- **WHEN** 巡检因读取失败无法判定某条引用的目标是否存在
- **THEN** 该条 MUST 计入不可判定计数，MUST NOT 计入悬空条目数

#### Scenario: 读失败不计入归零

- **WHEN** 自证巡检存在不可判定条目
- **THEN** 该条目 MUST NOT 被计入归零断言，系统 MUST NOT 宣称治理完成

#### Scenario: 不可判定逐条列出

- **WHEN** 巡检存在不可判定条目
- **THEN** 清单 MUST 逐条列出这些条目及其失败原因

#### Scenario: 记录自身解析失败亦记不可判定

- **WHEN** 巡检扫描到一条记录，其自身 JSON 解析失败或所属分区目录不可读
- **THEN** 该条 MUST 记入不可判定并列入不可判定节，MUST NOT 被静默跳过，MUST NOT 计入悬空条目数

#### Scenario: 不可判定不与证据不足混列

- **WHEN** 某条目已被判定为悬空但证据等级不足
- **THEN** 该条目 MUST 归入只报不删残留节，MUST NOT 归入不可判定节

#### Scenario: 存在性数据源不可用时记不可判定

- **WHEN** 某条引用的目标类型所需的存在性数据源或作用域在本判定面不可用（例如判定面没有 LC 上下文却要核对 checkout / aggregate_index / logical_member，或没有记录所属的 project/issue 作用域却要核对 enrollment）
- **THEN** 该条 MUST 判为「无法判定」（`Unknown`）并计入不可判定计数，MUST NOT 计入悬空条目数，MUST NOT 被判为「不存在」，MUST NOT 计入归零

#### Scenario: 容错列举面的窗口跳过不算静默

- **WHEN** 同一条损坏的 reviewer 窗口记录分别被容错列举面（REQ-DRG-05 例外：沿用既有 `list_windows` 语义跳过并留下告警痕迹）与巡检面读到
- **THEN** 容错列举面 MUST 按既有语义跳过该条并留下告警痕迹，MUST NOT 使整次列举失败；巡检面 MUST 逐条读出该条并记入不可判定，MUST NOT 被静默跳过

### Requirement: ①层容错接入产品读路径（REQ-DRG-24）

产品代码读取旁支记录（自动化 intent / reviewer 窗口 / reviewer 快照 / 实体身份台账）并顺着 id 去查目标的路径，MUST 接入①层容错：目标**已确认不存在**时 MUST 跳过该条目并记一条①层告警（REQ-DRG-07），MUST NOT 使整次列举失败，MUST NOT 顺着 id 去查已不存在的目标。只跳过「**已确认目标不存在**」；读失败（IO / 解析 / 权限）照旧上抛，MUST NOT 被当成「目标已删」。主记录缺失语义（REQ-DRG-04）与 fail-closed 读路径（N5–N12）MUST NOT 改变。**已接入的读取点**：实体身份台账按 owner 反查（`plan.rs` 的 `entity_owned_session_ids`）——台账绑定指向**已删实体**（L1 墓碑四条全成立）时跳过该绑定，不把已删实体的 session 算进待清理集合。

#### Scenario: 孤儿窗口不得被跳过语义覆盖

- **WHEN** 启动补偿扫描枚举 reviewer 窗口，其中一条窗口的 attempt 记录已不存在
- **THEN** 该窗口 MUST 继续进入补偿收口（孤儿窗口 MUST 被释放，MUST NOT 永久停在非终态而使租约与窗口永不回收），MUST NOT 被「目标已删」的跳过语义丢弃；attempt 依赖的补偿子步骤 MUST 沿用既有的优雅 no-op（记日志、不抛错），MUST NOT 使整批评审失败

#### Scenario: 台账绑定指向已删 plan 时跳过该绑定

- **WHEN** 实体身份台账按 owner 反查，其中一条绑定指向的 plan 已确认删除（L1 墓碑四条全成立）
- **THEN** 该绑定 MUST 被跳过，MUST NOT 把已删实体的 session 算进待清理集合，MUST 产生一条告警

#### Scenario: 删除进行中的绑定不得跳过

- **WHEN** 台账绑定指向的 plan 墓碑仍处于删除进行中（或缺完成时间、带待处置失败事实）
- **THEN** 该绑定 MUST 保留，MUST NOT 被当成「已删」跳过

#### Scenario: 读失败不被当成目标已删

- **WHEN** 核对目标存在性时发生 IO / 解析 / 权限错误
- **THEN** 系统 MUST 照旧上抛该错误，MUST NOT 跳过条目，MUST NOT 产生悬空告警

#### Scenario: 主记录缺失语义与 fail-closed 路径不变

- **WHEN** 对比接入①层容错前后的主记录缺失错误码与文案、以及 fail-closed 读路径的失败语义
- **THEN** 两者 MUST 完全一致，MUST NOT 被降级为「跳过并告警」

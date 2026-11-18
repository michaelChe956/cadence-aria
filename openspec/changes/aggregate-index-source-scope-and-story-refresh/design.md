# 聚合索引源码范围判定与 Story 前共享索引刷新 · 设计

产品边界以 [proposal](proposal.md) 为准；可验收条款见 [specs/](specs/)；高层工作包见 [tasks](tasks.md)。本文件记录**已确认的架构边界与取舍**，路径与行号相对仓库根，行号以本变更的设计基线为准。

## D1 · 判定与跳过的数据流（谁在何时判、事实落在哪、谁消费）

**判定点唯一。** `AggregateIndexAcceptance::verify`（`src/product/logical_codebase/aggregate_index/operation.rs:48-156`）是**首次建库、手动重建、Story 前同步**三条入口共用的验收入口；修复必须落在这里，不能只改「重建索引」按钮的报错。

判定顺序（先覆盖、后盘检、再汇总）：

1. **成员覆盖**：SDK 逐成员 `hasFilesUnder` 为主路径（`operation.rs:57-95`）；SDK 不可用/不兼容才回退 CLI `files --json`（`operation.rs:107-140`）。CLI 的空索引提示 `codegraph_files_empty_index` 在验收层归一为「全员未覆盖 → 进入交叉验收」（`operation.rs:118-124`），**不**解释成截断、非法 JSON 或协议错误。
2. **有界盘检**：只对**未被覆盖**的成员，在其**主工作目录**内按 Aria 自有源码类型清单做一次有界盘检（口径见 D2），产出四值结论：`SourceInNormalPath` / `SourceOnlyInWorktrees` / `NoSource` / `Unknown`（权限、超时、超限、IO 失败一律 `Unknown`）。
3. **汇总**：
   - 至少一个成员被覆盖，且其余未覆盖成员**全部**为 `NoSource` ⇒ 通过其它验收后可发布 active，并把 `NoSource` 成员记为本世代的**跳过事实**；
   - 出现 `SourceInNormalPath`（漏收）/ `SourceOnlyInWorktrees` / `Unknown` / 全部成员 `NoSource` ⇒ **失败**，不发布新 active。

**事实落在哪。** 跳过事实是**成功世代**里成员快照上的一个可选标记（见 D4），不是另一张表、不是第二份 durable 成员列表、不是运行期缓存。同一逻辑代码库只有一份共享索引世代记录。

**谁消费。**

| 消费者 | 位置 | 消费方式 |
|---|---|---|
| 聚合索引卡片 | `src/web/handlers/aggregate_index.rs:23-50`（`AggregateIndexActiveResponse`） | 从**被投影的那一份可读验收世代**派生一行告警；不扫盘、不启 CodeGraph |
| 创建 Issue 选成员 | `src/web/handlers/logical_codebase_members.rs` | 同一 LC 的当前验收世代 → 逐仓标注；仅影响提示，不影响资格 |
| 跨仓证据链 | `src/product/logical_codebase/evidence_mediator.rs` | 目标为跳过成员 ⇒ 不发起 CodeGraph 查询，返回显式「未检索」（D6） |
| 规划指纹 | `src/product/logical_codebase/planning_context_resolver.rs:604-625`（`build_member_fingerprints`） | 跳过成员仍产出指纹（成员 ID / checkout / revision / dirty），`available` 取既有口径，不因跳过而变成不可用 |

**不因跳过而改变的东西**：项目成员集合（不移除成员）、成员授权与 ACL、选仓资格、checkout 与工作区准入、Issue 的 `selection.focus_repository_ids` 语义。

## D2 · Aria 源码口径：必须是 CodeGraph 支持后缀的**子集**（约束 + 验证方式）

**约束（硬）**：Aria 自有的源码类型清单必须满足「**由 Aria 自有**」且「**是 CodeGraph 支持后缀的真子集**」。理由：若清单纳入 CodeGraph 根本不索引的后缀，会把「CodeGraph 根本不索引的文件」误判成「索引漏收」并报错——这正是本变更要消灭的假阳性。

**具体清单与「哪些算配置」在实施时用真实 CodeGraph 逐后缀验证后确定**，本契约**不假定上游内部表**。验证方式（黑盒探针，实施时执行，产物只落 `/tmp`）：

- 以 Aria 清单的每个候选后缀建独立目录，各放一个该语言的最小有效样本；对每个目录用 CodeGraph **公共**能力（逐成员目录覆盖查询）校验「该后缀是否被收录」；
- 清单只保留探针证明**被收录**的后缀（子集约束），并对 `.yaml/.yml/.xml/.properties/.tf` 一类做反向对照，确认它们**不被**当作 Aria 源码（不要求上游一定不索引它们，只要求它们不进 Aria 口径）；
- 探针必须在部署支持的 CodeGraph 版本基线（现行兼容区间 `>=1.6.0, <2.0.0`，`src/product/logical_codebase/aggregate_index/codegraph_cli.rs:16-23,131-150`）上至少各跑一次；
- 清单、兼容基线或推荐版本变更前重跑探针；探针未跑通则**不宣称**兼容已验收。

旧稿曾引用本机安装包内的上游表（`lib/dist/extraction/grammars.js`）——**未证实（沿用旧稿勘察记录）**，本次不得作为产品运行依赖，也不得复制整表或生成 `extensions` 配置。

**判定口径细则**：

- 只按**后缀**做元数据判定，不读文件内容、不推断语言或符号、不看仓库类型或标签；后缀按最后一个扩展名做 ASCII 不区分大小写匹配。
- `.yaml/.yml/.xml/.properties/.tf/.tfvars/.tofu`、`.json/.md/.toml/.txt/.csv/.html/.htm` **不算**源码（属配置/文档）。是否产出符号不是判据（上游对 yaml/xml 属 file-level-only，不据此断言「无符号」）。
- 符号链接（目录与文件）与非普通文件**都不算**源码；目录打开不跟随符号链接。
- 排除口径**复用**既有 `exclude.rs:16-36` 的 11 条内建规则与 `codegraph.json` 生成口径（`exclude.rs:49-113`），**不另写副本**；普通遍历剪枝 `.git/.aria/node_modules/target/build/dist/.worktrees`。
- `.worktrees/` 是**独立的第二段诊断**：普通路径查完且无源码时才查 `.worktrees/` 子树；两段都无候选且都完成 ⇒ `NoSource`；在 `.worktrees/` 找到候选 ⇒ `SourceOnlyInWorktrees`（失败，不是跳过）。
- **不模拟 `.gitignore`**：普通路径有源码但被 ignore 仍是真漏收（提示查规则，不是「配置仓」）。
- **有界性**：根深度 0、上限深度 32；每成员最多访问 20,000 个目录条目、单次验收整批 100,000 条目、**2 秒**（含 worktree 诊断，共享同一计数与 deadline）；命中普通路径源码即停；不展开全量路径、不读内容、不起 git/CodeGraph 来扫描。超限/超时/权限不足 ⇒ `Unknown`。
- 扫描期间已证实普通路径有源码时，无需继续收集无关错误；**仅部分完成且无正证据时必须返回 `Unknown`**（不能把「没查完」当「没有源码」）。

## D3 · 覆盖验收四态与错误码取舍（尽量少增码）

**四态与出口**：

| 盘检结论 | 事实 | 出口 | 稳定码 |
|---|---|---|---|
| `SourceInNormalPath` | 普通路径有 Aria 源码，索引未收录 | 失败，不发布新 active | **复用** `aggregate_index_member_coverage_failed` |
| `SourceOnlyInWorktrees` | 源码只在被排除的 `.worktrees/` | 失败，不发布新 active | 新增 `aggregate_index_member_source_excluded` |
| `Unknown` | 权限/超时/超限/IO 失败，无法确认 | 失败，不发布新 active | 新增 `aggregate_index_source_scan_incomplete` |
| `NoSource` | 两段范围都确认无源码 | **跳过该成员**（非阻断告警）；若因此没有任何被覆盖成员 ⇒ 失败 | 新增告警码 `aggregate_index_member_source_less`；无被覆盖成员时用新增 `aggregate_index_all_members_source_less` |

**失败主码优先级**：普通路径漏收 > `.worktrees/` 独有源码 > 盘检无法确认；仅当不存在上述阻断且全部成员确认无源码时，才用全员无源码码。按 manifest 成员顺序点名同类全部成员；一条失败文案里不得把不同类型的成员都说成漏收。

**为什么这四个码不能复用既有码**（登记处：`non-interrupt-repository-bootstrap` 的 `REQ-BOOT-10`，唯一来源）：

- `aggregate_index_member_source_less`（**告警码，非 FailureCopy**）：既有码表里没有「已确认无源码、可继续」的语义。`aggregate_index_member_coverage_failed` 是**阻断**码且成品文案是「成员仓里没有可索引的代码文件 / 把真实源码放进成员仓」——用它表达「跳过、可以继续」既会阻断激活，又会把用户没做错的事说成用户错误。跳过是成功世代里的一条**告警**，须与失败封套隔离（`AggregateIndexActiveResponse.failure_copy` 与 `evidence_copy` 已按后端级互斥，`src/web/handlers/aggregate_index.rs:23-50,253-276`），不分配 `FailureTriage`。
- `aggregate_index_member_source_excluded`：既有码都会断言「索引里没有该成员的可索引文件」并给出「把源码放进成员仓」的动作，而本事实是**磁盘上有源码、只是位置在开发工作树**，动作是「把源码合入成员主 checkout 的普通路径」。复用会让用户去改忽略规则，而真正要做的是换源码位置。
- `aggregate_index_source_scan_incomplete`：本事实是**证据不足**。复用 `aggregate_index_member_coverage_failed` 会断言「没有源码」——正是本变更禁止的「无法确认当无源码」；复用通用 `aggregate_index_io` 会丢掉「不能确认」这一关键限定，且该码的处置动作（换机器/重试 IO）不等于「检查主检出可读性与扫描范围」。
- `aggregate_index_all_members_source_less`：与告警码 `aggregate_index_member_source_less` 的区别是**后果不同**——此处没有任何被覆盖成员，索引不可激活，必须阻断并给出「加入含源码的成员」的动作。同一码同时充当告警与阻断失败会破坏告警/失败互斥。

**漏收（`SourceInNormalPath`）为何复用 `aggregate_index_member_coverage_failed`**：既有码的机器事实就是「该成员在索引里没有任何可索引文件」，与漏收事实一致；码本身已登记、已被 Bootstrap 投影消费、已被 `failure_copy.rs` 的 `subject_from_message` 解析点名（`src/product/logical_codebase/failure_copy.rs:322-335` 抽 `included members [...]` 并用顿号连接成员名）。为不新增码，**同一码在唯一码表内登记两个成品变体**，按持久化失败事实里的机器事实前缀选择：

- 旧记录前缀 `no indexable code file under included members [...]` ⇒ 保留既有成品（只说「覆盖不足」，**不编造**磁盘有源码的证据）；
- 新事实前缀 `source found on disk; no indexed files under included members [...]` ⇒ 新成品（点名漏收成员、说明「磁盘上有源码但索引没收录」、指引检查成员路径与忽略规则后重建）。

两个变体都登记在 `failure_copy_table.inc.rs` 这**同一张表**里，不另建文案表；新事实**不得**渲染成旧成品（会把用户的忽略规则问题说成「没放源码」），旧记录**不得**渲染成新成品（会编造磁盘证据）。既有 `subject_from_message` 的点名解析对新旧两种前缀都成立（都以 `included members [` 结尾），不重做三段 UI。

**弃用备选**：新增 `aggregate_index_member_source_not_indexed` 单独承载漏收，可省掉「同码两变体」的选择机制，但多一个对外稳定码要登记与维护；本次按「尽量少增码」选**复用 + 同表变体**。

## D4 · durable 形状与向前兼容（无 `schema_version` ⇒ 只能靠 serde default）

**形状。** `AggregateIndexMemberSnapshot`（`src/product/logical_codebase/aggregate_index/types.rs:18-27`）新增一个可选字段：

```
skip_reason: Option<AggregateIndexMemberSkipReason>   // wire: "no_source"
```

序列化带 `#[serde(default, skip_serializing_if = "Option::is_none")]`；枚举当前只有一个 wire 值 `no_source`。

**语义**：`Some(no_source)` = 该成员经盘检**确认无源码**，本次未纳入代码检索范围；`None` = 无跳过事实（覆盖，或旧记录）。跳过成员**仍然出现在快照里**。

**`included` 语义不变。** `included` 保持既有原义（该成员出现在这次索引快照里），被跳过的成员**不改** `included`；跳过与否只由 `skip_reason` 表达。（早期草稿曾提议把跳过成员的 `included` 写成 `false`，**本变更作废该提议**：用户已锁定「不改 `included` 的既有语义」，且 `included` 还被 `planning_context_resolver.rs:621` 当作 `available` 使用，写 `false` 会连带把跳过成员变成不可用目标仓。）

**向前兼容。** 记录形状**没有 `schema_version`**（`types.rs:43-73`），因此兼容只能靠 serde 的缺省语义：

- 老记录（无 `skip_reason` 字段）**必须可读** ⇒ 字段必须 `#[serde(default)]`，缺省 `None`；
- 新记录写盘后老代码读会忽略未知字段（serde 默认行为），不需要回填历史、不需要迁移脚本、不引入第二份 durable 成员列表；
- 合法组合只有两种：`skip_reason = None`（覆盖）与 `skip_reason = Some(no_source)`（已确认无源码）。失败/未验收世代的默认快照、未知字段组合**不得**被投影为跳过证据。

**写入时点。** 首次采集、`Building` 与失败的命令前后快照照旧；**只有验收成功**才在同代快照上标记 `skip_reason`，随后 `replace_active` 并更新 manifest 指针；失败世代不写跳过事实，last-known-good 保持**它自己那一代**的跳过事实（本次失败的猜测不写进旧代）。

**漂移与巡检。** 跳过成员与其它成员一样参与 membership revision、成员集合、主 checkout 身份、revision、dirty 的漂移判据；**不**拿 `skip_reason` 与只采 Git 的当前快照直接比大小（否则恒 stale）。跳过成员后来出现新源码（含只在 `.worktrees/`）时，非降级世代的 freshness 评估应重新验收（见 D5）。引用巡检继续监督全部快照的成员/checkout 引用；存在跳过标志**不是**悬挂引用。

## D5 · Story 前更新：三分支与失败语义

**触发点**：用户点**生成或重新生成 Story 之后、AI 开始写之前**。复用既有规划前索引检查（`planning_context_resolver.rs:274-330`：`build_with_fresh_index` → `refresh_index_for_read`）；Story 生成入口已在该路径上（`src/web/handlers/lifecycle.rs:574` 与 `:673` 调用 `build_with_fresh_index`）。

**不触发的动作**：创建 Issue、勾选成员、刷新页面、同一次生成中的后续对话，都**不**另跑一次更新；**不新增后台定时轮询**（既有 `freshness.poll_due` 的既有兜底口径不变，本次不扩大）。

**三个分支**：

1. **无可用索引** ⇒ 走既有**首次建库**流程（登记与主工作目录已就绪时），不是每新建一个 Issue 就从头建库。
2. **能确认没变** ⇒ 复用当前这一版。判据（全部满足才算「能确认」）：成员集合未变、主工作目录（唯一 available Main checkout 的身份）未变、Git 提交未变，**且上次与本次都无未提交修改**。任一成员上次或本次 `dirty = true` ⇒ **不能确认没变**（工作区内容可能在两次检查之间被改过，revision 不变也说明不了文件没变）。
3. **有变化或不能确认没变** ⇒ 实际做**增量同步**（`codegraph sync` 语义）并**重新验收**（走 D1 的同一套判定）。上次更新失败留下的旧版**不能**直接当成最新版：先尝试恢复并验收。

**并发**：若初始化或另一次重建正在更新同一份索引，就**等待并复核结果**，不并行写两份（既有 single-writer 互斥，`operation.rs` 的构建写入路径）。

**失败语义**：更新或验收失败 ⇒ **停在本次生成之前**（AI 不启动），说明原因并允许重试；**保留已有 Issue、Story 与上次成功的索引**；**不得拿旧索引冒充最新**。「最新」指这次检查或同步时**本地成员主工作目录**里的代码，**不主动拉取远端**，也不保证生成过程中后来保存的改动已经收录。

**记录**：通过后记录这份 Story 参考的索引版本（沿用既有 planning snapshot 的 `aggregate_index_id` / `index_revision` 字段，`planning_context_resolver.rs:487-489`）。

**取舍论证（实测耗时）**：本机临时夹具（`/tmp` 隔离，**不是用户现场**）：69 个成员仓 / 1380 个源码文件 / 约 23 MB。

| 动作 | 耗时 |
|---|---|
| 首次建库（`init` + 全量验收） | **24.6s** |
| 未变检查（成员/主工作目录/提交/脏标记比对） | **0.24s** |
| 一仓改一文件后：检查 + 增量同步 + 重新验收 | **3.4s** |
| 强制 `codegraph sync`（命令本身） | **2.0–10.6s** |
| 已有库上走「重建索引」后端路径 | **1.8s** |

结论：**每次检查、变了才同步**。每次 Story 前必跑全量建库（24.6s 量级）不可接受；每次必跑增量同步（2.0–10.6s 量级，且 69 仓现场更慢）也不必要；而「未变检查」只要 0.24s 量级，可以每次都跑。**「上次与本次都显示有未提交修改」时必须同步**（3.4s 量级），因为 revision 相同不代表文件没变——这正是本次把「不能确认没变」并入第三分支的原因。数字来自临时夹具，**不得**当成用户现场的规模承诺；用户现场规模更大时按比例放大，但分支结构不变。

## D6 · 配置仓目标的证据链语义：200 + 显式「未检索」

在既有鉴权、ACL 与快照钉住有效的前提下，若钉住的可读验收世代确认目标成员主 checkout 属跳过成员：

- **不执行 CodeGraph 查询**（不建 query、不探测库目录、不调用 CLI）；HTTP **200**；
- 回包带显式 `not_searched: {code, message_zh}`，且 `text` 携带同一说明（例如「目标仓无源码，未检索聚合索引；请在目标工作区直接读取配置或文档。」），供纯文本消费者理解；
- `truncated = false`，配额照常核算；审计记 `hit_count = 0`、空 `snapshot_refs`，并留可读的「未检索原因」；
- **跳过 ≠ 查询无命中 ≠ ACL 放行**：不得把未检索冒充成「查过了、没有结果」；
- 对**普通目标**，既有 ACL 仍先成立；随后结果取「授权范围 ∩ 钉住世代已覆盖成员」的交集，**不返回跳过成员的索引证据**；
- 越界请求、钉住记录缺失或损坏仍按既有规则失败；
- provider 在该仓**已授权的目标工作区**直接读写配置/文档；**不加文件类型拦截、不要求把 JSON/文档纳入索引、不改 ACL/配额/审计口径**。

## D7 · UI 面与文案单一来源

**三个提示落点**（都只渲染服务端下发的成品文本，前端**不自写中文正文**）：

1. **聚合索引卡片一行**：在索引摘要下方显示「已跳过无源码成员：…」，点名被跳过的成员；不着失败色、不添新按钮、不改既有重建动作。数据来自**被投影的那一份可读验收世代**；无记录/空列表/旧世代 ⇒ 不显示。
2. **创建 Issue 选成员逐仓标注**：在该成员行标注「未纳入代码搜索，可修改配置或文档」。标注取自**同一逻辑代码库已验收的索引记录**（成员身份与主 checkout 都要对上）；**没有对应记录就不猜、不按仓名贴标签**。前端不额外拉聚合索引请求、不跨 LC 对表。
3. **入口目录说明**：三个入口（创建 / 登记 / 聚合初始化）共用既有组件 `web/src/components/lifecycle/LogicalCodebaseLayoutTree.tsx:1-48`，**改共用文案即可**；本次**不新建服务、不新增只读接口**（该组件当前自带结构说明文字，本次只调整第六条口径的一句）。规则六的**判定**仍归聚合索引契约，此处只陈述范围。

**选择与提交资格不变**：提示**不是**准入条件。有无提示、有无跳过标记，都不改变：成员是否列出、是否可勾选、是否可提交、是否可作 Issue 目标仓、checkout 与工作区准入、授权判定。

**文案单一来源**：`src/product/logical_codebase/failure_copy_table.inc.rs` 的码表是**唯一**中文成品来源。新增的告警成品（跳过提示、逐仓标注、未检索说明）以**同一个告警码 + 模板**登记一次，三处消费面各自渲染同一模板渲染出的文本（逐仓标注按单个成员渲染，卡片按成员列表渲染）。失败面仍走既有五字段合同（`title_zh` / `reason_zh` / `action_zh` / `triage` / `machine_line`，`REQ-BOOT-11`），告警**不**占用 `FailureCopy`、不分配 `FailureTriage`、不触发失败通知。**不新建第二份中文文案表、不在 Rust 投影 / 通知 / TSX 维护同义文案。**

## D8 · 取舍、残余风险与未证实项

- **判定放在验收层而非 UI 层**：UI 只消费成品，不复算「有没有源码」；避免前后端两套口径。
- **盘检是元数据扫描**：只按后缀、不读内容、不起 git/CodeGraph；代价是非清单语言在未被上游覆盖时可能被归为无源码——通过可见的成员告警、重建与有证据的清单维护纠正，不加手工覆盖。
- **超限宁可失败**：扫描超限要求修复可读性或缩小工作树，宁可保留 last-known-good，也不扩大预算临时放行；无法确认绝不等于无源码。
- **「上次与本次都脏」必须同步**：牺牲 3.4s 量级，换取 Story 引用的是当次工作区内容；这是本次把「不能确认没变」并入第三分支的代价。
- **不拉远端**：「最新」只指本地成员主工作目录；远端更新由用户自己同步后再生成为准。
- **残余风险（已知）**：CodeGraph 已覆盖但内容不足仍由既有验收边界负责；跳过成员的源码后来出现时靠 freshness 重新验收发现（存在时间窗）。
- **未证实（实施时必须验证，不得在契约里假定）**：
  1. Aria 源码类型清单的**具体后缀集合**（D2 的探针）；
  2. 本机 CodeGraph 安装包内上游扩展名表的准确位置与内容（旧稿勘察记录，**未证实**）；
  3. 全员无源码夹具下 `init/sync` 的真实输出形态；
  4. 配置目标的真实 provider 操作（工作区读写配置文件）未于本次运行验证；
  5. 上表耗时来自临时夹具（`/tmp` 隔离），**不是用户现场**。

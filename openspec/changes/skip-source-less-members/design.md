# 无源码成员跳过与告警：设计

产品边界以 [proposal](proposal.md) 为准。下列 D1–D8 为实施决策；路径均相对仓库，行号引用为本 change 的设计基线。下游 MUST/SHALL 见 [规格 delta](specs/)，文件切片见 [tasks](tasks.md)。

## D1 · Aria 源码口径（W1）

**采用独立、保守的正向清单。** 在新模块 `src/product/logical_codebase/aggregate_index/source_scan.rs` 维护唯一 `ARIA_SOURCE_EXTENSIONS`（下列含点表示后缀）。按最后一个扩展名做 ASCII 不区分大小写匹配；仅普通文件可作为证据，不读文件内容、不推断语言或符号，不看仓库类型或标签。

| 语言族 | 纳入的后缀 |
|---|---|
| JavaScript / TypeScript / Web 组件 | `.js .jsx .mjs .cjs .ts .tsx .mts .cts .vue .svelte .astro` |
| Python / Go / Rust | `.py .pyw .go .rs` |
| JVM | `.java .kt .kts .scala .sc` |
| C / C++ / Objective-C / CUDA | `.c .h .cc .cpp .cxx .hpp .hxx .m .mm .cu .cuh` |
| 其他常见语言 | `.cs .swift .rb .php .lua .dart` |

`.yaml/.yml/.xml/.properties/.tf/.tfvars/.tofu` **不算 Aria 磁盘侧源码**；`.json/.md/.toml/.txt/.csv/.html/.htm` 也不算。是否产出符号不是判据：上游 yaml/twig/properties 属 file-level-only；XML/Terraform 不据此一概断言无符号。清单是 CodeGraph 1.6.x 支持后缀的**真子集**，不是上游兼容表。`hasFilesUnder=true` 的成员仍视为已覆盖，不扫盘、不重新贴标签；例如 YAML 被上游收录时，该仓不触发本机制。全无源码的失败边界指**无已覆盖成员，且每个成员都经交叉验收确认无源码**，不是对已覆盖成员做第二套盘检。

依据：Aria 只生成 `exclude`（`src/product/logical_codebase/aggregate_index/exclude.rs:16-36,71-113`）；勘察本机 CodeGraph 1.6.3 包 `lib/dist/extraction/grammars.js:205-218,443-450` 与 `lib/dist/db/queries.js:2403-2407`，`EXTENSION_MAP` 非包根公开合同（`lib/dist/index.d.ts:19`）。现行兼容区间 `>=1.6.0,<2.0.0`（`src/product/logical_codebase/aggregate_index/codegraph_cli.rs:16-23,129-150`）不在本次改动范围。**禁止**依赖上游内部 import、复制全表、生成 `extensions` 配置、为配置仓设置 `includeIgnored`。清单变更、CodeGraph 兼容基线变更或升级推荐版本前，执行 D7 黑盒探针；不以 CI 读取上游源码作为产品运行依赖。

现行 `openspec/specs/logical-codebase-aggregate-index/spec.md:10` 的精确版本文字与上述运行时兼容区间是既有差异；delta 保留该版本条款，不借本 change 修改版本契约或运行门。子集探针以实际支持的 1.6.x 口径为基线。

## D2 · 路径、排除与扫描上限（W1）

- **一套成员路径**：从已通过登记/布局校验的 active 成员与唯一 available Main checkout 取 `(logical_repository_id, checkout_id, 根相对单层目录名, 规范化路径)`；沿用 `checkout_root_name` 的目录语义，与生成 `exclude` 的直接子目录约束对齐；禁止以 display name 或末段猜测扫描边界。不扫聚合根。CLI 回退的 `Path::starts_with(&Path)` 已按组件匹配（不是字符串裸前缀），保留并补测 `member-al` / `member-alpha`，同时要求路径确有子项。依据：`src/product/logical_codebase/aggregate_index/operation.rs:465-472,785-833`；`src/product/logical_codebase/aggregate_index/exclude.rs:171-186`。
- **共用排除定义**：将 `exclude.rs:16-28` 的 11 条内建规则提取为同一组目录/文件谓词与配置生成输入；不另写副本、不改变 `codegraph.json` 产物。普通遍历剪枝 `.git/.aria/node_modules/target/build/dist/.worktrees`；文件名匹配 `.env`、`.env.*`、`*credential*`、`*secret*` 的路径不算源码，命中敏感目录时整支剪枝。`vendor/.venv/__pycache__` 不在内建定义：不新增单边排除（如其中有候选源码而上游忽略，保守报漏收）。不模拟 `.gitignore`：普通路径源码被 ignore 仍是真漏收，提示检查规则而非标为配置仓。非成员和代码库边界继续由既有配置及负查询验收保证。
- **单独诊断 `.worktrees/`**：先遍历正常路径，找到候选即结束（报正常位置漏收）；记录正常遍历遇到的 `.worktrees/` 目录，只有普通路径完整检查且无候选才检查这些子树。第二阶段除允许进入待检查的 `.worktrees/` 外继续剪枝内建垃圾/敏感路径；在此找到候选才报「源码仅在被排除的 `.worktrees/`」。不扫描其他垃圾/敏感目录来证明源码存在；这些路径不进入 Aria 源码口径。没有普通路径和 worktree 候选、且两阶段均完成，才可判无源码。
- **整批限额**：根深度 0，上限深度 32；每成员最多访问 20,000 个目录条目、每次验收整批 100,000 个条目与 **2 秒**（含 worktree 诊断，共享同一计数和 deadline；包含枚举的目录/文件/链接/被剪枝项）。正常路径命中即停；队列与条目计数同界，不展开全量路径、不读内容、不起 git/CodeGraph 来扫描。以 LC 现有 single-writer 约束写侧，盘检用进程内最多两个并发的阻塞 worker 槽位、无无界等待队列；调用方至多等到共享 deadline，超时发停止信号，worker 在下一个可中断边界退出；卡在 OS I/O 的执行槽位在实际退出前不能复用。只访问经根锚定的子目录；目录打开不跟随 symlink，符号链接（目录和文件）与非普通文件均不计为源码。IO/权限/入口无法验证、遍历深度或条目超限、无槽位或超时均是证据不足，不允许当空目录。扫描期间已证实正常路径有源码时无需继续收集无关错误；仅部分检查且没有正证据时必须返回未完成。设计依据：现有文件遍历按内容哈希且过滤口径不同，不复用（`src/cross_cutting/cli_adapter.rs:576-602`）。

## D3 · 验收顺序、三态与失败映射（W1/W3）

保持 `AggregateIndexAcceptance::verify` 唯一入口（`src/product/logical_codebase/aggregate_index/operation.rs:48-156,465-560`）。在 init/sync 成功、采集并对比命令前后 Git 快照后，SDK 优先逐成员覆盖；仅 SDK 不可用/不兼容才回退；未覆盖才调用 D2。CLI `codegraph_files_empty_index` 与合法空清单在验收层都归一为全员未覆盖，**进入交叉验收**；不把截断、非法 JSON、SDK 缺成员/协议错误、CLI init/sync 或排除验收失败解释成空索引。`status` 健康异常仍仅告警，不替代覆盖证据（`src/product/logical_codebase/aggregate_index/codegraph_cli.rs:201-238`；`src/product/logical_codebase/aggregate_index/codegraph_coverage.rs:340-355`）。保留现有排除负查询；至少一个成员覆盖且其余均确认跳过，并通过其余验收，才可发布 active。代表性检索作为可用性验收，不引入运行时硬编码符号或「至少两个覆盖仓」门槛（`operation.rs:33-37,135-151`）。

| 交叉验收结果 | 稳定码与分诊 | 中文主文案语义 / 行动 |
|---|---|---|
| 无覆盖，正常路径与 `.worktrees/` 均确认无源码，且整代有覆盖成员 | `aggregate_index_member_source_less`，告警码，**没有 FailureTriage** | 「已跳过无源码成员：{成员列表}。」可继续；选仓行提示「该仓无源码，只能改配置/文档类文件」。 |
| 无覆盖，源码仅在 `.worktrees/` | `aggregate_index_member_source_excluded`，`Environment` | 问题「{成员}的源码仅位于工作树目录」；原因：`.worktrees/` 不在聚合索引范围；动作：将需索引的源码合入成员主 checkout 的普通目录，再重建索引。 |
| 无覆盖，普通路径有 Aria 源码（含 `.gitignore` 误伤） | 既有码 `aggregate_index_member_coverage_failed`，`ProductDefect` | 问题「索引漏了这个仓的源码」（点名仓）；原因：成员普通路径找到源码，但索引未覆盖，**不推断**是哪条规则或工具故障；动作：检查 `.gitignore` 与路径、重建索引，连续两次仍失败则报 issue 并附机器码。 |
| 没有覆盖成员，且全部无覆盖成员都经盘检确认无源码 | `aggregate_index_all_members_source_less`，`Environment` | 问题「所有成员仓都没有源码」（点名）；原因：聚合索引需要至少一个可被覆盖的源码成员；动作：加入包含源码的成员仓，或把实际源码放到成员主 checkout 普通路径，再重建。 |
| 未覆盖且盘检不完整 | `aggregate_index_source_scan_incomplete`，`Environment` | 问题「无法确认 {成员} 是否有源码」；原因：未读完、权限或扫描限额，并保留实际原因；动作：检查主 checkout 可读性与扫描范围，重建；不宣称仓无源码。 |

失败主码优先：普通路径漏收 > `.worktrees/` 独有源码 > 扫描不完整；仅没有任何上述阻断且全部确认无源码时使用全员无源码码。按 manifest 成员顺序点名同类全部成员，在审计明细保留其他已观测的阻断原因；不可在一条失败文案中把其他类型成员都说成漏收。空 manifest 不作「全员都无源码」的空集判定，若到达验收则以既有 `aggregate_index_member_invalid` 阻断。失败沿用 `fail_building`，无新 active；首建失败仍不 ready，刷新失败仍保留并降级 LKG。对部署前旧 `aggregate_index_member_coverage_failed` 记录只说「该成员未被索引覆盖」，不猜盘上有/无源码。

并行隔离：**以 `feat/aggregate-index-failure-face` 的接口提交 `00724807`（或含其等效改动的后继）为前置，再接码表、Bootstrap 投影和 HTTP 字段**。保留失败封套、已有 `failure_copy`/`details.copy`、七个对外布局码及 `included members [...]` 点名片段；新输入用可区分旧格式的机器事实（普通漏收采用固定前导 `source found on disk; no indexed files under included members [...]`，旧输入仍识别 `no indexable code file under included members [...]`），沿用已有点名解析，不重做三段 UI。对新失败仅登记同源子类、稳定 `reason_code` 和模板；新告警与现有 `warning`（诊断/失败记录）隔离。不以中文或自由文本推断新告警。依据：`src/product/logical_codebase/failure_copy_table.inc.rs:38-41,124-129`；`src/product/logical_codebase/failure_copy.rs:322-352,487-522`；`src/product/logical_codebase/bootstrap_projector.inc.rs:697-706`；`src/web/handlers/aggregate_index.rs:34-42,243-298,307-330`。

## D4 · Durable 与消费者语义（W2/W4）

- **快照复用**：`AggregateIndexMemberSnapshot` 新增 `#[serde(default, skip_serializing_if = "Option::is_none")] skip_reason: Option<AggregateIndexMemberSkipReason>`，枚举当前只定义 wire 值 `no_source`。保留 `included: bool` 为历史必填字段，不加会掩盖损坏记录的默认值：现有生产者一直写 `true`，历史兼容为 `true + None`。保持整份 `AggregateIndexRecord` schema，不加版本号、不回填历史、不引入第二份 durable 成员列表。成功世代 invariant：`true + None` = 覆盖，`false + Some(no_source)` = 已登记但未纳入索引；其他组合不可作为跳过证据。
- **写入时点**：首次采集、`Building` 和失败的原始前后快照照旧；`included` 原本恒 true 不是临时覆盖判据。成功验收后，以逻辑成员 ID + checkout ID 关联覆写同代命令前后快照的两个字段，再 `replace_active` 并更新 manifest 指针；仍保留 revision/dirty/indexed_at 等全量快照，不把临时缺覆盖写成 `false`。既有 `warning` 保持健康、CLI 回退及失败/LKG 诊断意义。LKG 保持原代的 skip 事实，不把本次失败的猜测写到旧代。依据：`src/product/logical_codebase/aggregate_index/types.rs:18-71`；`src/product/logical_codebase/aggregate_index/operation.rs:474-484,514-584,639-671`。
- **Planning**：保持 LC 级 `active_required`（无可读索引不回退单仓）。`build_member_fingerprints` 继续按 selection 读取全部成员快照、保留被跳过成员 ID/revision/dirty；`available` 仍表示在验收时可用的 checkout，不再等于 `included`。它目前进入 access fingerprint 哈希而非选仓校验；因此不把仅改变索引纳入状态伪造成 checkout 不可用。已验证覆盖和跳过成员均为 true；工作区/执行仍走原准入，不放宽 checkout/Git/预检。依据：`src/product/logical_codebase/planning_context_resolver.rs:453-455,482-489,592-625`；`src/product/logical_codebase/aggregate_index/store.rs:123-146`；`src/product/coding_attempt_store/target_snapshot.rs:43-90`；`src/product/logical_codebase/production_policy_resolvers.rs:79-159`。
- **Freshness**：跳过成员与其他成员一样参与 membership_revision、身份、checkout、revision、dirty 漂移；不拿已验收的 `included/skip_reason` 与当前只采 Git 的默认值比，不能因 `false != true` 恒 stale。现有 freshness `assess` 的非降级路径，在 Git 快照不变时仍对已跳过成员做一次 D2 定界盘检：发现候选（含仅 `.worktrees/`）或盘检未完成则为 stale，后续同步按 D3 重新验收，不在评估中改旧快照。干净提交、dirty 翻转走既有漂移路径；补扫覆盖「已 dirty 时新增未跟踪的源码」不翻转布尔的窄例。保持 degraded 短路；手动重建和每次实际同步均重新验收，可在新世代中撤销标签。卡片和成员列表的 GET 不调用 `assess` 或盘检。依据：`src/product/logical_codebase/aggregate_index/freshness.rs:109-159`。
- **巡检**：不筛掉 `included=false`，监督所有快照 `logical_repository_id/checkout_id` 引用；存在跳过标志不是 dangling，目标真的缺失仍按原规则报告。不新增巡检对象/字段。依据：`src/product/dangling_reference/patrol/lc_partition.rs:257-313`。

## D5 · API / UI 与配置仓工作流（W3/W4）

- 现有 `GET /api/projects/{project_id}/logical-codebases/{lc_id}/aggregate-indexes/active`（legacy 路径共用投影）新增可选 `source_less_members_warning`，形状 `{code, message_zh, members: [{logical_repository_id, name}]}`（code 为 `aggregate_index_member_source_less`）；来自**同一份被投影的可读验收世代**，按快照成员顺序，名称取同 LC 成员 alias，名称不可读时可回退到成员 ID，不用 ID 之外的字段匹配事实。无记录、空列表或旧世代则省略；未验收的 `Building/Failed/Stale` 候选不产告警，可读 LKG/过时世代如有此事实使用「上次可用索引」前缀区分。保留 `failure_copy`、`evidence_copy`、失败/重建按钮与状态机。`AggregateIndexCard` 仅消费 DTO，在索引摘要下显示一行，不着失败色、不附新按钮。依据：`src/web/handlers/aggregate_index.rs:23-65,152-240,272-297`；`web/src/components/lifecycle/AggregateIndexCard.tsx:92-124`。
- **选择后端 DTO 装饰**：在现有 `GET /api/projects/{project_id}/logical-codebases/{lc_id}/members`（legacy 路径共用投影）的列表 DTO 加可选 `source_less_warning: {code, message_zh}`；同一 authority LC/project 读取 manifest 指定的当前验收 `Active` 世代，membership revision 和该 active 成员 ID、唯一 available 主 checkout ID 需一致；历史降级或无匹配记录时省略。与卡片共用快照判据和告警文案源；成员列表是附加提示，无法读到适用记录时不妨碍已登记 active 成员列出和提交，记录诊断即可。不改 LC 详情或登记批次 DTO。前端不额外拉 active GET、不跨 LC 对表、不按名称/`included` 或字符串推导，只渲染服务端 `message_zh`。主/焦点仓验证、无 physical 映射时禁用、复选框与提交仍按原规则。依据：`src/web/handlers/logical_codebase_members.rs:13-79`；`web/src/api/logicalCodebaseMembers.ts:14-27`；`web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx:70-101,126-149,196-240`；`src/web/handlers/product_resources.rs:296-427,434-470`。
- **配置仓本地操作，不派发 CodeGraph 查询**：正常完成 attempt 鉴权、目标与 LC 成员 ACL、读取/钉住已发布世代；若该世代事实匹配目标逻辑成员 ID 与主 checkout ID，跨仓 evidence mediator 不建 query、不探测库目录、不调用 CLI。HTTP 200 保留 `text/truncated/index_stale/budget_remaining`，新增可选 `not_searched: {code, message_zh}`，`text` 也携带同一说明（「目标仓无源码，未检索聚合索引；请在目标工作区直接读取配置或文档。」），供现有纯文本消费者理解；`truncated=false`，保持配额核算，审计存 `hit_count=0`、空 `snapshot_refs` 和向后兼容的可选 `not_searched_reason=aggregate_index_member_source_less`。缺失/损坏 pin 或越界请求仍按原规则失败。针对普通目标，既有 ACL 仍需先成立，随后与钉住世代已覆盖的成员/主 checkout 对取交集、继续排除目标仓，不返回跳过成员的索引证据；后续 DB 同步也不改变该世代分类。配置文件由 provider 在已有目标 worktree 直接操作；不加文件类型拦截或配置搜索。依据：`src/product/logical_codebase/evidence_mediator.rs:96-104,125-220,905-955,999-1102`；`src/product/logical_codebase/evidence_audit.rs:34-48`；`src/product/logical_codebase/evidence_index/mod.rs:106-113`；`src/product/coding_attempt_store/target_snapshot.rs:43-90`。
- **入口范围说明**：共享组件 `LogicalCodebaseLayoutTree` 保留目录示意；其三处宿主继续复用，组件从仅返回静态成品 `{source_scope_zh}` 的只读 `GET /api/logical-codebase-layout-guidance` 获取第六条说明。该入口沿用 Web API 访问约束，无需先建 LC，不读任何仓库或初始化状态；请求失败保留示意和可操作的原表单，不在前端抄写备用规则。选择这个不带成员数据的小读取面，避免创建前依赖 active 投影，或借列表接口重复枚举全项目仓库。依据：`web/src/components/lifecycle/LogicalCodebaseLayoutTree.tsx:1-48`；`web/src/components/lifecycle/AddCodebaseDialog.tsx:90-97`；`web/src/components/lifecycle/LogicalCodebaseBootstrapCard.tsx:294`；`web/src/components/lifecycle/LogicalCodebaseRegistrationWizard.tsx:306`。

不采用「对话框再读 active projection 并在前端拼表」：它新增跨请求的 LC 切换竞态，无法用成员列表行上的 display name 或 physical ID 安全关联主 checkout。服务端 authority + 同一成员/checkout 对按 D5 统一关联，不把索引可用性变成 issue 创建前置。

## D6 · 契约归属与文案（W1–W4）

| 规格 | 替换/新增与场景 | 归属和接地 |
|---|---|---|
| `logical-codebase-aggregate-index` | IND-01 保留 SDK/CLI/健康/排除场景，加交叉矩阵与空索引；IND-04 收窄真实覆盖失败并补全员无源码/LKG；新增 IND-06 世代快照、freshness、巡检、配置仓检索语义 | 现行 `spec.md:9-97,106-150,204-217`；IND-03「所有成员仍有快照」及既有写锁/恢复契约不变 |
| `logical-codebase-layout-guidance` | LAYOUT-01 改第 (6) 条与覆盖异常场景；LAYOUT-02 入口示意加混合/全无边界；LAYOUT-04 区分三段失败和行内告警；LAYOUT-05 明确单表包含补充分支 | 现行 `spec.md:8-78`；登记 REQ-REG-09 仍负责布局，不改登记判定 |
| `non-interrupt-repository-bootstrap` | BOOT-10 保留核心七码并登记三个覆盖相关补充失败码、一个警告码，绑定同源；BOOT-11 保留五字段，补 legacy 新旧语义和警告不得冒充 FailureCopy 场景 | 现行 `spec.md:307-353`，不改依赖前置/ready 计算或 notification 机制 |
| `logical-codebase-issue-entry` | MRE-01 原文全部保留；补 active 成员可选，跳过提示与直接工作区操作；旧 DTO 无提示也可选 | `openspec/specs/logical-codebase-issue-entry/spec.md:8-35`。`lifecycle-workbench-pipeline-ux/spec.md:108-121,136-148` 的零 API 变更针对该工作台改造，不是其他业务永远冻结接口；本次不改布局、摘要条、可访问区域名、旧 testid 或 WebSocket |
| `per-repo-coding-execution` | COD-05 保留鉴权/配额/审计场景，补跳过目标不查和普通目标排除跳过仓 | `openspec/specs/per-repo-coding-execution/spec.md:151-156`；由聚合索引 IND-06 提供针对此世代的事实，不重复盘检或增加执行门 |

第六条及其失败子类、布局提示、卡片摘要、选仓标注与未检索原因都由 `failure_copy_table.inc.rs` 登记，使用 `failure_copy.rs` 事实适配：失败生成现有必填五字段；告警生成独立轻量 copy（技术码 + 展示字符串），无 `triage`、无 `machine_line` 和可执行失败动作。`aggregate_index_member_coverage_failed` 保留历史模板分支及码标识；不在 Rust 投影、通知或 TSX 维护同义文案。见 D3 的集成依赖。

## D7 · 验证策略（实施时执行）

1. **Rust 单测**：普通路径源码 + SDK false **必须失败**（含 `.gitignore` 路径）；CLI 空 notice + 有源码仍报原覆盖码、空 notice + 全无报全无源码码；仅 worktree、混合、全无、无权限、链接、各限额、同前缀目录、两路径同真值；保持负查询失败、CLI 截断/解析/回退、健康告警原边界。测试扫描限时用可注入时钟/取消信号，不靠真实 sleep。
2. **Durable/投影**：旧 JSON（`included:true` 无新字段）读取无警告；混合世代 round-trip，失败不发布新代，LKG 只读；漂移在跳过成员 dirty 保持时也可发现新源码；巡检真悬挂/非悬挂、Bootstrap/HTTP 知名失败三段同构，告警只从正确 LC/世代/成员发出。证据查询断言配置目标零次 CodeGraph 调用，正常目标的命中不从 skipped 目录返回。
3. **`it_web`**：同一 LC 源码仓 + 配置仓，索引 ready → 成员查询带提示 → 创建 Issue 只选配置仓 → planning 返回该目标的有效 checkout 指纹；其他 LC 不串告警。`tests/it_web.rs:120-125,137-138,168-169` 已挂载索引假 CLI、证据模块和生命周期模块。`tests/support/aggregate_index_fixture.rs:141-149` 的旧 `files` 为每个成员造覆盖，需新增混合夹具变体；用 `tests/it_web/web_lifecycle_api/part_02.rs:523-547,652-774` 的真实 planning 接缝，在同模块新增带 focus 的创建请求（现有显式 focus 请求示例在另一二进制 `tests/web_logical_codebase_entrypoints/issue_attribution.rs:587-720`）。对照用例令磁盘有正常源码而覆盖 false，验证无新 active、已有 LKG 降级。证据链单独用已挂载的 `tests/it_web/evidence_query/part_01.rs:88-120` 夹具验证 HTTP 与工作区授权边界。
4. **Vitest + 页面验收**：卡片 active/无告警、active/含跳过、degraded/LKG、failed、rebuilding 状态；选 LC 后见到配置仓标注并可勾选提交，切换 LC 后告警不串；旧 DTO 无告警不影响可选。构建/测试/页面验证不是本次文档提交的交付动作。
5. **兼容防漂移**：用生产口径以临时 LC 为 Aria 清单每个后缀建独立目录，放最小有效语言样本；在 `1.6.0` 与当前发行验证所选的最新受支持版本，用公开 `hasFilesUnder` 逐目录校验子集（无 SDK 按现有 CLI 回退限定小夹具）。另以正反对照固定 `.yaml/.xml/.tf` 不算 Aria 源码（不要求上游结果为 false）。清单/支持版本变更若没有证据或探针失败，不合入相应变化，先确认夹具并缩减/修订清单；部分新版本未跑过不宣称全体 1.x 有稳定后缀契约。

## D8 · 取舍、残余风险与验收缺口

- **备选**：只看 `hasFilesUnder` 会吞掉真实漏收；完整复刻上游会绑定内部表与 ignore 实现；手工标注改变用户操作与权威事实。选交叉验收，代价是未覆盖成员多一次有界元数据遍历和有限语言覆盖。
- **限制与兜底**：非清单语言在未被上游覆盖时可能被归为无源码；通过可见成员告警、重建和有证据的清单维护纠正，不加手工覆盖。扫描超限会要求修复可读性或缩小工作树，宁可保留 LKG，不扩大预算临时放行；函数无法抢占单次 OS 文件操作，调用方有 deadline、后台并发有上限。没有文件系统快照，命令前后 Git 快照和周期性重评不保证实时；保持原冻结世代与审计语义。CodeGraph 已覆盖但内容不足仍由既有验收边界负责。
- **未证实**：现有安装包的 YAML 文件实际入库行为、完整支持版本 × 扩展名黑盒矩阵、全无源码 fixture 的 `init/sync` 真实输出、配置目标的真实 provider 操作没有于本次运行。实施时 D7 必须观察实际输出；若已有明确的空索引 notice，按 D3 归一；未知命令/协议失败不软化。若 provider 对已授权配置目标有未识别的索引前置，应定点审查该前置是否把索引覆盖误当作 checkout 准入，不扩大 ACL 或更改本 change 的只读判定合同。

# 无源码成员跳过与告警 · 实施切片草案

**阶段**：供合约审阅及后续 writing-plans 细化，不是 apply 授权。本次只写文档，不执行下列工作包，不运行构建/测试。以下路径均相对仓库根，每片只可改列出的 ≤3 文件（包括测试）；后续创建具体 Plan 后，以真实用例先红后绿、显式列文件提交。测试运行产物只放 `/tmp`。冻结契约：[设计 D1–D8](design.md)；需求映射：[proposal W1–W4](proposal.md)。

## 1. 判定、快照与只读消费

- [ ] **T01 · 单源排除与有界扫描**（独立，约 3h；D1/D2、IND-01）。文件：`src/product/logical_codebase/aggregate_index/source_scan.rs`（新增，含测）、`src/product/logical_codebase/aggregate_index/exclude.rs`、`src/product/logical_codebase/aggregate_index/mod.rs`。验收：11 条原模式和生成配置不变；清单大小写、普通文件、剪枝、`.worktrees/` 二阶段、普通位置优先、ignored 源码、symlink、无法读、32 层 / 20,000 / 100,000 / 2s/两并发槽位逐项有断言；盘检不读内容、不调用 CLI，预算不足不是无源码。限时用注入时钟与阻塞位点，避免 sleep。
- [ ] **T02 · Durable 标记和 planning 消费解耦**（独立，约 1h；D4、IND-06）。文件：`src/product/logical_codebase/aggregate_index/types.rs`、`src/product/logical_codebase/planning_context_resolver.rs`。验收：现有构造器默认 `true + None`；旧字段缺 `skip_reason` 仍可读；`false + no_source` round-trip，非法组合不当作跳过；服务统一读取 `is_source_less()`。planning 对覆盖/跳过都产出成员 ID、revision、dirty 与 available checkout 指纹，仍要求可读 LC 索引，无单仓回退。不改 Issue 创建校验。
- [ ] **T03 · 共用覆盖闸门与世代发布**（T01、T02 后，约 3h；D3/D4、IND-01/04/06）。文件：`src/product/logical_codebase/aggregate_index/operation.rs`、`src/product/logical_codebase/aggregate_index/operation_tests.inc.rs`、`src/product/logical_codebase/aggregate_index/operation_sdk_coverage_tests.inc.rs`。验收：两入口归一；只扫描 false；CLI 空 notice 和 `[]` 进入交叉验收；非空协议失败与截断不软化。同目录边界、1 覆盖 + 1 确认跳过成功；普通漏收（含 `.gitignore`）、仅 worktree、扫描不足、全员无源码按主码优先级失败，无新 active；负查询回归不放松。成功才在同代前后快照标记跳过并发布；旧世代、Git 差异、SDK/健康警告、LKG 降级行为保持。失败输入保留 `included members [...]` 英文点名段。
- [ ] **T04 · Freshness 与巡检回归**（T03 后，约 2h；D4、IND-06）。文件：`src/product/logical_codebase/aggregate_index/freshness.rs`、`src/product/dangling_reference/patrol_tests.rs`。验收：不拿 `included` 比 Git 快照，跳过成员新提交/dirty 改变/checkout 变化仍 stale；dirty 不翻转时正常位置新源码、仅 worktree 源码以及未完成盘检均进入重验收；degraded 不自动探测重建。巡检 fixture 一有引用、一无引用，跳过标志不影响正确性；不需要改巡检实现（`patrol/lc_partition.rs:282-313`）。

## 2. 同源文案、后端投影与证据

- [ ] **T05 · 同一文案源与 Bootstrap 适配**（可与 T01/T02 并行，但须包含 `feat/aggregate-index-failure-face` 接口提交 `00724807` 或等效后继；约 2h；D3/D6、BOOT-10/11、LAYOUT-01/04/05）。文件：`src/product/logical_codebase/failure_copy_table.inc.rs`、`src/product/logical_codebase/failure_copy.rs`、`src/product/logical_codebase/bootstrap_projector.inc.rs`。验收：仍只有六条布局规则、七个既有失败码仍命中具体模板，三个补充失败码给全三段；兼容旧覆盖消息，新漏收点名并不怪用户没放源码；新码在 Bootstrap 正确投影，不用告警触发失败；只此一处登记提示成品（卡片、选仓、范围、未检索）。不改三段 DTO、通知机制和既有 HTTP 失败投影。
- [ ] **T06 · 聚合索引卡片数据**（T02/T03/T05 后，约 1.5h；D5、LAYOUT-04、BOOT-11）。文件：`src/web/handlers/aggregate_index.rs`、`tests/it_web/web_aggregate_index_api.rs`。验收：canonical/legacy 共用投影，`source_less_members_warning` 只从真实被展示的验收世代派生，稳定列出全部跳过成员；missing/building/失败候选/旧记录不自造告警；可读 degraded LKG 历史告警与该次失败三段可同时显示。GET 不触发盘检/索引，现有 `failure_copy`/`details.copy` 不破坏。
- [ ] **T07 · Issue 成员行提示数据**（T02/T03/T05 后，约 1.5h；D5、MRE-01）。文件：`src/web/handlers/logical_codebase_members.rs`（含测试）。验收：仅同 LC/current Active/membership revision/active member/唯一 available 主 checkout 全部匹配才给 `source_less_warning`；配置仓仍可选，旧/不可读索引、其他 LC、checkout 不匹配不误标；不改 `product_resources.rs`、详情/登记契约和扫描行为。
- [ ] **T08 · 既有证据流程内的显式未检索**（T02/T05 后，约 2.5h；D5、IND-06/COD-05）。文件：`src/product/logical_codebase/evidence_mediator.rs`、`src/product/logical_codebase/evidence_audit.rs`、`src/web/handlers/evidence_query.rs`。验收：合法配置目标使用原 pin 与 ACL，不调用 CodeGraph；显式 `not_searched` + `text`，保留计费和可读 audit；普通目标证据范围取授权及覆盖交集。因新字段调整成功/拒绝审计构造器，不改 401/403 等既有失败行为。
- [ ] **T09 · 证据边界集成回归**（T08 后，约 1.5h；D7、COD-05）。文件：`src/product/logical_codebase/evidence_mediator_tests.inc.rs`、`tests/it_web/evidence_query/part_02.rs`。验收：假 runner 调用计数为 0 证明没有启动索引进程；覆盖普通目标过滤、钉住代与最新代分类不同、禁用/越界仍拒绝、HTTP 200 shape 与旧审计 JSON 兼容。旧 fixture 模块已由 `tests/it_web.rs:124-125` 挂载，不加新 harness。
- [ ] **T10 · 目录树静态成品读取面**（T05 后，约 1h；D5、LAYOUT-02）。文件：`src/web/app.rs`、`src/web/handlers/mod.rs`、`src/web/handlers/codebases.rs`。验收：`GET /api/logical-codebase-layout-guidance` 使用同源范围 copy，无 LC 也有说明；沿用 API 访问约束；不访问磁盘、不创建状态、不改已有选仓或登记入口。

## 3. 前端与集成验收

- [ ] **T11 · 聚合索引卡片**（T06 后，约 1h；D5、LAYOUT-04）。文件：`web/src/api/types/aggregate-index.ts`、`web/src/components/lifecycle/AggregateIndexCard.tsx`、`web/src/components/lifecycle/AggregateIndexCard.test.tsx`。验收：只消费同源 `source_less_members_warning.message_zh`；一行提示、不添按钮、不复制三段模板；老 DTO 兼容，失败三段优先关系保留（`AggregateIndexCard.test.tsx:92-208`）。
- [ ] **T12 · 创建 Issue 成员提示**（T07 后，约 1h；D5、MRE-01）。文件：`web/src/api/types/logical-codebase-members.ts`、`web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx`、`web/src/components/lifecycle/CreateLifecycleIssueDialog.test.tsx`。验收：显示返回的「该仓无源码，只能改配置/文档类文件」；仅勾配置仓时发送原 `focus_repository_ids` 与 `repository_id`；无物理映射禁用行为保持，旧 DTO 无告警不影响选择，切换 LC 不串标注；不另发聚合索引请求。
- [ ] **T13 · 三处入口共用目录树说明**（T10 后，约 1h；D5、LAYOUT-02）。文件：`web/src/api/codebases.ts`、`web/src/components/lifecycle/LogicalCodebaseLayoutTree.tsx`、`web/src/components/lifecycle/LogicalCodebaseLayoutTree.test.tsx`。验收：消费同源 `source_scope_zh`，同时保留目录树、旧 testid；网络失败仍可操作原表单，无第二份中文副本。真实入口覆盖创建、登记、聚合初始化三处共享组件，不新增宿主重复逻辑。
- [ ] **T14 · 冷启动和选仓→规划 `it_web` 联测**（T03–T07 后，约 2h；D7、IND-04/06、MRE-01）。文件：`tests/support/aggregate_index_fixture.rs`、`tests/it_web/web_lifecycle_api/part_02.rs`、`src/product/logical_codebase/bootstrap_tests_aggregate_index.inc.rs`。验收：真实临时 Git 仓配生产聚合操作和可控 CodeGraph fixture，源码仓被覆盖，配置仓仅 `.json/.md/.toml` → 冷启动最终 ready → 现有成员 API 返回提示 → 仅选配置仓创建 Issue → 走实际 planning 路径捕获该仓指纹；同项目另一 LC 不串提示。对照「普通路径存在源码，覆盖 false」必失败，重建保留 LKG；全员无源码不 ready。保留此前测试对真正源码漏收的断言（`bootstrap_tests_aggregate_index.inc.rs:138-185`）。
- [ ] **T15 · 支持后缀契约探针**（T01 后，约 2h；D1/D7、IND-01）。文件：`src/product/logical_codebase/aggregate_index/source_scan.rs`（T01 后串行，复用清单、加入显式版本条件的兼容探针，不另抄扩展表）。验收：部署支持基线 `1.6.0` 和当前选用最新版本，各用实际临时样本逐后缀核查 CodeGraph 公共目录覆盖；子集验证不能以编造 fixture 结果代替，缺依赖注明未跑并阻止声称兼容已验收；`.yaml/.xml/.tf` 磁盘反例不要求上游不索引。本片不能改工具支持区间或新建默认安装作业。

## 4. 串并行和页面放行标准

T01 ∥ T02 ∥ T05（满足并行分支前置后）；T03 合入必须与 T02 的 planning 解耦一同交付，不允许对外出现写 `included=false` 却把配置目标变成不可用的中间状态。共享文件片串行：T01→T15；若并行失败面尚在修改 T05/T06/T11 文件，等待其提交并在该基线上接线，禁止一起写同文件。前端只加提示节点，不改工作台结构及表单权限。

| 实施时的页面/动作 | 应显示 | 按钮 / 阻断 |
|---|---|---|
| 索引 active、无跳过或旧 DTO | 既有摘要 | 既有重建 |
| 索引 active、有跳过 | 摘要下「已跳过无源码成员：…」一行 | 既有重建；不阻断 ready |
| 重建中 / 无 LKG 的失败 | 既有加载或同源失败三段 | 重建中仍禁用；失败沿用原重建/重试；不出现新跳过提示 |
| 有历史跳过的 LKG 降级 | 失败三段 + 「上次可用索引」历史告警 | 原重建，不伪造 active |
| 建 Issue 选中含配置仓的 LC | 该成员旁同源提示；切换 LC 后正确清除 | 原复选/提交；满足原准入可只选该仓提交 |
| 创建/登记/初始化入口 | 树 + 同一源码范围说明 | 原入口与操作，不加扫描按钮 |

实施验收用内置浏览器在开发端真实点击（不使用 Playwright）；配置仓目标以真实 provider 在自己的工作区读改配置文件，观察无 CodeGraph 查询，同时普通目标证据查询仍可用。以上为后续放行标准，不是本设计提交已执行的验证。

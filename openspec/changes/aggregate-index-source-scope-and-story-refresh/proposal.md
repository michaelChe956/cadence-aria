# Change: 聚合索引源码范围判定与 Story 前共享索引刷新

> 本变更取代早期草稿 `openspec/changes/skip-source-less-members/`（已删除；历史内容见 git `da20b7cdc9`）。范围以用户已批准的方案稿 `cadence/designs/2026-11-18_技术方案_聚合索引与Story生成_v2.0.md` 为唯一权威。

## Why

逻辑代码库（LC）可以由源码仓和配置仓共同组成。今天有三处把用户挡在门外：

1. **配置仓拖垮代码搜索。** 聚合索引验收把「CodeGraph 没收录这个成员」直接判成整次失败（`src/product/logical_codebase/aggregate_index/operation.rs:838-852` 的 `require_member_coverage`；失败码 `aggregate_index_member_coverage_failed`，成品文案见 `src/product/logical_codebase/failure_copy_table.inc.rs:123-129`）。但「索引里没找到」推不出「磁盘上没源码」——只有配置和文档的成员仓会让整个 LC 建不出索引，冷启动无法进入 `planning_ready`（`src/product/logical_codebase/bootstrap_projector.inc.rs:49-52`；`src/product/logical_codebase/planning_context_resolver.rs:453-455` 读不到可读 active 索引即 fail-closed，不回退单仓）。
2. **Story 生成前不更新共享索引。** 规划前的索引检查只做 freshness 评估（`src/product/logical_codebase/planning_context_resolver.rs:274-330` 的 `build_with_fresh_index` / `refresh_index_for_read`），而漂移判据只比较成员集合长度、Git revision 与 dirty 布尔（`src/product/logical_codebase/aggregate_index/freshness.rs:180-191`）。成员工作区存在未提交修改时，索引内容与磁盘可以不一致，Story 仍按旧索引写；主工作目录换人（checkout 身份变化）也不在漂移判据里。
3. **改配置没有合法路径。** 配置仓被排除在代码检索之外后，没有一条既有链路允许 provider 在该仓**获准工作区**里直接读写配置/文档；跨仓证据链对这类目标只能返回「查不到」，用户要的却是「把超时从 30 秒改成 60 秒」这种真实改动。

本次修正这三件事，不引入新服务、不扩大 CodeGraph 收录范围、不新增后台轮询。

## What Changes

- **W1 · 配置仓不拖垮代码搜索（用户行为①）。** 把「成员未被索引收录」与「该成员没有源码」分开。验收阶段对**未被收录**的成员，在其**主工作目录**内按 Aria 自有源码类型清单做一次**有界盘检**；盘检确认无源码 ⇒ **跳过该成员**（只跳过代码检索，**不移除项目成员**，该成员仍可作 Issue 目标仓）；普通目录明明有源码却漏收 / 源码只在被排除的 `.worktrees/` / 盘检因权限·超时·超限**无法确认** / **所有成员都确认无源码** ⇒ 都不发布新索引。**无法确认不算无源码。**
- **W2 · 每次开始生成 Story 前先更新共享索引（用户行为②）。** 触发点在「用户点生成或重新生成 Story 之后、AI 开始写之前」，复用既有规划前索引检查（`src/product/logical_codebase/planning_context_resolver.rs:274-330`）。三种分支：无可用索引 ⇒ 走既有首次建库流程；能确认没变（成员、主工作目录、Git 提交均未变，且**上次与本次都无未提交修改**）⇒ 复用这一版；有变化**或不能确认没变**（含「上次与本次都显示有未提交修改」）⇒ 实际做增量同步并重新验收。更新或验收失败 ⇒ **停在 AI 启动之前**并说明原因，保留已有 Issue、Story 与上次成功的索引，**不得拿旧索引冒充最新**。同一逻辑代码库共享一份索引，不为每个 Issue 另建；创建 Issue、勾选成员**不**触发更新；**不新增后台定时轮询**。
- **W3 · 改配置直接改文件、不靠 CodeGraph（用户行为③）。** provider 直接在该仓获准工作区读写配置/文档，**不为该目标派发 CodeGraph 查询**；不要求把 JSON/文档纳入索引；不改 ACL、配额与审计口径。
- **W4 · 提示落点。** 聚合索引卡片一行「已跳过无源码成员：…」；创建 Issue 选成员时逐仓标注「未纳入代码搜索，可修改配置或文档」；**选择与提交资格不变**；提示取自**已验收的索引记录**，不按仓名猜。
- **W5 · 失败码与文案落点。** 新增事实只在 `non-interrupt-repository-bootstrap` 的 `REQ-BOOT-10` 唯一登记处登记（尽量少增码，逐条说明为何不能复用既有码）；中文成品单一来源是 `src/product/logical_codebase/failure_copy_table.inc.rs` 的码表，前端只渲染服务端下发的成品文本。

**明确不做**：不手工标注「配置仓」；不为一行说明新建服务或新增只读接口（三个入口共用既有目录说明组件 `web/src/components/lifecycle/LogicalCodebaseLayoutTree.tsx:1-48`，改共用文案即可）；不扩大 CodeGraph 的文件收录范围；不改成员快照 `included` 的既有语义（被跳过的成员**也在**快照里）；不把「配置仓」变成需要额外授权的角色；不新增后台定时扫描。

## Capabilities

### New Capabilities

无。调整现有索引、规划刷新、布局引导、失败面登记、Issue 创建与每仓执行契约。

### Modified Capabilities

- `logical-codebase-aggregate-index`：逐成员交叉验收（四态）、跳过事实的世代记录、失败不发布新 active、世代记录与向前兼容（W1/W2）。
- `logical-codebase-aggregate-planning`：Story 生成前的共享索引刷新三分支与失败停机语义（W2）。
- `logical-codebase-layout-guidance`：第六条（成员无源码不再阻断）、入口说明、单一来源映射（W1/W4）。
- `non-interrupt-repository-bootstrap`：新增稳定码与告警码的唯一登记处、五字段合同与告警的边界（W1/W4）。
- `logical-codebase-issue-entry`：成员行逐仓标注、选择与提交资格不变、提示取自已验收记录（W4）。
- `per-repo-coding-execution`：配置目标的「未检索」证据语义（200 + 显式未检索，而非查询无命中）（W3）。

## 非目标

不改仓库登记、目录布局与授权准入；不手工标记「配置仓」；不让全员无源码的 LC 通过；不修改 CodeGraph 支持版本区间、扩展名机制、ignore 或索引范围机制；不增设配置搜索、文件类型写入拦截或单仓规划回退；不为 Story 前更新新增后台定时任务；不为入口说明新增服务端点；不重做 `feat/aggregate-index-failure-face` 已完成的三段中文与失败面/证据面互斥工作。

本次只写契约：改 `openspec/changes/<新id>/` 下的文档，删除被取代的早期草稿目录，**不改实现代码、不改现行规格正文（`openspec/specs/`）、不跑构建与测试**。精确文件、命令与提交步骤由后续 `writing-plans` 写入 `cadence/plans/`。

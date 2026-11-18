# Change: 无源码成员跳过与告警

## Why

逻辑代码库（LC）可以由源码仓和配置仓共同组成。配置仓今天已可登记并选为 Issue 目标仓，但聚合索引验收把「未被 CodeGraph 覆盖」一律视为失败，导致冷启动无法进入 `planning_ready`，影响整个 LC，而非只影响该配置仓（`src/product/logical_codebase/aggregate_index/operation.rs:92-145,587-656`；`src/product/logical_codebase/planning_context_resolver.rs:454`；`src/product/logical_codebase/bootstrap_projector.inc.rs:49-52`）。应把正常的无源码成员和真实漏收区分开，同时保住覆盖验收的阻断边界。

## What Changes

- **W1 · 交叉验收**：继续用索引逐成员确认覆盖；只对未覆盖成员按 Aria 自有口径做有界磁盘检查。确认无源码才可跳过；普通路径有源码、源码仅在 `.worktrees/`、扫描未得出结论以及全员无源码均不发布新 active。保留排除验收、SDK/CLI 回退纪律与 last-known-good。
- **W2 · 按世代记录**：保留所有已登记成员的快照，在验收成功的世代记录哪些成员未纳入索引及原因。历史记录不重写；跳过成员仍参与规划成员范围、checkout 快照漂移和引用巡检。
- **W3 · 一处文案、两处告警**：聚合索引卡片展示一行成员告警，创建 Issue 时逐仓标注，勾选仍可用。布局引导讲清混合 LC 和全员无源码的分界；错误仍用同源三段式失败面，告警不伪造失败对象。
- **W4 · 配置仓可执行**：不变更 Issue 创建和成员授权校验。在已有可用 LC 索引的前提下，选择配置仓不导致规划 fail-closed；跳过成员不参与 CodeGraph 证据检索。Provider 仍在该仓正常工作区直接读写配置/文档。

## Capabilities

### New Capabilities

无。调整现有索引、布局引导、冷启动失败面、Issue 创建与每仓执行契约。

### Modified Capabilities

- `logical-codebase-aggregate-index`：逐成员交叉验收、世代快照、刷新与失败/降级边界（W1/W2/W4）。
- `logical-codebase-layout-guidance`：第六条范围说明、入口示意、三段失败面与单源映射（W1/W3）。
- `non-interrupt-repository-bootstrap`：保持七个既有稳定码及五字段合同，登记覆盖失败子类和非阻断告警的同一文案来源（W3）。
- `logical-codebase-issue-entry`：成员行可提示无源码，不改变 active 成员的选中、提交和授权（W3/W4）。
- `per-repo-coding-execution`：配置目标不发起 CodeGraph 证据查询，保持工作区与原 ACL/配额/审计约束（W4）。

## 非目标

不改仓库登记、目录布局和授权准入；不手工标记「配置仓」；不让全员无源码的 LC 通过；不修改 CodeGraph 支持版本、扩展名、ignore 或索引范围机制；不增设配置搜索、文件类型写入拦截或单仓规划回退。不重做 `feat/aggregate-index-failure-face` 的三段中文；本次仅提交设计、规格 delta 与实施切片草案，不改实现或现行规格，不运行构建/测试。

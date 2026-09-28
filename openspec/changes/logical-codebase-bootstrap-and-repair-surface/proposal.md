# Proposal

## Why

逻辑代码库（LC）当前存在两套路径解析与初始化前提：旧项目级布局可能与 LC 子树不一致，聚合索引首建又与成员准入互相依赖；身份迁移失败时普通仓库列表无法访问，成员规则缺失则在真正运行时才失败。结果是全新多仓必须靠手工铺底、删除 journal 或直接改数据才能继续，无法形成可重入、可诊断、有人到点处理后自动续进的产品链。现在按方案 v1.2 C4 将这些缺口收敛为唯一 authority 路由、分步冷启动和 repair 操作面，以满足 A03/A04 的零手工铺底验收。

## What Changes

- 建立 LC 与单仓并列解析的唯一 `RepositoryRouting` resolver：明确 authority、禁止多套猜路径和静默 fallback；旧布局或目标冲突进入 fail-closed 的迁移/核验等待。
- 将 LC 首次创建拆为可重入的身份、manifest/checkout、规则/策略、成员索引、聚合索引 active 步骤；每步持久化 checkpoint/失败事实，GET 只投影状态，不触发 provider 或其他副作用。
- 消除 aggregate index 首建的成员索引鸡生蛋：没有 active index 时按缺失步骤准备并继续，成员材料完成后再生成聚合索引；失败通过产品操作停等，成功后自动推进 PlanningReady。
- 为 Failed identity journal 提供不依赖普通成员列表的诊断与 repair 入口：展示 source digest、已完成键和冲突映射；安全前缀可继续，冲突必须由用户提交 mapping 并重新核验，不能删 journal、改权威 JSON 或未确认切读。
- 在准入前预检实际 provider 将消费的规则与 capability 材料；缺失时通知并提供可重入准备/重试操作，禁止用伪造 capability 记录、项目级旧路径或绕过 gateway 放行。
- 为各等待点提供可审计通知和稳定命令键；同一操作重放返回同一结果，版本/对象不匹配 fail-closed，操作成功后复用既有 registration/index/编排链继续。

## Capabilities

### New Capabilities

无。本 change 只修改既有 LC 能力的行为契约，不新增独立 capability。

### Modified Capabilities

- `logical-codebase-registration`: 增加唯一 authority resolver、冷启动 checkpoint、identity journal 诊断/repair、规则与 capability 预检及可操作停等语义。
- `logical-codebase-aggregate-index`: 调整首建准入顺序以解除成员索引与 aggregate active 的鸡生蛋，明确缺失/失败状态和只读投影边界。
- `codebase-kinds`: 明确单仓与 LC 的路由归一和目标冲突 fail-closed 行为，避免跨 kind 猜路径。
- `project-rule-aware-prompts`: 增加规则材料缺失的准入预检与产品化准备入口，保持有效聚合政策加载后才允许逻辑代码库 prompt 继续。
- `session-policy-envelope`: 补充 LC authority/policy 引用必须来自唯一 resolver，材料缺失或 digest 不一致时在 provider 启动前停等。

## Non-Goals

- 不建设新的通用 durable operation 平台、全局 owner/fence/epoch 或第二套 durable 状态机。
- 不在 GET/只读投影中偷偷启动 provider、索引、checkout 或任何外部副作用。
- 不用手工复制规则/selection/manifest/policy/index、直接编辑 JSON 或删除 journal 绕过失败；不自动吞并 identity mapping 冲突。
- 不在本 change 中实现单仓自动化入口、完整 plan/coding 生命周期或 Pi 初始化 recipe；单仓入口由 C5 负责，Pi recipe 按方案 v1.2 延期。
- 不放宽 LC gateway、单 target 约束或现有真实 provider 能力边界，不把 capability 记录当作实际能力证明。

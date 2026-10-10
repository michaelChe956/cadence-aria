# Proposal

## Why

LC 会话的 provider capability 门已按证据 fail-closed（launch/write_boundary/resume 非 Confirmed 即拒），但用户没有任何产品内通道把 Unknown 补成 Confirmed：真实边界探针（`run_cli_boundary_probe`）与 durable 导入（`record_verified_probe`）只存在于矩阵 harness，页面等待面上的 `Revalidate` 动作落到 bootstrap dispatch 后没有 capability 分支，等于空壳按钮。页面 E2E S4 已实证该墙（`provider_capability_launch_not_confirmed: PlanningReadOnly=Unknown`，capabilities 全 Unknown、provider_start_ledger=0）。需要一个用户可达的一次性真实验证通道：点 Revalidate → 真探针 → Confirmed，让有真实 CLI 的现场可以自举通过能力门。

## What Changes

- 新增 per-LC、per-provider 的 capability 重验证 HTTP 面：
  - `POST /logical-codebases/{lc_id}/capability-revalidate`（body：`provider_type`）：对该 provider 执行一次真实现场探针（复用 `run_cli_boundary_probe`，覆盖 Coding/Planning/Review 三个 action，含 resume 面），逐 action 经 `record_verified_probe` 原子导入 durable Confirmed（与 gateway 消费的 capability store 同一 LC 作用域）。
  - `GET /logical-codebases/{lc_id}/capabilities`：只读投影 durable capability 记录（provider × action 三态、version、probed_at、证据引用），供等待面与测试断言消费。
- 一次性语义：已 Confirmed 且 exact version 与当前 CLI 一致的 action 行不重跑；三行全齐时点击秒回 `already_confirmed`（仅做一次 `--version` 探测）；版本漂移或行缺失才补探测。不点击不运行任何探针。
- 探针失败如实上报（稳定错误码 + action + detail），已通过的三方校验导入不回滚、不伪造 Confirmed；Fake/未知 provider 返回稳定 unsupported 错误，不冒充探测。
- 前端在 LC 管理面板新增 provider capability 卡：四家真实 provider 的能力状态只读展示 + 每家一个「核验」按钮调用新端点，结果（already_confirmed / revalidated / 错误详情）就地回显。
- 不改门本身：admission/gateway fail-closed 语义、sandbox/边界探针机制、自动触发（零自动探测）均保持不变。

## Capabilities

### New Capabilities

- `provider-capability-revalidation`: LC 作用域内 provider capability 的用户显式重验证通道——真实现场探针、原子 durable 导入、版本钉定的一次性幂等语义与只读状态投影。

### Modified Capabilities

（无——本 change 只新增验证通道；capability 门、admission 等待面与五步 bootstrap 动作面的既有 spec 行为不变。）

## Non-Goals

- 不改 admission/gateway 的 fail-closed 门语义，不加「迁移默认 allow」或手工改 Confirmed 的旁路。
- 不改 sandbox/bwrap 探针机制、fixture 拓扑与证据 schema（`BOUNDARY_PROBE_EVIDENCE_SCHEMA`）。
- 不加自动/定时/启动时探测触发；探针只在用户显式点击时运行。
- 不做 detached 任务状态机：同步执行（与既有 aggregate index rebuild 分钟级同步动作同先例）；不做多 provider 批量单点，逐家点击。
- 不动五步 bootstrap 投影（`LogicalCodebaseBootstrapStep` 不新增步骤），不改 bootstrap actions 既有分支语义。

## Impact

- 后端：`src/product/logical_codebase/`（新增 revalidate 服务：探针通道映射、fast-path 判定、逐 action 探测+导入）、`src/web/handlers/`（新 GET/POST 端点）、`src/web/app.rs`（路由）、`src/web/state.rs`（测试注入口）。
- 前端：`web/src/api/`（capability 状态与 revalidate 客户端）、`web/src/components/lifecycle/`（capability 卡）。
- 测试：产品层单测（seam 注入：fast-path/导入/失败/unsupported）、it_web 集成（HTTP 面 + durable 断言 + 幂等秒回）、前端组件测试。
- 受益：页面 E2E S4 及后续阶段在有真实 CLI 的现场可经该通道自举 capability，无需测试侧播种。

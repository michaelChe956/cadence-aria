# Proposal

## Why

LC 聚合初始化目前仍有 provider turn 占位实现，且 admission 在 provider spawn 前要求成员规则，导致尚无规则的新 LC 无法启动生成规则的 recipe，形成冷启动死锁。预研已证明四家 provider 能从非 Git、多成员根真实发现根规则并调用 MCP；现在可按 root-cwd 模型收敛根初始化、信任前提与后续 session 契约。

## What Changes

- 将 LC 初始化定义为聚合根上的 root-cwd recipe：保留既有五步 durable operation，以四条无中断命令生成根规则与配置，不进入单仓注册、持久化或 GitFinalize 外层流程。
- 为后续 LC 会话中需要原生读取根配置的 Codex/Kimi provider，在 LC 根准入已冻结 canonical root 后、Claude Code root recipe 启动前建立独立的、精确、幂等、可撤销的 provider trust 登记准备；该准备不作为 Claude Code recipe 的执行前提，Codex LC root 启动适配追加 `--skip-git-repo-check`。用户级 trust 写入失败时，依赖该 trust 的后续会话 fail-closed 并进入等待面，保留可审计记录。
- 将 admission 与 bootstrap readiness 规则源统一切至 canonical LC root；新增由 durable Running operation 派生的 bootstrap phase credential，仅豁免根规则尚未生成，不豁免 policy、authority、capability、gateway、target 与 cwd 校验。
- 冻结 LC provider cwd 为 canonical root，同时保持 member/checkout/worktree target 独立；扩展 envelope、adapter input、resume fingerprint 与 spawn 前复验。逻辑 writable roots 不宣称 OS 沙箱，保留 provider-specific 写权限证据门及 D4 cross-target baseline。
- 保留两套五步状态机并建立 readiness 闭环；为四锁 D1–D4 更新行为回归，补充 cwd≠target 专用 fixture 与端到端验收。
- 采用 `AGENTS.md` 为通用根指令入口、`CLAUDE.md` 为兼容副本；skills 安装及机器级 managed links 零改造，不新增注入或成员配置复制通道。
- **不改变**传统单仓初始化/会话、单仓 cwd、GitFinalize 与既有 role 工具策略。

## Capabilities

### New Capabilities

无。本 change 扩展现有 logical-codebase 与初始化契约，不新增独立 capability。

### Modified Capabilities

- `non-interrupt-repository-bootstrap`: REQ-BOOT-01 的 LC“逐仓本地化由确定性程序完成”语义由 root-cwd recipe supersede；保留单仓契约，并补齐生产执行和步骤状态要求。
- `logical-codebase-registration`: REQ-REG-12 将实际规则准入源切为 canonical LC root；增加 Codex/Kimi trust 登记生命周期与失败等待面。
- `logical-codebase-aggregate-planning`: 规定 LC 会话 cwd=root、目标身份与 cwd 分离，并保持 root policy/rules 作为唯一规则事实源。
- `session-policy-envelope`: 澄清 cwd 与 target 分离、cwd 纳入 resume fingerprint；声明 provider trust 与原生 root 配置发现前提；保留受控 pointer publication 状态但不把指针作 root-rules fallback，保留 Kimi Aria MCP 注入契约。

## Non-Goals

- 不改变传统单仓 recipe、provider cwd、GitFinalize、operation 或工具策略。
- 不实现 C6 legacy 身份冲突处理、不做存量 LC 静默迁移；旧 per-member digest 必须等待、准备并重跑 root recipe 后重新冻结。
- 不向成员仓发布新 thin pointer，不向成员 checkout 复制根配置或写规则。
- 不新增 Aria prompt、环境变量、adapter sidecar 等 provider 配置注入通道；ENV-06 原生自发现边界与 ENV-08 Kimi 受控注入边界维持原契约。
- 不把逻辑 writable roots 或 git status 证据宣称为 OS 级隔离；大 LC 成本、非 target 写边界和多版本 CLI 作为实施期验收，不由已有预研替代。
- 不扩展为自动发现任意外部目录/嵌套 monorepo，不新增 provider fallback、成员 cwd 回退或宽泛路径授权。
- codex/pi/kimi 的 LC gateway 接入（包括后续 Story/Design/Plan/Coding/Review 的真实 provider launch）由后续 change `lc-gateway-multi-provider` 承担，不在本 change 范围内；本 change 只交付 Claude Code recipe 与四家 provider 根配置发现的证据。

## Impact

修改的契约 capability 为 `non-interrupt-repository-bootstrap`、`logical-codebase-registration`、`logical-codebase-aggregate-planning`、`session-policy-envelope`；权威设计为 `cadence/designs/2026-09-30_方案设计_LC根初始化_v1.0.md` v1.3。预研证据见 `cadence/reports/2026-10-01_预研报告_LC根初始化provider实测_v1.0.md`。实现预计 11–15 人日，含信任登记与 Codex 启动旗标的约 0.5–1 人日增量。
# Design

## Context

权威设计为 [`cadence/designs/2026-09-30_方案设计_LC根初始化_v1.0.md`](../../../cadence/designs/2026-09-30_方案设计_LC根初始化_v1.0.md) v1.2；本文件只记录实现决策摘要，不复制全文。预研证据为 [`cadence/reports/2026-10-01_预研报告_LC根初始化provider实测_v1.0.md`](../../../cadence/reports/2026-10-01_预研报告_LC根初始化provider实测_v1.0.md)。

当前 `AggregateInitializationCoordinator` 已有五步 durable operation 与 `AggregateProviderTurnDriver` 接口，但聚合 provider turn 仍需接入真实四命令循环；`LogicalCodebaseProviderAdmissionPreflight` 的规则来源需要从 active member checkout 切换到 canonical LC root。`SessionPolicyEnvelope` 已冻结 policy/target/writable roots，需增补独立 `working_directory`；现有 gateway/resolver 测试中存在 cwd 与 target 相同的 fixture，不能替代分离形态证据。

预研在非 Git、多成员根中验证 Claude Code、codex 0.155.1、pi、kimi 2.0.2 均可通过根 `AGENTS.md` 引用 `.claude/rules` 并完成真实 MCP 调用；codex 需要用户级 projects trust 与 `--skip-git-repo-check`，kimi 需要 workspace trust。大 LC 成本、非 target 写边界、多版本 CLI 未覆盖，仍是实施期验收门。

## Goals / Non-Goals

**Goals:**

- 在 canonical LC root 执行一次可恢复、可审计的四命令 root recipe，并闭合 recipe operation 与 bootstrap readiness 两套五步事实。
- 让所有 LC provider session 使用 root cwd，同时保留 member/checkout/worktree target、逻辑 writable roots、target resolver 与 D4 基线的独立约束。
- 以 durable Running operation 派生唯一 bootstrap phase credential，解决根规则冷启动死锁而不放宽 policy、gateway、capability、cwd 或 target 校验。
- 将 Codex/Kimi trust 登记作为 recipe ① 冻结 canonical root 后、首个 provider turn 前的确定性、幂等、可撤销、可审计步骤；失败时 fail-closed。
- 保留 provider 原生 root discovery、AGENTS.md 通用入口、CLAUDE.md 兼容副本和机器级 skills 安装零改造结论。

**Non-Goals:**

- 不改变单仓 initialization、cwd、GitFinalize、legacy direct topology 或既有 role tool-policy。
- 不新增 prompt/环境变量/sidecar 注入，不向成员仓复制配置或新增 thin pointer，不做 C6 和存量 LC 静默迁移。
- 不把逻辑 writable roots、git status 或 MCP 调用成功表述为 OS 级写隔离；未实测成本、写边界和版本矩阵不提前承诺。

## Decisions

### 1. Root-cwd 三字段模型

`working_directory` 是 provider 进程 cwd 与目录发现根，唯一取自 manifest `provider_context_root` 的 canonical path；`target` 是 member/checkout/worktree identity；`writable_roots` 是 envelope 的逻辑授权。LC 的 planning/review/author 继续空写根，Coder 仅允许 target worktree。launch 层按 validated envelope 重绑 cwd，target resolver 继续复验成员归属、git-dir/worktree identity 和 TOCTOU。

选择该模型而不是把 target worktree 继续兼作 cwd，是因为预研证明 root-native discovery 可用，而混用两者会让根配置发现和精确 coding 写边界相互污染。单仓把两字段映射为既有目录，保持零变化。

### 2. 两套状态机与 bootstrap credential

保留 recipe operation 的 MachineSkills → AggregatePreflight → PreCheck → RuleAndMcpConfig → OpenspecAndExamples，以及 bootstrap projection 的 Identity → ManifestCheckout → RulesPolicy → MemberIndex → AggregateIndexActive。AggregatePreflight 冻结 canonical root 后、PreCheck 前登记所选 Codex/Kimi 的 trust，不新增第六个 step；recipe 完成不直接等价于 `planning_ready`，RulesPolicy、索引和摘要/receipt 必须闭环。

bootstrap phase credential 只由当前 Running operation 的 LC/root/step/input digest 派生，provider/admission 在 spawn 前重新读取并比对。它只跳过“根规则尚未生成”，不能跳过 policy、authority、capability、gateway、canonical cwd、target 或 availability。普通 session 不能自报 credential。

### 3. Root recipe 与产物审计

复用已有四命令的顺序、取消、超时、摘要和单命令 session 语义，但不复用单仓 registration coordinator、Repository persistence 或 GitFinalize 外层。生产 root receipt/filesystem auditor 记录 canonical root、step/command、前后快照、allowlist、content digest、policy/rule identity 与未知变更证据；用户冲突、symlink escape、成员工作树变化或不可观测写入 fail-closed。

### 4. Provider trust 与自发现边界

Codex 在用户级 `~/.codex/config.toml` 的 projects trust 登记当前 canonical root，并在 LC root 启动追加 `--skip-git-repo-check`；Kimi 以 `wd_<basename>_<sha256(canonical_root)[:12]>` 写 workspace-trust 记录。根 provider turn 前完成所需登记，普通 session 前继续复验；登记只影响当前 root，重复执行幂等，仅撤销本 LC 管理的记录，用户原有 trust 不被误删，LC 删除/解绑可撤销，写入失败停等，审计不记录无关 trust 或凭据。pi/Claude 不增加用户级 trust 写入。

AGENTS.md 是四家通用入口，CLAUDE.md 是兼容副本；既有机器级 `CadenceSkillsManager::prepare()` 与 managed links 不因 cwd 变化而重装、复制或重链。provider 原生自发现与 Aria 注入 bundle 是独立通道：前者沿用 ENV-06/08 的受信任边界，后者继续原有 allowlist、digest、脱敏和审计。

### 5. Gateway、admission 与回归锁

Normal admission 检查 canonical root rules/policy；bootstrap admission 使用 credential 仅放宽根规则存在性。两条 gateway factory 生成的 root 必须 canonical-equal，否则 zero spawn。D1 的 BootstrapExecutor marker 仅由 credential、bootstrap action、root 和 receipt context 联合证明；D2 receipt auditor 拒绝未知变更；D3 允许抽取共享四命令 executor 但禁止单仓 registration/GitFinalize 调用图；D4 对 LC coding/retry/review 继续采集全部 member main checkout baseline。

必须新增 cwd≠target fixture、resume fingerprint cwd 漂移、缺根零 spawn、provider-specific writable evidence 及完整 E2E；旧同 cwd fixture 只保留为兼容边界。

## Spec Mapping

- `non-interrupt-repository-bootstrap`：REQ-BOOT-01 改为 LC root-cwd 四命令一次执行，新增 REQ-BOOT-03 recipe/readiness 闭环与 REQ-BOOT-04 bootstrap phase credential；单仓场景原样保留。
- `logical-codebase-registration`：REQ-REG-12 将实际规则源切为 canonical root，并纳入 trust/capability/gateway 失败等待；新增 REQ-REG-14 Codex/Kimi 登记、撤销、隔离和 fail-closed。
- `logical-codebase-aggregate-planning`：REQ-PLN-01、03、06、07 增加 root cwd、独立 target、resume/access 快照与全入口一致性；其余聚合 selection/change-order 契约不变。
- `session-policy-envelope`：REQ-ENV-01、03、04 增加 canonical `working_directory` 与 cwd fingerprint；REQ-ENV-06、07、08 澄清 provider 自发现、既有 pointer 与 Kimi Aria bundle 边界；新增 REQ-ENV-10 root-cwd 会话契约与 REQ-ENV-11 cwd≠target 合法形态。

## Risks / Trade-offs

- [跨工作区副作用] Codex/Kimi trust 写入用户 home 可能受权限、并发或格式冲突影响 → 采用 root-scoped 幂等键、前后摘要、撤销记录与失败等待；不覆盖无关配置。
- [Provider 自发现写边界不可由预研证明] 根 MCP 真实调用不等于 OS 级只读 → 保留 provider-specific evidence gate、D4 baseline 和前后快照；证据不足阻断支持结论。
- [cwd 与 target 解耦扩大可见范围] provider 可能看到多个成员 → 所有 target 仍经过 authority/gateway/git identity 复验，planning/review 空写根，coding 只保留 target worktree。
- [旧 fixture 误导] 同 cwd/target 测试无法证明新契约 → 增加专用分离 fixture，旧测试只声明同路径兼容边界。
- [大 LC 成本和 CLI 漂移] 预研仅覆盖单版本小 fixture → 实施期以数十成员、版本矩阵和上下文/启动时延报告设门，超预算或不兼容版本 fail-closed。

## Migration Plan

1. 新 LC 先完成 identity、manifest/checkout 与 canonical root preflight；root recipe 运行前不启动普通 provider。
2. AggregatePreflight 冻结 canonical root 后先完成所需 Codex/Kimi trust 登记、核验与副作用审计；失败即等待、禁止首个根 provider turn。随后 recipe 按原 durable step 生成根规则/policy/MCP/receipt，不新增 step ID。
3. bootstrap projection 仅在 RulesPolicy、MemberIndex、AggregateIndexActive 与摘要一致后进入 `planning_ready`；之后所有 LC session 使用 root cwd + 独立 target。
4. 已有旧 per-member digest 的 LC 不静默切换：进入 waiting → Prepare → root recipe → 新 root policy/rule receipt 冻结；单仓完全走旧路径。
5. 任一阶段失败保留 checkpoint/receipt/外部副作用事实，执行原链 retry/continue；若发现 root identity、未知写入或 trust 撤销不确定，保持 fail-closed，不自动回滚用户文件。

## Open Questions

以下问题不改变已冻结的 root-cwd 规范和任务拆分，可在实施期用证据回答：

- provider-native 根文件最终 allowlist 在真实版本矩阵中的具体集合。
- 数十成员 LC 的上下文、token、启动时延预算与阻断阈值。
- 各 provider/OS 对非 target 成员写尝试的 evidence gate 证据与支持矩阵。
- 祖先 Git 污染、Codex 项目级 MCP 限制和多版本 CLI 的最终报告口径。
## Context

Coding Workspace 当前生产 runner 编排 Coder、Code Reviewer、review request 与 Internal Reviewer，但没有 Testing 阶段的生产调用。完成门禁仍同时支持 schema v2 group、legacy group 与 single-attempt 路径，并在三条路径中要求 required verification check 对应 Passed 或 PassedWithWarnings testing report。

这形成了不可满足的门禁：internal PR review 已通过、unit 与 handoff 已完成，但 attempt 因不存在 testing report 停留在运行态。产品已确认至少未来半年不引入 Testing 阶段，并选择最小、可逆的改动范围：只解除完成门禁与 testing report 的绑定，保留 Testing 相关模型、存储、枚举、Provider 配置与其他基础设施。

## Goals / Non-Goals

**Goals:**

- 让 schema v2 group、legacy group 与 single-attempt 在 internal PR review 通过且非 testing 门禁满足时完成。
- 即使 canonical contract 或 legacy verification plan 声明 required 检查，缺少 testing report 也不阻塞完成。
- 保留文件范围、runtime binding、handoff、unit 状态、worktree 清洁性等既有非 testing 完成校验。
- 以回归测试固定“Testing 不参与完成判定”的产品语义。

**Non-Goals:**

- 不编排或实现 Testing 阶段。
- 不删除 Testing stage、tester prompt/configuration、TestingReport 模型与存储。
- 不新增项目级、attempt 级或环境级 testing gate 开关。
- 不改变 Internal Reviewer、review request、rework 或 final-confirm 的其他行为。
- 不自动迁移、自动重放或直接修改历史停滞 attempt。

## Decisions

### 1. 从所有完成路径移除 testing report 校验

完成门禁的 schema v2 group、legacy group 与 single-attempt 路径均不再调用 required verification result 校验函数；不只修复当前 schema v2 生产路径，以免同一完成语义在不同 attempt 类型中分叉。

**替代方案：** 仅修改 schema v2 group。该方案改动最小，但 legacy/single 路径仍保留一个生产 pipeline 无法满足的门禁，行为不一致，因此不采用。

### 2. 删除专用于完成判定的 testing 校验函数

删除原 `verify_schema_v2_required_gates_satisfied` 与 `verify_required_gates_satisfied` 实现，在完成门禁调用点留下产品决策注释。保留不再被调用的公共错误变体 `VerificationGateResultMissing`，避免为本次行为放宽引入无必要的公共类型破坏。

**替代方案：** 保留函数并无条件返回成功。该方案便于日后恢复，但制造误导性死代码和未使用参数，恢复成本优势不足，因此不采用。

### 3. 不引入配置开关

Testing 至少半年不进入产品流程，当前完成语义对所有 attempt 一致生效。新增 `testing_gate_enabled` 等开关会扩大配置、持久化、API 与测试矩阵，违反 YAGNI。

若未来恢复 Testing，必须通过新的 OpenSpec change 同时设计生产编排、report 绑定、失败恢复和完成门禁，而不是只重新打开旧校验。

### 4. 保留所有非 testing 完成门禁

此次仅解除 TestingReport 依赖。changed-file/runtime scope、可见 handoff、全部 unit 完成、completion commit、共享 worktree 清洁性及现有一致性检查仍按原顺序执行并可阻塞完成。

### 5. 旧语义测试显式退役但保留上下文

断言“缺少 testing report 必须返回 `VerificationGateResultMissing`”的 legacy 集成测试不再代表产品要求。由于该 fixture 当前还受既有 lineage NotFound 问题阻塞，不在本 change 中修复无关 lineage 行为；测试将标记为 ignored，并注明 testing 门禁已停用及既有 fixture 阻塞原因。新增可运行的 schema v2 回归测试承担新语义的主要验证。

## Risks / Trade-offs

- **[风险] 未执行自动化测试的代码也可被标记完成。** → 这是已确认的产品取舍；继续依赖 Code Reviewer、Internal Reviewer 与非 testing 结构门禁，UI/API 不伪造 testing 成功结果。
- **[风险] 存量 TestingReport 即使失败也不再影响完成。** → Testing 已明确不属于当前验收标准；report 数据继续保留，但不参与 completion decision。
- **[风险] 三条完成路径同时修改扩大回归面。** → 分别覆盖 schema v2 group，并对 legacy/single 调用结构做针对性测试或静态验证；运行全量 lib、clippy、fmt 和相关集成测试。
- **[风险] 历史停滞 attempt 不会自动完成。** → 部署后使用现有恢复/重试入口重新触发 final completion；若现有入口无法安全重放，则创建新 attempt，不在本 change 中改写持久化状态。

## Migration Plan

1. 部署移除 testing-report completion gate 的后端版本；无需数据迁移。
2. 经用户确认后重启后端，使新门禁生效。
3. 对当前停滞 attempt 尝试通过现有恢复/重试入口重新触发最终完成；失败时保留证据并新建 attempt 验证完整流程。
4. 回滚时恢复原三处校验调用与校验函数即可；现有 TestingReport 数据格式未变化。

## Open Questions

无。产品范围、持续时间、是否引入开关及 Testing 基础设施保留策略均已确认。

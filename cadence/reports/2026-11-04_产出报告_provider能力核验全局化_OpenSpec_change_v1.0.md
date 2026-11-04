# 2026-11-04 · 产出报告 · provider 能力核验全局化 OpenSpec change

日期：2026-11-04
工作树：`.worktrees/bugfix-b-11-02`
执行者：`SpecGlobalCapabilityVerification`（设计角色，只产出 OpenSpec 契约产物，**零产品代码改动**）

---

## 0. 一句话结论

已产出 change `globalize-provider-capability-verification`（proposal + design + 两份 spec delta + tasks + `.openspec.yaml`），
`openspec validate <change> --strict` ⇒ **exit 0**；`openspec validate --specs --strict` ⇒ **仍 78 passed / 0 failed**。

## 1. 产物清单

```
openspec/changes/globalize-provider-capability-verification/
├── .openspec.yaml                                    （owner 决策登记）
├── proposal.md                                       （Why / 目标 / 非目标 / 范围 / 风险 / owner 决策）
├── design.md                                         （12 条决策 + 冲突裁决 + 25 条额外影响面）
├── tasks.md                                          （7 节 43 个可勾选工作包，按依赖排序）
└── specs/
    ├── provider-capability-revalidation/spec.md      （4 MODIFIED + 1 ADDED(REQ-PCR-07)）
    └── non-interrupt-repository-bootstrap/spec.md     （2 MODIFIED + 1 ADDED(REQ-BOOT-14)）
```

## 2. validate 输出

```
$ openspec validate globalize-provider-capability-verification --strict
Change 'globalize-provider-capability-verification' is valid
EXIT=0

$ openspec validate --specs --strict
Totals: 78 passed, 0 failed (78 items)
EXIT=0
```

## 3. 方案要点（owner 裁定的落点）

- **写入**：一律落项目级全局 `projects/{project}/logical-codebase/capabilities.json`
- **读取**：只读项目级全局；**无迁移脚本、无读时提升、无兼容读取分支、无 `for_lc` 别名**（owner 裁定：本产品无历史数据）
- **门 L2 联动**：L2 读全局 ⇒ 四家 provider 全局核验一次 ⇒ 冷启动推进到聚合初始化时 operation 自动创建并开始 ⇒ **用户不再需要往聚合根放 `AGENTS.md` + `.claude/rules/language.md`**
- **口径澄清（写入 proposal / spec）**：该自动开始路径**会**真跑四条无中断命令、**会**消耗所选 provider 额度；聚合根放两个判定文件仍是**另一条**零 provider turn 路径，本次**不取消、不改写**它

## 4. 我额外发现的影响面（design.md §13，逐条已核对行号）

最容易被漏的四条：

| # | 发现 |
|---|---|
X1 | `StoreBackedProviderCapabilitySource::for_lc`（`production_policy_resolvers.rs:233`）**同时**承担 capability store 作用域与 root-recipe 凭据重核验通道（per-LC 的 operation store）⇒ 只改一半会留下「能力读全局、凭据仍按 LC」的半改状态；已设计为职责拆分 + 改名自证 |
| X2 | `ProviderCapabilityRevalidateService::revalidate(project_id, lc_id, provider)` 的 `lc_id` **还**用于探针执行上下文与证据根 ⇒ 不能盲删参数；证据根随记录一起全局化 |
| X3 | 同文件 `ProductionPolicyTargetResolver::for_lc`（`:53`）**不是** capability 作用域（checkout 目标解析必须按 LC）⇒ 不得一起改 |
| X18 | `provider_selection.rs:900-903` / `:928-931` 的两个测试场景原本靠**不同 `lc_id`** 造「同源一致 / snapshot 漂移」两种场景；作用域改全局后该区分方式失效 ⇒ 必须改用记录内容区分，**不得**靠删断言绕过 |

其余：X4（gateway_factory 两处 `match lc_id`）、X5、X6/X7（路由与失败面 `details.revalidate.path` 保留 `lc_id`）、
X8/X9（模块文档已正确；legacy 判定只看 `manifest.json` ⇒ 全局化无影响）、X10~X17、X19~X25（夹具播种点与被并行任务改动的
`ProviderCapabilityBatch.tsx` 需按文案内容定位而非行号）。

## 5. 纪律声明

- 未改任何产品代码（`src/` / `web/src/` / `tests/`）——`git status` 中 `web/src/components/lifecycle/*` 的改动来自**并行任务**，非本任务
- 未改 `openspec/specs/**`（主 spec 等归档时 sync）
- 未做任何 git 操作（无 add / commit / branch / stash）
- 未创建迁移脚本、未留兼容分支

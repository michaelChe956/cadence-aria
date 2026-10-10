# Proposal: fix-admission-member-rules-scope

## Why

owner 使用四个纯骨架成员仓（均无 `.claude/rules/language.md`）验收时，「启动聚合初始化」被 `member_language_rules_missing` 阻断。公共 admission 把所有 active 成员的规则缺失泛化为任何相位的必需材料，既与五阶段多仓 provider 使用 canonical aggregate root、SingleCandidate（SC）才按实际 `repository.path` 读取规则正文的消费边界不符，也重建了首次初始化要消除的冷启动死锁。

漂移来自 lcg 计划 Task 3a/评审发现 A9 的「成员规则缺失任何相位阻断」决策，而非 OpenSpec 规格级推翻：在案但未归档的 `lc-root-initialization/specs/logical-codebase-registration/spec.md` 的 REQ-REG-12 已要求「成员缺失、根规则与 receipt 有效时使用根材料继续，不复制成员规则」。本 correction change 仅持久化已冻结的三路收窄契约，同时修正该规格对 fresh-LC 尚无根规则的张力。

## What Changes

- **路 A：普通根消费者。** 根规则、已发布政策和最终 receipt 有效时，不因不消费的成员 `language.md` 缺失阻断；`missing_member_rules` 保留为非阻断审计事实，不清空含有真实阻断项的整个 `missing_materials`，不提示准备无关成员。根材料缺失或既有校验发现漂移时维持原门；不改变、不扩大预存的 `Normal` 加自举桩政策的根缺失放行边界。
- **路 B：AggregateBootstrap。** 仅由 durable Running operation 派生并重验通过的 bootstrap credential 可豁免成员规则门；首次根 recipe 不以尚未生成的根规则和最终 receipt 为前提。gateway 按重验后的相位复用现有 `validate_root_recipe_request`，保留真实 capability 门，不伪造 `Confirmed`，不把 recipe 证据提供给普通会话。
- **路 C：SC 实际消费路径。** LC SC 以已解析的 `repository.path` 为唯一消费来源，而非 SC 标签、成员数量或全体 active 成员；消费前读取本身成为 waiting 门，成功读取的同一份 `String` 用于 prompt。缺失、权限错误、目录及非法 UTF-8 均在 provider 前可诊断停等，保持 `member_language_rules_missing`、Prepare/Retry、durable Prepare/Open 和零启动；非 LC 行为不变。
- **恢复裸仓验收。** 回滚 controller 手工补入四个验收成员仓的规则文件，并同样处理会被装配器复制入成员基线的四个模板源；保留真实 SC 测试自备的 `seed_member_rule_material`。仅处理已确认的注入，不重置仓库、不覆盖用户修改；已回滚的文件保持不存在。

## Supersede 与规格时序

本 change **显式取代（supersede）** `cadence/plans/2026-10-02_实施计划_LC网关多provider接入_v1.0.md` Task 3a 的成员缺失「任何 phase 均阻断」条款及评审发现 A9 的对应处理（取证锚点：261、263、631 行）。取代范围仅是成员规则消费边界；authority、policy、trust、capability、gateway、cwd、target、D4 和 #8 发布链各门不因本 change 放宽。

REQ-REG-12 delta 针对**创建本 change 时** `lc-root-initialization` 中尚未归档的现行修订文本；若该 change 在本 correction 同步/归档之前先归档，则以归档后的 `openspec/specs/logical-codebase-registration/spec.md` 为目标复核并应用本 delta。不得把尚未归档的「成员缺失时使用根规则」场景错误归因于旧归档主规格，也不得用旧「任何成员缺失都等待」文本覆盖本次修正。

本 change 的准确新场景「有效 bootstrap credential 解除裸仓首建死锁」**显式替换** `lc-root-initialization` 的旧 Scenario「bootstrap phase 只豁免根规则」身份；旧场景的「只豁免根」语义已被本 change supersede，不能把该错误标题留作新行为标识。若旧 change 先归档，同步者须在归档后主规格将该旧场景身份及内容替换为本 delta 的准确新场景，再对新基线完成 strict 校验；不得仅为保留旧标题兼容校验而恢复错误语义。

## Capabilities

### New Capabilities

无；不增加新 capability。

### Modified Capabilities

- `logical-codebase-registration`：修订 REQ-REG-12，明确普通根消费者、有效 bootstrap 和 SC 实际消费来源的准入边界及验收场景。

## Non-Goals

- 不扩建 receipt TTL 或全源摘要；不把现有 receipt 校验描述成所有五阶段已经具备相同校验能力。
- 不自动生成、复制或补齐成员规则；该替代方案已否决。
- 不改变 trust、capability、cwd-authority、target、D4、#8 发布链的门或既有 SC 拒绝启动与 conversational-gate revision 注入合同。
- 不增加伪装成 phase 的 SC 分类，不靠 `PlanningReadOnly` 等 action 推断 SC，不从其他 worktree 兜底寻找规则文件。
- 不治理五阶段历史准入缺口，不顺手扩大普通请求的 placeholder 放行。
- 本次交付仅为 change 文档，不修改产品、测试或服务配置，不执行 commit；4317 服务仍由 controller 管理。

## Impact

- 生产边界：公共 admission 的成员诊断/阻断划分、bootstrap preflight 到既有 RootRecipe gateway 的接线、LC SC 消费前同源读取与 waiting 投影；checkout 身份使用既有解析结果。
- 测试边界：先修真实有效根 fixture，再改写历史全相位阻断测试；保留 SC 核心停等臂，增加三路矩阵、凭据漂移、同源消费及既有根/身份/capability 门回归，调整 bootstrap repair 集成时序。
- 终验锚：装配后确认 `busi`、`api`、`gateway`、`frontend` 四仓无任何规则文件，重新点击「启动聚合初始化」，不再被 `member_language_rules_missing` 阻断，根材料由 recipe 生成且成员零写，五阶段后续可继续。

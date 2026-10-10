# 方案设计:admission 成员语言规则门收窄(2026-10-10 v1.0)

> 状态:已获批(oracle `adopt_with_changes` + max-task `feasible_with_changes` 双审一致,owner 拍板「开干」)。
> OpenSpec change:`openspec/changes/fix-admission-member-rules-scope/`(proposal/design/spec delta/tasks,strict 校验通过)。
> 实施计划:`cadence/plans/2026-10-10_实施计划_admission成员语言规则门收窄_v1.0.md`。

## 1. 背景与问题

owner 用纯骨架仓(四成员仓 busi/api/gateway/frontend,无任何规则文件)验收「添加代码库→建 LC→登记→启动聚合初始化」,被 provider admission preflight 阻断:

```
member_language_rules_missing
missing_materials: [member frontend missing /tmp/notify-center/frontend/.claude/rules/language.md, ...×4]
allowed_actions: [Prepare, Retry]
```

当前实现(`src/product/logical_codebase/provider_admission_check.inc.rs:184-265`):**全体 active 成员×任何相位**,缺一即拦。

### 1.1 架构事实(ScoutRootVsMember 带行号取证)

| 事实 | 证据 |
|---|---|
| 五阶段多仓 validated 流 provider 进程 cwd 一律=canonical aggregate root(cwd/target 分离;coding 写码定向成员 git worktree 但进程在根) | author_root_launch.inc.rs:129-132 等 |
| 成员仓 language.md 唯一真实运行时消费者=SingleCandidate 链(读 repository.path 全文进 prompt) | run/single_candidate.rs:634-647 |
| 其他阶段规则权威=聚合根政策;明令不得在 target worktree 猜 language.md | routing_reference.rs:47-62 |

### 1.2 规格在案文本与漂移根源

- lc-root-initialization change(未归档)REQ-REG-12 MODIFIED(specs/logical-codebase-registration/spec.md:9-22):「成员仓原有规则材料缺失,但 canonical LC root 的规则与 policy receipt 有效」时「系统 SHALL 使用已核验的根规则材料继续,不将成员规则缺失当作根规则缺失,不复制成员规则」
- 漂移根源:lcg 计划 Task 3a/评审发现 A9 把 missing_member_rules 钉为「任何相位阻断」——计划层决策,从未写入 OpenSpec。A9 真实关切(bootstrap 不得连带豁免成员规则)成立,但修法过冲,重建了 lc-root-initialization 立项要消除的冷启动死锁
- oracle 核查:2026-09-30 后无任何 change 在规格层推翻上述文本;本方案=回归规格,非新发明

## 2. 方案:三路收窄

### 路 A:普通根消费者(五阶段多仓入口)

根材料(根规则+已发布政策+最终 receipt)有效时,成员 language.md 缺失**不阻断**;missing_member_rules 保留为**非阻断审计事实**(不清空整个 missing_materials——其中含「成员无 recorded checkout」等真阻断项);allowed_actions 不再提示准备无关成员。根材料缺失/漂移维持 fail-closed;预存 Normal+自举桩政策根缺放行边界不动、不扩大。

### 路 B:AggregateBootstrap 相位(聚合初始化 recipe turns)

**死锁关键**:裸仓首建时根规则尚不存在(正是 recipe 要生成的)——只按「根有效放行」死锁依旧。有效 bootstrap credential(由 durable Running operation 派生并重验;LC/root/step/input digest 漂移即失效)进入时豁免成员规则门;gateway 校验按已重验 phase 走既有 `validate_root_recipe_request`(provider_gateway.rs:1061-1074),合法 recipe 不先被 Normal capability 矩阵拒绝。保留 capability 门,不伪造 Confirmed,recipe 证据不给普通会话。

### 路 C:SingleCandidate 精确化

SC 不是成员数判据:多 target 且 focus 非唯一锚 aggregate_root_view 时 repository.path=聚合根(workspace_repository.rs:393-406)——SC 按**实际解析出的 repository.path** 判断必要材料。落点 run/single_candidate.rs:498-647:LC 实际文件读取变为消费前 waiting 门,**复用同一份 String 构造 prompt**(不预检一份、消费另一份);NotFound/权限/目录/非法 UTF-8 在 provider 前可诊断停等(reason=member_language_rules_missing 语义,Prepare/Retry,durable Prepare/Open,provider 零启动)。非 LC 旧行为、SC 阻断合同(split prompts 拒绝启动+revision 注入)不动。admission 与 repository 解析收敛到同一已解析来源(identity registry primary checkout),不从其他 worktree 兜底。

## 3. 边界语义(禁止事项)

- 缺失与不可读不混称:不消费的成员文件不做无谓读取
- ProviderRuleReference digest:None 全库无下游消费(实施时复核),保留审计记录
- receipt 缺失/损坏/locator/AGENTS digest/root 身份漂移不得借成员豁免放行;placeholder 不是「根材料有效」证明
- 不动:trust/capability/cwd-authority/target/D4/#8 发布链各门;SC revision 合同
- 非目标:不扩建 receipt TTL/全源摘要;不自动生成成员规则(已否决:动用户仓+1.5-2 天+违背「不复制成员规则」MUST NOT);不治理五阶段历史准入缺口

## 4. 回滚条款

- controller 曾手动补进四测试仓的 language.md(违反 REQ-REG-12 MUST NOT「不复制成员规则」)——已回滚,四仓恢复裸 init commit
- e2e/fixtures/repos/{四仓}/.claude/rules/language.md 模板源(a273726c 加入,assemble 会复制入成员基线、持续掩盖裸仓场景)——已删除
- 区分:tests/it_web/web_lc_gateway_multi_provider/harness.rs 的 seed_member_rule_material 是 SC 测试自备材料,**保留**

## 5. 验收锚

owner 裸仓(四仓无任何规则文件)重新点「启动聚合初始化」→ 不再出现 member_language_rules_missing 阻断;根规则由 recipe 生成;成员仓树/Git 状态零写;五阶段后续可继续。

## 6. 关键资源

| 内容 | 位置 |
|---|---|
| OpenSpec change(strict 通过) | openspec/changes/fix-admission-member-rules-scope/ |
| 冻结契约 brief | local://admission-change-brief.md |
| oracle 裁定 | agent://OracleAdmissionNarrow |
| max 技术审查 | agent://MaxAdmissionNarrow |
| 架构取证 | agent://ScoutRootVsMember |

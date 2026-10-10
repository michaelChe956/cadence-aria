# admission 成员语言规则门收窄 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把「全体 active 成员×任何相位缺 language.md 即阻断」的 admission 收窄为三路:普通根消费者根材料有效即放行、AggregateBootstrap 凭据豁免、SingleCandidate 按实际消费路径。

**Architecture:** 修改 `provider_admission_check.inc.rs` 的成员规则分支(诊断与阻断材料分离)+ bootstrap 相位复用既有 root-recipe 校验通道 + SC 消费点把实际读取变为 waiting 门并复用同一份正文。规格契约在 OpenSpec change `fix-admission-member-rules-scope`。

**Tech Stack:** Rust(axum)、vitest 不涉及(纯后端)、cargo test。

**Spec:**
- `cadence/designs/2026-10-10_方案设计_admission成员语言规则门收窄_v1.0.md`
- `openspec/changes/fix-admission-member-rules-scope/`(proposal/design/specs/tasks)
- `local://admission-change-brief.md`(冻结契约)

## Global Constraints

- 不动 trust/capability/cwd-authority/target/D4/#8 发布链各门;不伪造 Confirmed
- placeholder 不是「根材料有效」证明,不得借机扩大普通请求放行
- missing_member_rules 保留非阻断审计;不清空整个 missing_materials(含「成员无 recorded checkout」真阻断)
- 缺失与不可读不混称;不消费的成员文件不做无谓读取
- receipt/locator/AGENTS digest/root 身份漂移不得借成员豁免放行
- SC 阻断合同(work_item_split_engine/prompts.rs 拒绝启动+revision 注入)与非 LC 旧行为不动
- TDD:每个行为变更先写失败测试

## Review Focus

1. 裸仓首建(recipe turn)缺根规则+缺成员规则 → 应放行(bootstrap 凭据);测试在 Task 3
2. SC 多 target 无唯一 focus(读根)而他成员缺规则 → 应继续;测试在 Task 4
3. 实际消费文件「存在但不可读/目录/非法 UTF-8」→ provider 前停等,不是预检过后运行时才炸;测试在 Task 4
4. 凭据漂移(LC/root/step/input digest 变化)→ 拒绝,不静默降级;测试在 Task 3
5. 根材料缺失+成员缺失并存 → 仍 fail-closed(不把成员豁免误当根豁免);测试在 Task 2

---

### Task 1: 有效根 fixture 先行

**Files:**
- Modify: `src/product/logical_codebase/provider_admission_preflight_tests.inc.rs:285-438`(默认 fixture 无根 language)、`:1241-1292`(publish_root_policy_with_receipt 不建根 language)

**Interfaces:**
- Produces: 测试夹具函数(真实根 language.md+已发布政策+最终 receipt),供 Task 2/3 断言复用

- [ ] **Step 1: 构造真实根材料夹具(根 .claude/rules/language.md + policy artifact + finalized receipt),不改任何断言**
- [ ] **Step 2: `cargo test provider_admission_preflight` 现有套件全绿(夹具不破坏既有语义)**

### Task 2: 路A 普通根消费者放行

**Files:**
- Modify: `src/product/logical_codebase/provider_admission_check.inc.rs:210-265`(成员规则分支)、`:289-354`(根材料判定)、注释 :184-188
- Test: `provider_admission_preflight_tests.inc.rs`(missing_member_language_rule_waits_before_provider_spawn 改写;ordinary_missing_root_session_zero_spawns 更正名实)

**Interfaces:**
- Consumes: Task 1 夹具
- Produces: admission 结果语义——根材料有效时成员缺失 → ready,missing_member_rules 仅审计

- [ ] **Step 1: 失败测试:根材料有效+成员缺失 → 放行且审计记录保留;根材料缺失+成员缺失 → 仍 fail-closed**
- [ ] **Step 2: 跑测试确认失败(member 缺失仍阻断)**
- [ ] **Step 3: 实现诊断/阻断分离(allowed_actions 不再含无关成员准备)**
- [ ] **Step 4: 跑测试通过;全 fixture 套件绿**
- [ ] **Step 5: Commit `fix(admission): 根消费者成员规则缺失不再阻断`**

### Task 3: 路B bootstrap 凭据豁免

**Files:**
- Modify: `provider_admission_check.inc.rs:356-379`(gateway 校验按已重验 phase 走 validate_root_recipe_request)、`coordinator_provider_turn.inc.rs`(凭据传递)
- Test: bootstrap_phase_only_waives_missing_root_rules、lcg_t03_member_language_rule_missing_is_not_root_phase_exempt 改写;web_lc_bootstrap_repair.rs:1010-1056 随行为更新

**Interfaces:**
- Consumes: ProviderAdmissionPhase::AggregateBootstrap(credential)已重验(:149-168)
- Produces: bootstrap 相位成员门豁免;凭据漂移拒绝;Normal 相位 capability Unknown 对照不变

- [ ] **Step 1: 失败测试:有效凭据+裸仓(recipe 首建)→ 放行;凭据漂移 → 拒绝;无凭据普通请求 → 仍走 Normal 矩阵**
- [ ] **Step 2: 确认失败**
- [ ] **Step 3: 实现 phase 分流(复用 validate_root_recipe_request,不伪造 Confirmed)**
- [ ] **Step 4: 测试通过;web_lc_bootstrap_repair 套件绿**
- [ ] **Step 5: Commit `fix(admission): bootstrap 凭据豁免成员规则门`**

### Task 4: 路C SC 按实际消费路径

**Files:**
- Modify: `src/web/workspace_ws_handler/run/single_candidate.rs:498-647`
- Test: `src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs`(保留 SC 臂+新增矩阵)

**Interfaces:**
- Consumes: repository.path 实际解析(workspace_repository.rs:393-406)
- Produces: SC 消费前 waiting 门(同一 String 构造 prompt);聚焦成员有规则他成员缺失可继续;四类读取错误停等

- [ ] **Step 1: 失败测试:focus 成员缺规则 → SC waiting(零启动,Prepare/Open);其他成员缺+focus 有 → 继续;多 target 无唯一 focus(读根)→ 按根判断;不可读/非法 UTF-8 → 停等**
- [ ] **Step 2: 确认失败**
- [ ] **Step 3: 实现:消费点读取复用同一正文,checkout 来源收敛 identity registry primary**
- [ ] **Step 4: 测试通过;SC 套件+split prompts 套件绿**
- [ ] **Step 5: Commit `fix(sc): 成员规则按实际消费路径停等`**

### Task 5: 收口与全局验证

**Files:**
- Modify: `openspec/changes/fix-admission-member-rules-scope/tasks.md`(勾选)
- 全局:`cargo test`(admission/SC/bootstrap repair/gateway multi-provider 全套)、`cargo build --release`

- [ ] **Step 1: 全套受影响测试绿**
- [ ] **Step 2: release 构建成功**
- [ ] **Step 3: controller 重启服务,owner 裸仓验收:启动聚合初始化不再被 member_language_rules_missing 阻断**
- [ ] **Step 4: Commit 收口+change tasks 勾选**

## 执行说明

- 实施者:glm53-task(单轮承接;每任务 TDD 先红后绿;遇文档与代码冲突停下报告)
- 服务 4317 由 controller 管理,实施者不重启
- 回滚条款已由 controller 执行完毕(四仓裸态+fixtures 模板删除),实施者只核实不重建

# Task 3 报告:Bug2-A 统一关闭原语与 gate_origin

- Commit: 178c9cfe `fix(gate): 统一持久化关门原语+gate_origin 关联——resolved_action/at 完整、幂等重放、重开防误关(Bug2-A)`(以此消息在 git log 的最新一条为准)
- 测试摘要: 红测确认(action=null/resolved_at 缺失/resolution_generation 缺失/gate_origin 缺失/首关事实被重写 5 类红)→ 绿:`cargo test --locked --lib coding_attempt_store` 217 通过、`failed_review_recovery` 57 通过、`group_review_failure` 19 通过、`reviewer_retry_diagnostic` 3 通过、`plan_confirmed_info` 9 通过、it_product 受影响 2 用例通过;本人文件 clippy 0 告警、rustfmt 干净。

## 交付内容

1. **统一持久化关闭原语**(gate.rs):`resolve_blocked_gate_with_action` 重写为 blocked-gate-mutations 锁内「校验→根记录写完整 resolved 事实(resolved_action+resolved_at)→写归档→删根记录」,返回 `ResolveOutcome{gate,resolved_action,resolved_at,resolution_generation}`(经 mod.rs 导出)。四种崩溃前缀幂等收敛:根已关归档未完→按根首关事实补齐;归档完根仍在→清理;都全→同动作重放返回首关结果;都不存在→NotFound。异动作冲突返回 `Conflict{coding_blocked_gate_resolved_action}` 不覆盖首关审计(None 传入视作不指定,仅收敛——兼容旧归档)。
2. **BlockedGateRecord** 新增 `resolved_at: Option<String>`(serde default,旧档 None 不伪造)+ `resolution_generation: u64`(default 0)。
3. **重开防误关**:`reopen_failed_code_review_gate_for_plan_repair` 入同一锁,递增代次+清空关闭字段+删归档;新增 `resolve_blocked_gate_for_origin(origin)`:仅 open 根记录 gate_id+generation 与 origin 精确一致才关(动作=origin.action_id);代次失配/异动作已关→Ok(None) 零写入;同代次同动作已关→幂等返回首关结果。Task 4 补偿直接消费。
4. **gate_origin**:`CodingRoleRun.gate_origin: Option<GateOrigin{gate_id,generation,action_id}>`(coding_models/role_run.rs,serde 可选旧数据 None)。首写入口=三个 gate-retry 入口+create_retry_role_run 继承:handler(coding_workspace_engine/gates_parts/blocked_gate.inc.rs)读 `open_blocked_gate_generation` 构造 origin 传入 `supersede_latest_role_run_and_create`(三个调用)与 `create_manual_retry_role_run`;recovery.rs 专用 retry(`ensure_failed_code_review_retry_role_run`)首写 origin(gate_id+open 代次+retry_review);`create_retry_role_run` 继承 prior.gate_origin;`supersede_latest_role_run_and_create` 改完整构造后首存(消除先 create 后补写的半构造首写)。不扩全仓通用 create_role_run。
5. **专用恢复链显式动作**:`resolve_failed_code_review_gate_idempotent` 改走统一原语+显式 `Some("retry_review")`(修 resolved_action=None 缺陷)。
6. **三处分散状态赋值接原语**:create_group_review_failure_gate 替代分支+结论门循环、resolve_open_group_review_failure_gates——统一 `system_superseded` 审计值,归档+删根。
7. **读面**:新增 `get_archived_blocked_gate_facts`(Task 4 启动对账用)、`open_blocked_gate_generation`。
8. **动作收口边界未动语义**:retry_review/retry_internal_review=admission+run 持久化后关;retry_coding(_with_context)=说明落盘+admission+run 创建后关,失败不关;retry_group_* 只 resume 不虚构代表 run,关门经原语统一记录动作;send_to_coder/manual_continue/accept_risk/abort/plan_terminated 各自前置成功后关(既有);provide_context 不关;非法动作零关闭零新增(blocked.rs 新守护测试)。

## 测试(TDD 红→绿)

- 红测(先写先跑,确认 5 类红):blocked.rs 首用例扩展(归档 action=retry_review+resolved_at 非空+retry run origin 首写)、重放幂等/异动作冲突、重开代次递增、普通 retry_review 路径 origin 首写;gate.rs 崩溃前缀四态收敛+首关事实保持。
- 新增:blocked.rs 4 用例+扩展 1;gate.rs 崩溃前缀 1;role_run.rs 单测 3(supersede 完整首存/manual origin 首写/自动重试继承)。
- 回归:上述摘要全绿;存量基线红(part_26/workspace_ws_integration/sc flake)未触及、未计入。

## Concerns

1. **面外机械适配 5 处**(接口变更的必要调用方迁移,各仅 +`None,` 实参,无行为变更):`coding_attempt_store/tests/failed_review_recovery.rs`、`coding_workspace_engine/tests/reviewer_retry_diagnostic_teaching.rs`(2 处)、`tests/it_product/product_coding_attempt_store/part_01.rs`、`tests/it_product/product_coding_workspace_engine/part_03.rs`。已越 brief 点名文件面,但均为本任务签名变更的编译必须,与 Bug1 文件面(git_workspace_service/gates/plan_final_confirm_tests)零冲突——请审查确认。
2. 全仓 `cargo clippy -D warnings` 基线本就红(存量 100+,如 conversational_gate/human_gate 等,非本批引入);本人文件 clippy 0 告警。`cargo fmt --check` 全仓当前红来自并行 Bug1 worker 未完成文件(git_workspace_service.rs/plan_final_confirm_tests.rs),非本人文件;本人文件 fmt 干净(曾全仓绿时验证)。
3. `resolve_blocked_gate`(None shim)保留:仅旧测试(coding_attention_info 等)与手动场景调用;生产恢复链已改显式动作。
4. Task 4 就绪接口:`resolve_blocked_gate_for_origin` / `get_archived_blocked_gate_facts` / `open_blocked_gate_generation` / `GateOrigin`(coding_models 导出);origin 关门对「异动作已关」返回 None(不与人工决策互抢),与原语直关的 Conflict 语义有意不同。
5. 已归档旧数据缺 resolved_at 按 None 解释(历史时间不伪造);Task 4 的「按 journal 证据补 action」属启动对账职责,本任务未做按证据回填。

## 修复轮 1(F1,P2,审查必修)

**问题**:统一关闭原语 `root_exists && archive_exists` 分支未看根 status,无条件「以归档为首关事实、删根」。reopen 半完成路径(写重开根→删归档)崩溃后产生「根 open+上一代残留归档」态,此时任何关门调用会删掉新代次 open 根、返回上一代 resolved 事实;异动作时对该旧事实永久 Conflict 卡门;与 reopen 原语对同态的收敛方向相反。

**修复**(gate.rs `resolve_blocked_gate_record_unlocked`,按 reviewer 建议):该分支先读根记录——根非 open(崩溃残留根)→维持原收敛(归档为首关事实,清理根);根 open(重开半完成残留)→删上一代残留归档,落回下方 root_exists 的 open 首关路径(以根为当前事实,新写完整 resolved 事实)。与 reopen 的「根已 open⇒仅清残留归档」同向收敛。

**覆盖测试**(新增 `resolve_blocked_gate_prefers_open_root_over_reopen_residual_archive`,TDD 红→绿):
- 红:首关(gen0/retry_review)→reopen(gen1)→手工重建残留归档→以 `abort` 关门 → 旧代码 `Conflict{coding_blocked_gate_resolved_action}` panic(红测实录:`open root must close despite residual archive: Conflict {...}`);
- 绿:同场景修复后关门成功,断言 `resolved_action=abort`、`resolution_generation=1`、根删除、归档为新事实(action=abort/gen=1)——不复活旧事实。

**命令与输出**:
- `cargo test --locked --lib resolve_blocked_gate_prefers_open_root` → 1 passed(修复前 1 failed:Conflict)
- `cargo test --locked --lib coding_attempt_store` → 217+1=218 passed, 0 failed
- `cargo test --locked --lib failed_review_recovery` → 57 passed, 0 failed

**Commit**:单独提交 `fix(gate): 关门原语区分重开残留归档与崩溃残留根——根 open 时以根为当前事实`(消息最新一条为准)。

**连带核查**:`resolve_blocked_gate_for_origin` 对同态先读根(open→代次比对→按根关门),不受残留归档影响;origin 代次失配仍零误关。审查遗留 O2(plan_terminated×在飞恢复交错)、O4(generation 读取不在锁内)已记录,归 Task 4。

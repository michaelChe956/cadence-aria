# Tasks: globalize-provider-capability-verification

> owner 2026-11-04 已拍板（全局作用域；本产品无历史数据 ⇒ 不做存量处理）。决策记录见同目录 `design.md`。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记高层工作包，每个工作包映射到相关 requirement。
> **纪律**：本工作包只改 `src/`、`web/src/`、`tests/` 与本 change 的 OpenSpec 产物；MUST NOT 改 `openspec/specs/**`（主 spec 等归档时 sync）。

## 1. capability store 收敛为唯一作用域（项目级全局）

映射 requirement：REQ-PCR-07

- [x] 1.1 `ProviderCapabilityStore` 删掉 `lc_id: Option<String>` 字段与 `for_lc(paths, lc_id)` 构造器；`capabilities_path` 固定为 `paths.logical_codebase_root(project_id).join("capabilities.json")`，不再经 `lc_scope_root` 派生（**不删 `lc_scope_root` 本身**——它仍服务 manifest / policy / trust / receipt / operation / checkout）
- [x] 1.2 同步 `provider_capability_store.rs` 模块文档（`:3-4` 自述落盘位置即项目级 ⇒ 保持，并删除与 per-LC 有关的表述）
- [x] 1.3 验收：`grep -rn "ProviderCapabilityStore::for_lc" src/ web/ tests/` → **0 命中**；`grep -rn "fn for_lc" src/product/logical_codebase/provider_capability_store.rs` → **0 命中**；`grep -rn "fn for_lc" src/product/logical_codebase/production_policy_resolvers.rs` → 仅剩 `ProductionPolicyTargetResolver::for_lc`（**不得**动，见 design X3）
- [x] 1.4 单测：项目级全局路径断言（`projects/{project}/logical-codebase/capabilities.json` 存在；`projects/{project}/logical-codebases/{lc_id}/capabilities.json` **不**存在）

## 2. 写入 scope 收敛为项目级全局

映射 requirement：REQ-PCR-07、REQ-PCR-01

- [x] 2.1 `src/product/logical_codebase/provider_capability_revalidate.rs:404-406`（`import_outcome` 的 durable writer）改用项目级 store ⇒ 探针结果导入全局记录
- [x] 2.2 `provider_capability_revalidate.rs:447-459`（`capability_evidence_root`）改为项目级（与全局记录同根）⇒ 一次全局核验的证据与记录同作用域
- [x] 2.3 `src/web/gateway_factory.rs:144-150`（`build_for_lc_material_prep` 的 `capabilities.ensure_bootstrap`）改项目级；`ensure_bootstrap` 语义不动（文件已存在即跳过、不覆盖）
- [x] 2.4 `src/web/gateway_factory.rs:237-243`（`durable_probe_writer_for_lc`）改项目级 ⇒ 与 gateway 读的作用域同源
- [x] 2.5 `src/product/logical_codebase/coordinator_provider_turn.inc.rs:955`（聚合初始化执行期 `ensure_bootstrap`，测试夹具内）改项目级
- [x] 2.6 验收：全仓 `upsert(` / `record_verified_probe(` / `ensure_bootstrap(` 的 capability 调用点，其落盘路径 SHALL 全部等于 `projects/{project}/logical-codebase/capabilities.json`；`grep -rn "logical-codebases" src/ | grep -i capab` → **0 命中**

## 3. 读取 scope 收敛为项目级全局

映射 requirement：REQ-PCR-07、REQ-PCR-01、REQ-BOOT-07

- [x] 3.1 `src/product/logical_codebase/provider_selection.rs:236-240`（**L2 判定**）改读项目级全局记录
- [x] 3.2 `src/product/logical_codebase/production_policy_resolvers.rs`：删除 `StoreBackedProviderCapabilitySource::for_lc`（`:233`），新增职责自证的构造器（建议名 `global_capability_with_lc_recipe_recheck(paths, project_id, lc_id)`）——capability store **全局** + root-recipe 凭据重核验通道**仍按 LC**（`AggregateInitializationOperationStore::for_lc` 不动）；`new` / `with_store` 语义不变
- [x] 3.3 `src/web/gateway_factory.rs:181-190`（gateway 的 capability source 装配）：`Some(lc_id)` 分支改用 3.2 的新构造器；`None` 分支保持 `with_store` / `new` 原语义（历史上无 root-recipe 通道）
- [x] 3.4 `src/product/logical_codebase/provider_capability_revalidate.rs:352-353`（只读状态投影 `capability_snapshot`）改读全局 ⇒ 状态面对不同 LC 给出同一份事实
- [x] 3.5 `production_policy_resolvers.rs:213-217`（`StoreBackedProviderCapabilitySource::new`，已是项目级）**不动** —— 作为正确形态样板
- [x] 3.6 验收：gateway 准入（`require_supported`）与 provider 选择器读**同一份**全局记录；`launch_cell_state` 仍是两者共用的唯一只读判定函数（**不新增判定函数**）

## 4. 门 L2 联动：全局核验后聚合初始化自动开始

映射 requirement：REQ-BOOT-14、REQ-BOOT-07

- [x] 4.1 确认 L2（3.1）读全局后，**四家 provider 全局核验一次**（用户执行一次「一键核验全部」）⇒ 冷启动推进到聚合初始化时 operation 被自动创建并开始；**不再需要**用户往聚合根放 `AGENTS.md` + `.claude/rules/language.md`
- [x] 4.2 端到端用例：聚合根**无**两个判定文件 + 四家全局能力已 `Confirmed` ⇒ 冷启动推进到聚合初始化 ⇒ operation 被创建并开始执行（不再是 `aggregate_initialization_provider_needs_verification`）
- [x] 4.3 端到端用例：聚合根**无**两个判定文件 + 全局记录中无任何候选为 `Confirmed` ⇒ **不创建 operation**，返回既有「需要核验」失败码与可点击核验入口（**不得**自动发起核验）
- [x] 4.4 回归用例：聚合根**有**两个判定文件 ⇒ 继续走「零 provider turn」路径（免过 provider 选择门、`provider_selection` 为空、`stream_launches() == 0`）——该路径 MUST NOT 被本次变更取消或改写
- [x] 4.5 回归用例：某候选全局记录为 `Denied{reason}` ⇒ 选择器继续按 owner 顺序探测下一家（**不得**放行 `Denied`）
- [x] 4.6 回归用例：L1（health degraded）判定与既有四家拒绝码（`aggregate_initialization_no_provider_available` / `_provider_needs_verification` / `_provider_capability_denied` / `_provider_health_degraded`）逐字不变

## 5. 界面文案改全局口径

映射 requirement：REQ-PCR-06

- [x] 5.1 `web/src/components/lifecycle/ProviderCapabilityBatch.tsx` 里 `data-testid="capability-batch-scope"` 的那段文案：「逐 LC 核验：会真实跑一次探测，可能几分钟，消耗 provider 额度。」⇒ 全局口径（如「全局核验：一次核验全项目可用；会真实跑一次探测，可能几分钟，消耗 provider 额度。」），**三项成本告知（真实探测 / 几分钟 / 消耗额度）逐项保留**（⚠ 该文件正被并行任务改动，**按文案内容定位，不按行号**；写盘前重读，避免覆盖他人改动）
- [x] 5.2 同步该段上方的注释（现写「REQ-PCR-06『按钮区域 SHALL 说明：逐逻辑代码库 / 真实探测 / 消耗额度 / 可能几分钟』」）
- [x] 5.3 同步三处前端断言：`web/src/components/lifecycle/ProviderCapabilityBatch.test.tsx:402`、`ProviderCapabilityCard.test.tsx:618`、`LogicalCodebaseManagementPanel.test.tsx:117`（另 `ProviderCapabilityBatch.test.tsx:8` 的文件头注释）
- [x] 5.4 验收：`grep -rn "逐 LC" web/src/` → **0 命中**；`grep -rn "逐逻辑代码库" web/src/` → 仅允许出现在「能力是项目级事实」的说明性注释里，**不得**出现在面向用户的按钮区文案
- [x] 5.5 验收：按钮位置、逐项进度「核验中（N/4）」、失败项「重试」、`skipped` 引导【安装】、版本落后引导【更新】**行为零变化**（属块2/块3 已冻结契约，MUST NOT 顺手改）

## 6. 测试夹具迁移与断言改写（只改作用域，不改断言语义）

映射 requirement：REQ-PCR-07、REQ-BOOT-07（全部受影响用例）

- [x] 6.1 产品层夹具：`provider_capability_revalidate.rs:718-719` / `:762-763` / `:796-797` / `:824-825` / `:875-876`；`provider_boundary_probe.rs:2366-2371`；`provider_admission_preflight_tests_policy_body.inc.rs:781` / `:812-816`；`provider_capability_store.rs:1155-1159` / `:1629-1634`（后两处**本来就是项目级** ⇒ 只需确认无需改）
- [x] 6.2 选择器夹具：`provider_selection.rs:561`（`write_records` 播种）、`:900-903`、`:928-931`。**注意**：这两处原本靠**不同 `lc_id`** 造出「同源一致」与「snapshot 漂移」两种场景；作用域改全局后该区分方式失效，MUST 改用**记录内容**（capability_snapshot_ref 不同）来区分，**不得**靠删断言绕过
- [x] 6.3 web 层夹具：`src/web/handlers/automation_enrollment_test_support.rs:859-860`；`src/web/wiga_gate_fixture.rs:252` / `:507`（**两处同改**）；`src/web/test_controls/plan_repair/provider_matrix.rs:431-435`；`src/web/handlers/automation_gateway_preflight.rs:569-573`；`src/web/handlers/automation_target.rs:186`；`src/web/handlers/automation_enrollment_tests_reenable.inc.rs:627`；`src/web/handlers/automation_reenable_supersede_tests.inc.rs:647`
- [x] 6.4 聚合初始化夹具：`src/web/handlers/aggregate_initialization/tests_parts/root_safety.inc.rs:475` / `:621`；`tests_parts/selection_failure.inc.rs:126` / `:435` / `:485`
- [x] 6.5 **改写依赖 per-LC 语义的断言**（语义变更必然红，逐条改成新口径）：
  - `src/web/gateway_factory.rs:524` `build_for_lc_scopes_policy_and_capability_to_lc_subtree` ⇒ 拆成两条：policy / manifest **仍按 LC**（断言不变）+ capability **落项目级全局**（`legacy_root/capabilities.json` 存在、LC 子树下**不存在** `capabilities.json`）
  - `src/product/logical_codebase/provider_gateway_tests/audit.inc.rs:174-176`（读 durable 字节）⇒ 路径改全局，断言语义（导入不得改写既有 durable 字节）不变
  - `src/web/workspace_ws_handler/tests/gateway_start_parts/author_launch_cases.inc.rs:649-652`（删 `capabilities.json` 以确定性触发 validate 失败）⇒ 路径改全局，触发语义不变
  - `tests/it_web/web_lc_capability_revalidate.rs:198-206`（拒绝路径不得写 capability 记录）⇒ 路径改全局，断言语义不变
  - `tests/it_web/web_lc_capability_revalidate.rs:423-426`（播种旧 v2 字节）⇒ 路径改全局
- [x] 6.6 验收：`grep -rn "capabilities.json" src/ tests/ web/` ⇒ 每处命中的路径要么是项目级全局，要么是**故意断言「LC 子树下不存在」**；MUST NOT 残留任何「往 LC 子树写 capability 记录」的夹具
- [x] 6.7 纪律：MUST NOT 删用例、MUST NOT 加 `ignore`、MUST NOT 放宽断言强度（改的是**作用域**，不是**期望值**）；MUST NOT 改任何非 capability 的 per-LC 作用域（manifest / policy / trust / receipt / operation / checkout / index 的断言一律保持）

## 7. 验证矩阵

映射 requirement：REQ-PCR-01 ~ REQ-PCR-07、REQ-BOOT-03、REQ-BOOT-07、REQ-BOOT-14

- [x] 7.1 `cargo test --locked --lib` ⇒ 全绿（含 `provider_selection` / `provider_capability_revalidate` / `provider_capability_store` / `production_policy_resolvers` / `gateway_factory` 各模块单测）
- [x] 7.2 `cargo test --locked --all-targets` ⇒ 全绿（含 `tests/it_web/web_lc_capability_revalidate.rs` 与 `tests/it_web/provider_gateway_envelope.rs`）
- [x] 7.3 `pnpm test`（web）⇒ 全绿（含三处文案断言）
- [x] 7.4 `cargo clippy --locked --all-targets -- -D warnings` ⇒ exit 0；`cargo fmt --check` ⇒ 干净
- [x] 7.5 行数门禁（`tests/it_core/large_file_guard.rs`）⇒ 不新增超限文件；新增/改动文件按既有登记口径处理
- [x] 7.6 `openspec validate --specs --strict` ⇒ **仍 78 passed / 0 failed**（MUST NOT 把现有 spec 改红）
- [x] 7.7 `openspec validate globalize-provider-capability-verification --strict` ⇒ exit 0
- [x] 7.8 契约留痕：本 change 的 proposal / design / 两份 spec delta / tasks；归档前补独立审查结论
- [ ] 7.9 真机验收：①「一键核验全部」跑一次 ⇒ 面板显示全局口径文案；②新建第二个逻辑代码库 ⇒ 面板直接显示已核验（**无需**再核验）；③冷启动推进到聚合初始化 ⇒ **不往聚合根放** `AGENTS.md` + `.claude/rules/language.md` 也**能**自动开始；④聚合根放了两个判定文件 ⇒ 仍走零 provider turn 路径

## 8. 机器级化（owner 2026-11-05 追加裁定：provider 是平台公用能力）

> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`。本节只记高层工作包。范围与验收边界见同目录 `proposal.md` / `design.md` 决策 13~16。

映射 requirement：REQ-PCR-07（作用域升级）、REQ-PCR-08、REQ-PCR-09、REQ-BOOT-14（整机生效）

- [ ] 8.1 **落盘机器级**：capability 记录与探针证据根改为机器级目录（`.aria/provider-capabilities/`）；项目级目录与 LC 子树下 SHALL NOT 再产出 capability 文件
- [ ] 8.2 **读写点全量迁移**：写入侧（探针导入 / `ensure_bootstrap` 自举 / 外部入口导入）与读取侧（provider 选择门 L2 / gateway 准入 / 只读状态投影 / 批量作业前置）全部指向机器级；SHALL NOT 残留第二条读路径或回退分支
- [ ] 8.3 **root-recipe 事实拆出**：`root_recipe_evidence` 从 capability 记录移出、改为 per-LC 独立事实；`require_root_recipe_supported` 改读新载体；SHALL NOT 改变判定强度与结论
- [ ] 8.4 **路由改机器级**：新增四条机器级路由；旧四条 `/api/projects/{p}/logical-codebases/{lc_id}/capabilities*` **删除**（MUST NOT 留兼容别名/转发 shim/双写）；批量作业落盘改机器级、job 结构去 `project_id`/`logical_codebase_id` 归属校验
- [ ] 8.5 **前端解门控**：能力卡与能力面板去掉 `projectId` / `logicalCodebaseId` props 与 id 取值逻辑；API 层路径改机器级；「Provider 管理」入口 SHALL NOT 以选中项目为前置条件
- [ ] 8.6 **文案口径**：按钮区说明改为机器级口径（「一次核验整机可用」）；面板顶部作用域说明同步；SHALL NOT 残留「逐 LC」「全项目」等旧口径
- [ ] 8.7 **夹具与断言迁移**：第 6 节列举的按项目级路径断言改指机器级路径；只改作用域、不改期望值；MUST NOT 删用例、加 `ignore` 或放宽断言强度
- [ ] 8.8 **验收 grep**：旧 capability 路由全仓 **0 命中**；`projects/**` 与 `logical-codebases/**` 下 **0** 个 capability 文件；机器级文件路径唯一
- [ ] 8.9 **验证矩阵**：同第 7 节全套（`cargo test --locked --all-targets` / `pnpm test` / `clippy -D warnings` / `cargo fmt --check` / `openspec validate --specs --strict` 仍 78 passed 0 failed / 本 change `--strict`）
- [ ] 8.10 **真机验收**：① 无选中项目时「Provider 管理」可打开并显示机器级事实（不再置灰）；② 在**另一个**项目 / 逻辑代码库下打开面板 ⇒ 仍是同一份机器级事实、**无需**再核验（验证「不存在第二作用域」，而非「跨项目共享」）；③ 机器级核验一次后，冷启动自动开始（不报 `aggregate_initialization_provider_needs_verification`）

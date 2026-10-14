# Task 3.7(B0f)实施报告:四 provider 公共汇合与审计一致

- 日期:2026-10-11
- Worker:ImplAnchorB037(glm53-task)
- Worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 依据:plan v1.1 §Task 3.7(L563-593)、tasks.md 3.7、设计 §5.4(B0 完整 projection 与计数口径)、§7.1-B0a、§4.1 reason 表(`resume_fingerprint_unavailable`)
- 前置:3.1–3.6 已全部落库(`744c5b65`/`4372de28`/`2c6c7e1a`/`3f30e806`/`10f1b529`/`be140540`/`6109e35f`);本包为汇合层,不跨改 provider 文件

## 一、交付面(Files 唯一写面)

| 文件 | 变更 |
|---|---|
| `src/product/logical_codebase/provider_gateway.rs` | ① `RESUME_FINGERPRINT_UNAVAILABLE` 常量(缺 full map/旧 basis 等待稳定原因码,与既有 `resume_fingerprint_mismatch` 区分);② `LcSupersededWaiting::fingerprint_unavailable()` 私有构造(不 supersede 审计未知会话);③ `PreparedLcResumeBasis` pub 枚举(`Full{previous_fingerprint, previous_session_id}`/`Unavailable`);④ `PreparedLcStreamingCandidate::full_context_fingerprint()`(五参 `from_envelope`,action evidence 取 validated 冻结 capability row 摘要);⑤ `resume_or_start_prepared_lc_candidate`:Full 臂委托 `resume_or_start_with_projection`(full 对 full),Unavailable 臂直接 `StartNew` 等待 `resume_fingerprint_unavailable` 且两臂 validated 都携带候选冻结 full projection;⑥ **`start_lc_prepared_streaming`:workspace LC PlanningReadOnly full 唯一 start 路径**——spawn 前 fail-closed 链(相位收口→重观测 facts→冻结版本比对→**混合 provider 矩阵守卫**(`Arc::ptr_eq` registry 当前绑定 vs 候选冻结 adapter,`PolicyDrift{provider_adapter}`)→**audit v2 完整性门**(以冻结 full projection 装配 `LcProviderStartAudit` 字面量,`is_v2_complete`+会话投影摘要非空,四家同判据)→身份一致门+admission verdict(capability row 摘要==投影 capability 摘要由 admission 第 3 步持有)→`with_lc_projection` 后 `revalidate_before_spawn`(指纹基准=候选 full projection,重算与冻结逐字一致)→kimi 例外映射→候选冻结 adapter `start_validated`→与 `start_streaming` 同源 gateway stream 审计);⑦ 测试模块声明 `mod tests`→`pub(crate) mod tests`(anchor_t10_support 供 lifecycle_store durable 测试复用,同 lifecycle_store `workspace_session_read_spy` 先例) |
| `src/cross_cutting/tool_policy_audit.rs` | **零生产改动**(合同"reuse `LcProviderStartAudit`/full 字面量、不改变 audit DTO 字段":v2 完整性门直接复用现有 `is_v2_complete()`,DTO/字段/错误枚举均未动;其 durable 语义测试落在下述 Test 文件) |
| `src/product/logical_codebase/provider_gateway_tests.rs` | 新增 `anchor_t10_support`(pub(crate):四家 `AnchorT10Family` 形态参数/`AnchorT10CapabilitySource`(version 与 facts 同源、action 行摘要==t10 projector capability 摘要)/`AnchorT10TrustSource`(可翻转,verify/fingerprint 同源)/`AnchorT10Projector`(匹配门只投影本家、capability 摘要与行逐字相等、负例可覆写 capability/会话摘要)/`AnchorT10FamilyAdapter`(真实 prepare 链 `LcVersionCache`+口径①计数;`start_validated` 消费冻结 full projection——裸投影 typed 拒并计 `starts_without_projection`;本家 mutable facts 复验(version 恒复验/kimi 追加 MCP 来源);audit 字面量与真实 `lc_start_audit_literal` 同形且 v2 fail-closed;resume 先对存档完整字面量判定(漂移→superseded 终止审计写旧 run+零 child 拒);pi extension 写只在 mismatch 判定后;child(③)/kill/reap 如实计数;成功才写 provider_start 首行含本家 argv/sandbox/approval)/`AnchorT10Harness`+`registry()/gateway_with_registry`+`anchor_t10_preset_resume_archive`)+`anchor_t10_convergence` 两用例(见 §三) |
| `src/product/lifecycle_store/tests/tool_policy_audit.rs` | 新增 `anchor_t10_durable_convergence` 模块两用例(见 §三):四家纯矩阵+真实 LifecycleStore durable 分区同源链 |

不改动:四家 provider 文件(3.3–3.6 写面)、`provider_projection.rs`/`streaming_provider` public core(3.2 写面)、既有 `start_streaming`/`resume_or_start*`/prepare 链零行为变化(仅新增 API)。

## 二、TDD 红→绿证据(cwd=worktree 根)

### 红(编译红:plan §0.2 "新接口初次可编译红"获准形态)

测试先写;以 HEAD 生产面(gateway 实现暂存 /tmp)运行:

命令:`cargo test --locked --lib anchor_t10`

- 20 个编译错误全部命中两文件的新用例符号:`no method named start_lc_prepared_streaming`、`no method named resume_or_start_prepared_lc_candidate`、`no method named full_context_fingerprint`、`cannot find PreparedLcResumeBasis`、`cannot find value RESUME_FINGERPRINT_UNAVAILABLE`、`module tests is private`(lifecycle 复用面)——即新接口缺失时新用例不可运行,红内容即合同缺口本身
- 证据文件:`/tmp/anchor_t10_red.txt`

### 绿(实现恢复后同命令)

```
test product::lifecycle_store::tests::anchor_t10_durable_convergence::anchor_t10_lc_start_audit_matrix_four_families ... ok
test product::lifecycle_store::tests::anchor_t10_durable_convergence::anchor_t10_durable_audit_same_source_full_path ... ok
test product::logical_codebase::provider_gateway::tests::anchor_t10_convergence::anchor_t10_full_projection_all_providers_no_basis_resume ... ok
test product::logical_codebase::provider_gateway::tests::anchor_t10_convergence::anchor_t10_prepared_facts_revalidate_before_session_start ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 4345 filtered out; finished in 0.01s
```

证据文件:`/tmp/anchor_t10_green.txt`

### 范围化回归(plan §8 表 3.2/3.7 行+相邻范围)

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib lcg_t09` | **19 passed / 0 failed**(gateway 全部旧 full 指纹/native 语义) |
| `cargo test --locked --lib provider_gateway` | **69 passed / 0 failed**(含 anchor_t09 prepare 合同全部旧断言) |
| `cargo test --locked --lib tool_policy_audit` | **23 passed / 0 failed**(durable 分区旧语义+新 anchor_t10) |

证据文件:`/tmp/anchor_t10_regression_lcg_t09.txt`、`/tmp/anchor_t10_regression_gateway.txt`、`/tmp/anchor_t10_regression_audit.txt`。本包文件 0 新 warning(现存 37 条 warning 中 3 条位于本包两文件者均为开工前既有:`provider_gateway.rs:1438` 的多余 `mut`(未触碰的 `resume_or_start_with_projection`)与 `provider_gateway_tests.rs:2417` 的 t09 projector 未用绑定,不属本包 hunk,按最小 diff 纪律不动)。

## 三、两个用例覆盖的合同断言(plan Step 1 逐项)

1. `anchor_t10_full_projection_all_providers_no_basis_resume`(gateway,Claude/Codex/Pi/Kimi 四家循环):
   - **默认 unsupported 不可达正例前置**:未接入 adapter(默认 trait)注册四家名下,full 路径 prepare 只得 typed `Adapter` 拒(`lc prepare facts is not supported`),零 registry start;
   - **真实 facts**:①probe 恰一次、exact_version/probe key provider/approval/MCP 家族值可观察;full projection 与 facts 同版本同源;
   - **fullmap/audit/fingerprint 同源**:`candidate.full_context_fingerprint()` 即 plan 断言的 `full_launch.fingerprint().digest`,与 prepared full map 记录(`LogicalLaunchFingerprintRecord{basis: FullLcProjectionV2, state: Prepared}` 的 `fingerprint_digest`)、adapter audit 字面量 `resume_fingerprint`、durable `provider_start.lc_projection` 五字段逐字相等;audit 字面量 `is_v2_complete()==true`;
   - **oldbasis/missingfull 拒等待**:legacy 指纹→`StartNew` 等待 `resume_fingerprint_mismatch`(supersede 审计 +1);`Unavailable`(缺 full map/旧 basis)→等待 `resume_fingerprint_unavailable`(supersede 计数不变,不审计未知会话)——即 plan 关键断言 `legacy_basis_recovery.reason_code == "resume_fingerprint_unavailable"`;两 disposition 后 ②③=0;
   - **full 唯一 start 路径**:成功 spawn ②=1/③=1/killed=0;spawn 前复验 TTL 命中 ①不增;`starts_without_projection()==0`(无 basis 旁路);gateway stream 审计 +1;
   - **本家事实消费(公共断言,不跨文件)**:Claude argv(fresh 无 `--resume`,同 ID resume 携 `--resume <id>` 且 native id 确认);Codex 持久化 `sandbox=read-only`/`approval=on-request`/受限 argv;Pi `extension_writes` 在 prepare/两判定阶段恒 0、仅 start 器械段=1(早写拒收);Kimi Aria bundle sha 与 `kimi-native-project-config` 两来源 prepare 的 facts/projection digest 不混源,trust digest `lc-trust-v1:KimiCode:…`;
   - **wrongID 如实 kill/reap**:预置同源存档+握手错 ID→`Err`,③+1 与 killed+1 相等(不伪称零 child),零 fresh provider_start。
2. `anchor_t10_prepared_facts_revalidate_before_session_start`(gateway):
   - **version(四家)**:identity+version 漂移→typed `PolicyDrift{provider_version}`,①重观测=2(换 key),②③=0,stream 审计 0;
   - **混合 provider 矩阵**:prepare→start 间 registry 换绑同家新实例/跨家(Claude 槽装 Codex adapter)→`PolicyDrift{provider_adapter}`,两侧 adapter ②③=0;
   - **audit v2 完整性门**:会话投影摘要为空的 projector 可 prepare 冻结但不可达生产 spawn(`UnsupportedCapability` 含 `v2 material incomplete`),②③=0;
   - **trust spawn 前变化受门**(Codex 需 workspace trust):prepare 冻结后 trust 翻转→typed 拒(含 trust),②③=0;
   - **相位收口**:ReviewReadOnly 候选可 prepare 但唯一 start 路径拒(`PlanningReadOnly` only),Coding/Recipe/Review 仍走同名 gateway API。
3. `anchor_t10_lc_start_audit_matrix_four_families`(lifecycle,durable 判据):四家(wire 文本各异)匹配→Resume;v2 三字段任一为空→拒(四家一致);旧 basis 存档(缺 lc_projection)→拒;跨家混比(claude current vs codex 存档)→拒。
4. `anchor_t10_durable_audit_same_source_full_path`(lifecycle,真实 LifecycleStore):Codex 家唯一 start 路径经 durable 分区落 provider_start 首行唯一——lc_projection 五字段/projection digest==gateway 冻结 full projection;fresh(seq 1)与 full-matching resume(seq 2)两代 provider_start 投影逐字相等;`find_latest_tool_policy_provider_start` 检索命中最新代;Resume validated 指纹==durable audit 字面量 resume_fingerprint(同源闭环)。

## 四、提交(按 plan Step 5 显式列表)

```bash
git add -- src/product/logical_codebase/provider_gateway.rs src/cross_cutting/tool_policy_audit.rs src/product/logical_codebase/provider_gateway_tests.rs src/product/lifecycle_store/tests/tool_policy_audit.rs
git commit -m "fix: B0f 四provider公共汇合与审计一致"
```

commit hash:`dcc8bac9`(3 files changed, 2165 insertions(+), 25 deletions(-);`tool_policy_audit.rs` 零改动不入 diff;25 删除=开工前遗留 fmt 重排 10 行+`mod tests;`→`pub(crate) mod tests;` 声明变更+defunct 占位行,与 3.6 报告 §四同惯例报备)。

## 五、边界与说明

1. **`start_streaming` 未加 PlanningReadOnly 硬拒**:plan Step 3 "现同名 gateway API 仍保持 Coding/Recipe 相位"——生产 caller(gateway_start/author_root_launch/drive/coordinator)仍在 4.1/5.x 迁移窗口内,现加硬拒会破全部在树 LC 启动与旧测试;"唯一"以结构保证:候选类型 opaque、crate 内唯一 spawn 出口为 `start_lc_prepared_streaming`(裸 validated 不携带 full projection,basis 旁路在 adapter 侧亦有 `starts_without_projection` 观测拒)。B1(4.1)起生产 caller 迁移到本路径。
2. **capability row 摘要==投影 capability 摘要的同源门**复用 admission 第 3 步既有比较(prepare 与唯一 start 路径都施加),不另造第二套判定;`full_context_fingerprint` 的 action evidence 因此取 validated 冻结 row 摘要,与 `resume_or_start_with_projection`/revalidate 重算/adapter 字面量逐字同源。
3. **parking_lot 规则偏差**:`parking_lot` 非本 crate 依赖(Cargo.toml 无;plan 冻结"不新增框架或锁体系";3.6 报告 §五同判),anchor_t10 新测试沿本文件既有 `std::sync::Mutex`+`lock().unwrap()` 约定(StaticCapabilitySource/AnchorT09 同型)。
4. **混合矩阵守卫的粒度**:`Arc::ptr_eq` 比较 registry 当前返回的 adapter(生产为 availability 包装实例)与候选冻结实例——同 gateway 内 prepare/start 恒同一实例(正例通过),任何 registry 重绑/跨网关串接即拒(负例通过);生产单 gateway 生命周期内 registry 不可变,无误杀面。
5. **并行协调留痕**:首跑测试编译被 trust 线 T6 修复轮在 `provider_run.rs` 的在途编辑(E0603)阻塞,经 Main 确认非废弃半成品、待其落地后重试;落地后取得本包红/绿,红内容不受并行影响(测试体自首版未改)。
6. **红证据取得方式**:测试先写;红跑时将本包 gateway 实现暂存 /tmp 并以 `git show HEAD:` 还原生产面(未用 stash,符合共享树禁令),跑毕即恢复——红跑树=HEAD 生产+新测试(+开工前 fmt),20 个编译错误全部指向新 API 缺失。
7. 独立审查(k3)与 Main 统一回归(plan §8 表)由 controller 调度;服务 4317 未动;e2e/ 未动;临时证据均在 /tmp;`/tmp/anchor_t37_gateway_impl.rs` 为红跑暂存件(已恢复,可清理)。

## 六、fix-loop round 1(P2:Full 臂 legacy basis 混比改走 Unavailable 待遇)

**Finding**(Main 转独立审查):原用例把 `sha256:legacy-v1-basis-t10` 传入 `PreparedLcResumeBasis::Full` 臂断言 mismatch+supersede+1——机械上正是 spec 禁止的「旧 basis 与 full 混比」(lc-gateway spec:41、workspace-session-resume-recovery spec:39:不同 basis → `resume_fingerprint_unavailable` 等待,不回填、不 supersede 未知会话),且对从未可信的会话写了 supersede 审计。

**红**(把混比断言当被禁行为写红,先测后修):

- 测试改写:legacy digest 误传 Full 臂必须 fail-closed 为 `resume_fingerprint_unavailable` 等待、supersede 计数不变;同时新增「真 v2 漂移臂」(带 schema 前缀、单维度不同的 `from_envelope` digest)保持 mismatch+supersede+1 既有语义覆盖。
- 命令 `cargo test --locked --lib anchor_t10` → `1 failed`(断言原文:"legacy digest fed to the Full arm must fail closed to resume_fingerprint_unavailable waiting, never a mixed comparison");证据 `/tmp/anchor_t10_fixloop_red.txt`。

**修**(写面仍限本包文件):

1. `SessionResumeFingerprint::from_envelope` digest 增加 schema 前缀:`{LC_RESUME_FINGERPRINT_SCHEMA}:sha256:<hex>`(fix-loop P2 ③落地为真守卫而非 debug_assert;schema 版本递增时前缀随变,旧 digest 自然 fail-closed);
2. 新增 `SessionResumeFingerprint::is_full_projection_v2()`(schema 前缀判别);
3. `resume_or_start_prepared_lc_candidate` Full 臂守卫:previous 无 v2 前缀 → 不进比对、不 supersede,直接 `StartNew{fingerprint_unavailable}`(与 `Unavailable` 臂同待遇);带前缀的 genuine 漂移仍委托 `resume_or_start_with_projection`(mismatch+supersede 保持);
4. `PreparedLcResumeBasis::Full` doc 合同(②):只接受 basis=`FullLcProjectionV2` 的 digest,B1/B2 读 `LogicalLaunchFingerprintRecord.basis` 非 Full 时必须传 `Unavailable`。

**绿与回归**:

| 命令 | 结果 | 证据 |
|---|---|---|
| `cargo test --locked --lib anchor_t10` | 4/0(legacy→unavailable、真 v2 漂移→mismatch 双臂均过) | `/tmp/anchor_t10_fixloop_green.txt` |
| `cargo test --locked --lib provider_gateway` | 69/0(含两处形状断言改前缀形态:tests L64、lcg_t09a 域测试 L1689-1698) | `/tmp/anchor_t10_fixloop_regression_gateway.txt` |
| `cargo test --locked --lib lcg_t09` | 19/0 | `/tmp/anchor_t10_fixloop_regression_lcg09.txt` |
| `cargo test --locked --lib tool_policy_audit` | 23/0 | `/tmp/anchor_t10_fixloop_regression_audit.txt` |
| `anchor_t33/t34/t35/t36`(from_envelope 共享函数保险) | 2/2/2/3 全绿(家测指纹比较两侧同源,格式变更一致传播) | 终端输出(见会话) |

**波及面说明**:`from_envelope` 为共享函数,但全部比较为 from_envelope↔from_envelope 同源(旧 API 的 `sha256:stale` legacy fixtures 走 mismatch 语义不变、resume 判据只比 5 投影字段不含 resume_fingerprint),唯一两处 digest 形状断言在本包文件内同步为前缀形态;durable `LogicalLaunchFingerprintRecord.fingerprint_digest` 由 B1/B2 持久化本格式,向前一致。

**提交**:`fix: B0f Full 臂 legacy basis 混比改走 Unavailable 待遇`(两文件:provider_gateway.rs + provider_gateway_tests.rs;tool_policy_audit 两文件本轮零改动)。

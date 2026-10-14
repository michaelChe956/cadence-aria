# Task 3.6(B0e)实施报告:kimi_code_provider 真实 facts 与 full 投影接线

- 日期:2026-10-11
- Worker:ImplAnchorB036Kimi(glm53-task)
- Worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 依据:plan v1.1 §Task 3.6(L531-561)、tasks.md 3.6、设计 §7.1-B0a(D5:真实 adapter 供 exact-version/MCP/boundary/approval facts+同版本纯 projector,spawn 前同 facts 重验/audit,default unsupported 仅未接入/测试 adapter)
- 前置:B0a1 已落库(`744c5b65`,只读 trust 冻结事实)、B0a2 已落库(`4372de28`,`prepare_lc_facts` trait 合同+`LcVersionCache` 60s TTL+`ProviderLcPrepareFacts`+gateway `prepare_lc_streaming_candidate`/`reobserve_lc_prepare_facts`)

## 一、交付面(Files 唯一写面,四文件)

| 文件 | 变更 |
|---|---|
| `src/cross_cutting/kimi_code_provider/mod.rs` | ① `KimiCodeProvider` 新增 `lc_version_cache: LcVersionCache`+`supplier_scope: u64`(实例唯一,进程内 `AtomicU64` 分配;derive Clone 保留,Debug 手动实现保持原两字段输出);② 私有 helpers:`lc_version_probe_key`(provider=KimiCode+canonical executable+只读可执行身份 len/mtime+`--version` argv+supplier scope,身份变化即换 key)/`lc_executable_identity`(纯 metadata 读,不可得固定哨兵)/`lc_version_probe`(有界 `--version`→`kimi X.Y.Z` 文本;exit0 之外 timed_out→`Timeout`、其余→`Unavailable`)/`observe_lc_version`(TTL 缓存观测+`ensure_kimi_version_compatible`,失败 fail-closed)/`lc_mcp_bundle_digest`(Aria bundle digest 或 `native-project-config` 标记);③ `prepare_lc_facts` 真实现:guard(通用策略例外稳定码→adapter 匹配)先于 version probe/child→TTL 观测→envelope 派生 boundary→`ProviderLcPrepareFacts::new`(KimiCode/KimiAcpV1/KimiAcp+exact_version+observed_at+probe_key+`KIMI_LC_APPROVAL_POLICY`+MCP 来源+boundary+同版本 `KimiPolicyProjector`);④ `start_validated` 第 4-5 段重写:exact version 改经 `observe_lc_version`(与 prepare 同 key 命中);validate 携带冻结 full projection(`resume_or_start_with_projection` 的 `Resume` 产物)时**消费该投影**——version/MCP 来源/boundary evidence 引用三重复验,不一致 typed 拒于 child 之前(`provider version drifted`/`mcp source drifted`/`boundary evidence drifted`+`session NOT started; no child was spawned` 字面量),audit `lc_projection` 字面量逐字段取自冻结 full projection(含 root trust digest,不建第二套投影矩阵);裸 validate 路径(既有测试/旧链)保持本 adapter 自建投影(trust 空)原语义;direct `start` 与 `probe_kimi_version` 无过期语义逐字节不变 |
| `src/cross_cutting/kimi_code_provider/projection.rs` | `boundary_evidence_ref_for` 私有→`pub(crate)`(start_validated 冻结投影 boundary 引用复验消费;零行为变化) |
| `src/cross_cutting/kimi_code_provider/tests/session_tests.rs` | 新增 `anchor_t36_provider_prepare_and_audit_share_full_fingerprint`+`anchor_t36_real_drift_zero_session_child`(详见 §三);fixture 扩展:`LcKimiResumeFixture::gateway_with_streaming_adapter`(注册真实 adapter+`with_readonly_lc_facts`)/`policy_revision_and_digest`/`gateway_supersede_count`,`gateway()` 提 `pub(crate)` 供 mcp 复用(零行为变化);`AnchorT36KimiCapabilitySource`(version=`kimi 0.34.0`,action 行 `projection_digest` 用真实 `KimiPolicyProjector` 对同 action+native 来源+version 计算,与 gateway admission 第 3 步同源)/`AnchorT36KimiTrustSource`(`fingerprint_facts` 返回固定 root record digest,Kimi 需 workspace trust);`anchor_t36_kimi_acp_script/fixture/rewrite`(版本占位注入,drift pad 使不同版本脚本 len 确定不同;`${0}.probe` 探测计数/`${0}.session-new` fresh 计数/pid 登记,与 9c fixture 同语义) |
| `src/cross_cutting/kimi_code_provider/mcp_bundle_tests.rs` | 新增 `anchor_t36_prepare_facts_mcp_source_follows_aria_injection`(adapter 级 prepare facts 的 Aria/native 来源分开面);`codegraph_bundle` 提 `pub(crate)` 供本包测试复用 |

不改动:其它三家 provider 文件、publiccore(gateway/provider_projection/streaming_provider)、`session.rs`(ACP 握手/kill 链既有语义原样)、`mcp_bundle.rs`、direct 路径。

## 二、TDD 红→绿证据(cwd=worktree 根)

### 红(行为红,非 0-tests/非 default 虚绿)

命令:`cargo test --locked --lib anchor_t36`

- 3 用例全部命中且 FAILED,失败点均为 default `prepare_lc_facts` unsupported 的行为断言:
  - `unexpected rejection: ... "streaming provider lc prepare facts is not supported by this adapter"`(通用策略例外断言)
  - `prepare freezes v1 facts before the drift: Adapter(...unsupported...)`
  - `native-source prepare facts are observed: Adapter(...unsupported...)`
- 证据文件:`/tmp/anchor_t36_red.txt`(`test result: FAILED. 0 passed; 3 failed; 4341 filtered out`)
- 红路径说明:首跑被并行兄弟包(codex/pi/claude)在途编译错阻塞,先后两轮全 crate 修复(本人两处:`gateway()` 可见性、panic 不 Debug 无 Debug 的 Ok 候选)后取得行为红;红内容不受影响(测试体自首版未改)

### 绿

命令:`cargo test --locked --lib anchor_t36`

```
test cross_cutting::kimi_code_provider::tests::mcp_bundle_tests::anchor_t36_prepare_facts_mcp_source_follows_aria_injection ... ok
test cross_cutting::kimi_code_provider::tests::session_tests::anchor_t36_real_drift_zero_session_child ... ok
test cross_cutting::kimi_code_provider::tests::session_tests::anchor_t36_provider_prepare_and_audit_share_full_fingerprint ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 4341 filtered out
```

证据文件:`/tmp/anchor_t36_green.txt`

### 范围化回归(plan §8 表 3.6 行)

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib kimi_code_provider` | **154 passed / 0 failed / 2 ignored**(含既有 4c/9c 全部 LC direct/validated/resume/superseded/kill 链用例与 direct 工具面;kimi 四文件 0 新 warning) |

证据文件:`/tmp/anchor_t36_regression_kimi.txt`(其余 37 条 warning 均为本包之前/其它包文件已存在)

## 三、三个用例覆盖的合同断言(plan Step 1 逐项)

1. `anchor_t36_provider_prepare_and_audit_share_full_fingerprint`(session_tests):
   - **通用策略例外保持**:gateway 为 Orchestrator 注入 deny 策略→adapter 以稳定码 `provider_generic_tool_policy_forbidden` typed 拒,且拒于 version probe 之前(probe 计数 0);
   - **prepare 真实 facts**:exact_version=`kimi 0.34.0`(真实 `--version` 观测恰一次)、probe_key provider/argv 冻结、observed_at 单调;
   - **native 来源**:`assert_eq!(native_mcp_source, "native-project-config")`(facts 与 full projection 一致);
   - **root trust 同源**:full projection `trust_digest()=="lc-trust-v1:KimiCode:sha256:kimi-root-record-anchor-t36"`(readonly `fingerprint_facts` 冻结);
   - **facts/projection 同版本同源**:full_projection.exact_version==facts.exact_version;
   - **prepare 零写**:policy revision/digest、gateway supersede、audit sink 事件前后相等;pid 标记未出现(probe 是独立 `--version` 观测,不计 session child);
   - **TTL**:spawn 前复验同 key 命中(总 probe 数仍 1);**ACP 同 ID**:session/load 确认请求 id,零 session/new;会话完成 `lc anchor t36 done`。
2. `anchor_t36_real_drift_zero_session_child`(session_tests):
   - **真实漂移**:prepare 冻结 v1→full-matching Resume 取得冻结 spawn 输入→同路径重写 fixture 二进制(len+mtime 变→可执行身份变→换 key 再观测);
   - **gateway 复验**:`reobserve_lc_prepare_facts`→`kimi 0.34.1`,`assert_frozen_version`→typed `PolicyDrift{provider_version}`;
   - **adapter spawn 守卫**:`start_validated` 于 child 之前 typed 拒(错误含 `provider version drifted`+两版本字面量+`session NOT started; no child was spawned`),`assert_eq!(mismatch_children, 0)`(pid 标记从未出现)、零 provider_start、零 session/new;
   - **错误握手**:第二场景 wrong native id——child 已启动→`Err`(双 id+`session NOT resumed`)、零 fresh(`assert_eq!(fresh_after_explicit_resume_count, 0)`)、零 provider_start、`assert!(wrong_native_child_killed_and_reaped)`(复用 `t09c_child_killed_and_reaped` 轮询 kill+reap)。
3. `anchor_t36_prepare_facts_mcp_source_follows_aria_injection`(mcp_bundle_tests):
   - **Aria/native 分开**:无注入实例 facts mcp==`native-project-config`(非 sha256 形态);`with_mcp_bundle(codegraph_bundle())` 实例 facts mcp==bundle.digest()(64-hex)≠native 标记;两实例同版本事实、各自 TTL 观测恰一次(supplier 隔离不串值);adapter 级只读观测零 spawn。

## 四、提交(按 plan Step 5 显式列表)

```bash
git add -- src/cross_cutting/kimi_code_provider/mod.rs src/cross_cutting/kimi_code_provider/projection.rs src/cross_cutting/kimi_code_provider/tests/session_tests.rs src/cross_cutting/kimi_code_provider/mcp_bundle_tests.rs
git commit -m "fix: B0e kimi_code_provider真实facts与full投影接线"
```

commit hash:`6109e35f`(4 files changed, 1032 insertions(+), 44 deletions(-);44 删除中 2 行为开工前遗留 fmt import 重排,与 3.2 报告 §四.1 同惯例报备)。

## 五、边界与说明

1. **跨家组合归 3.7**:本包只断言 Kimi 专属面(同 id/错 id 收口、native-source/bundle/trustfacts/projection/audit 同源、genericpolicy 例外、direct 工具面);未动 `provider_gateway.rs`/`tool_policy_audit.rs`/四家组合守卫。
2. **`start_validated` 版本观测语义变化说明**:LC validated 路径的 exact version 从无过期 `probe_kimi_version` 改为 60s TTL 观测(design D5 owner 裁决的 LC 合同;direct `start` 仍走 `probe_kimi_version` 无过期语义,两路严格分离)。同实例同 key 命中缓存;可执行身份(len+mtime)变化换 key 再观测。既有 4c/9c 用例(每用例新建 provider 实例)行为不变,154/0 全绿佐证。
3. **prepared projection 消费面**:冻结投影经 `ValidatedSessionLaunchPolicy::lc_projection`(Task 9a 字段,`resume_or_start_with_projection` 的 `Resume` 产物)进入 `start_validated`;fresh 转换面(`into_validated_streaming_input` 不带 lc_projection)保持裸 validate 自建投影路径,生产 caller 接线归 B1/B2(4.x/5.x),本包不越界。
4. **capability 行摘要对齐**:测试 fixture 的 `AnchorT36KimiCapabilitySource` 以真实 `KimiPolicyProjector` 计算 action 行 `projection_digest`(admission 第 3 步要求 capability 行与 full projection 同源);生产 capability 行归能力探针链,不由本包书写。
5. **并行协调留痕**:红/绿运行两次被兄弟包在途编译错阻塞并经 agent 消息协调修复(本人修 kimi 侧两处;codex/pi/claude 各自修复);全 crate 编译恢复后取得行为红,红内容未受并行影响。
6. **工具坑留痕**:edit 工具对相对路径解析到主仓 checkout 而非 worktree(hash mismatch),按合同改绝对路径+`git diff` 校验后恢复;一次误删行(fmt 后行号漂移)已即读即修,绿与回归均为修复后运行。
7. 独立审查(k3)与 Main 统一回归(plan §8 表)由 controller 调度;服务 4317 未动;e2e/ 未动;临时证据均在 /tmp。

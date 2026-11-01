# Task 3.2(B0a2)实施报告:TTL single-flight 与 public full prepare 合同

- 日期:2026-10-11
- Worker:ImplAnchorB032(glm53-task)
- Worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 依据:plan v1.1 §Task 3.2、tasks.md 3.2、设计 §5.4(版本缓存合同)+§7.1-B0a、owner 2026-10-11 `--version` 裁决
- 前置:B0.1 已落库(commit `744c5b65`,`provider_trust::ProviderTrustFingerprintFacts` 只读面)——本任务 `lc_trust_digest` 直接消费 `ProviderTrustSource::fingerprint_facts`,未复制推导

## 一、交付面(Interfaces 冻结逐项)

| 合同名 | 位置 | 说明 |
|---|---|---|
| `LC_VERSION_PROBE_TTL = Duration::from_secs(60)` | `src/cross_cutting/streaming_provider/mod.rs` | 单调时钟、非滑动(age≥60s 必再观测) |
| `LcVersionProbeKey { provider, canonical_executable, executable_identity, argv, supplier_scope }` | 同上 | derive Hash/Eq;identity/路径/scope 变化直接换 key |
| `LcVersionObservation { exact_version, observed_at }` | 同上 | 缓存值仅版本字符串+观测时刻 |
| `LcVersionProbe`(async factory 类型) | 同上 | `Fn(key) -> Pin<Box<dyn Future<Output=Result<String, VersionProbeError>> + Send>>`;single-flight 仅 winner 调用,不预造 future、不 spawn |
| `LcVersionCache` + `cached_lc_cli_version(key, probe, cancel)` | 同上 | 实例级进程内缓存;`with_clock` 为 crate-private 手动时钟测试 seam;watch 通道 single-flight(loser 等 winner 同一结果);失败/超时/取消不缓存、不回退过期成功;caller 取消 typed `Cancelled`(probe future 随之丢弃,迟到结果不入缓存) |
| `VersionProbeError::Cancelled` 新变体 | 同上 | Display 补臂;既有匹配均为 `matches!`/构造,无破坏 |
| trait `StreamingProviderAdapter::prepare_lc_facts`(async,默认 typed unsupported) | 同上(先例 977–994 的 start_validated 默认) | 默认拒绝仅供未接入/测试 adapter;真实接线归 3.3–3.6 |
| `GatedStreamingProviderAdapter::prepare_lc_facts` 透传 | `src/cross_cutting/provider_availability_gate.rs` | 先复验同一 gate 再转发 inner,错误原样上抛(wrapper 透传验收项) |
| `ProviderLcPrepareFacts`(provider_type/dialect/wire、exact_version、version_observed_at、version_probe_key、approval_policy、mcp_bundle_digest、boundary、projector) | `src/product/logical_codebase/provider_projection.rs` | 私有字段+pub(crate) 构造/getter;facts 只 crate 内 |
| `LogicalCodebaseProviderGateway::prepare_lc_streaming_candidate(input, request, audit, cancel) -> PreparedLcStreamingCandidate` | `src/product/logical_codebase/provider_gateway.rs` | 角色矩阵派生+双向守卫→validate→run-bound audit 绑定→registry(gated wrapper)→adapter facts→只读 trust 维度(B0a1 fingerprint_facts/不适用哨兵/裸 gateway 空串零回归)→gateway 私有 ProjectionInput 装配→facts 同版本 projector 冻结 full projection→身份一致门+admission_verdict(无副作用) |
| `reobserve_lc_prepare_facts(&candidate, cancel)` | 同上 | spawn 前重取当前 facts(同 TTL 合同;identity 变化换 key 再观测) |
| `PreparedLcStreamingCandidate`(opaque:input/request/validated/full_projection/facts/run_bound_audit/adapter) | 同上 | 字段私有仅 gateway 构造;`assert_frozen_version` typed `PolicyDrift(provider_version)` 拒;`into_validated_streaming_input` 为 crate 内唯一转换出口(仅 full-matching resume/明确 fresh 后;B1/B2 接线,`cfg_attr(not(test), allow(dead_code))` 标注) |

不改动:旧 direct `cached_cli_version`(无过期缓存)原样保留;`resume_or_start_with_projection`/`admission_verdict`/四家 adapter 文件未动(3.3–3.6 写面);`web/state.rs` registry 空版本 projector 现状保持(B0a2 合同即补此链,facts 携带同版本 projector 而非取 registry 空版本)。

## 二、TDD 红→绿证据

### 红(新接口初次可编译红,符合 plan §0.2)

命令:`cargo test --locked --lib anchor_t09`(cwd=worktree 根)

- 证据文件:`/tmp/anchor_t32_red_errors.txt`(20 行,19 个编译错误)
- 错误面全部为新合同符号缺失:`LcVersionCache`/`LcVersionProbe`/`LcVersionProbeKey`/`ProviderLcPrepareFacts` unresolved、`prepare_lc_facts` not a member of trait、`prepare_lc_streaming_candidate`/`reobserve_lc_prepare_facts` no method——即测试先行、接口未实现的真实红

### 绿

命令:`cargo test --locked --lib anchor_t09`

```
test cross_cutting::streaming_provider::tests::anchor_t09_command_identity_change_invalidates_live_cache ... ok
test cross_cutting::streaming_provider::tests::anchor_t09_process_restart_does_not_reuse_version_cache ... ok
test cross_cutting::streaming_provider::tests::anchor_t09_probe_timeout_or_cancel_cannot_revive_old_run ... ok
test cross_cutting::streaming_provider::tests::anchor_t09_version_ttl_boundary_singleflight_and_failures ... ok
test product::logical_codebase::provider_gateway::tests::anchor_t09_prepare_contract::anchor_t09_prepared_full_candidate_freezes_real_facts ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 4329 filtered out
```

证据文件:`/tmp/anchor_t32_green.txt`

### 五个用例覆盖的合同断言(plan Step 1 逐项)

2. `anchor_t09_command_identity_change_invalidates_live_cache`:identity/路径/supplier scope 变化→`assert_ne!` 新 key→`probe_calls_after_identity_change == before+1`;旧 key TTL 内命中不串值。
3. `anchor_t09_process_restart_does_not_reuse_version_cache`:新缓存实例同 key 必再观测(`new_cache_instance_probe_calls==1`),不复用旧 `observed_at`;旧实例自身值未串改。
4. `anchor_t09_probe_timeout_or_cancel_cannot_revive_old_run`:超时 typed `Err(VersionProbeError::Timeout)` 且再调再观测;caller 取消 typed Err、probe future 丢弃;释放 barrier 后迟到成功不入缓存(`assert_ne! "cli 9.9.9-late"`)、`!old_run_started`(取消旧 run 不复活)。
5. `anchor_t09_prepared_full_candidate_freezes_real_facts`(gateway 合同):fake 正例 facts 真实可观察(①probe 恰一次、观测时刻/键冻结);full projection 与 facts 同源同版本、`trust_digest()` 非空(readonly source 注入+Claude 不适用哨兵,不造无适用 trust 记录);run-bound audit/会话身份绑定;负例三口径 ②adapter start=0、③session child=0,durable(policy revision/digest/supersede/stream/sync/审计 sink events)前后相等;full-matching resume→`Resume`(转换面保留绑定);旧 basis(v1 指纹)不混比→`StartNew` 等待;冻结+复验矩阵:identity 变更+新观测 `1.4.1-replaced`≠冻结 `1.4.0`→`assert_frozen_version` typed `PolicyDrift{provider_version}`、零 spawn、零新 claim;未接入 adapter(CountingStreamingAdapter 默认 trait)→typed unsupported(`stderr` 含 `lc prepare facts is not supported`)、registry start 0。

### 范围化回归(全绿)

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib 'streaming_provider::'` | 34 passed / 0 failed |
| `cargo test --locked --lib provider_gateway` | 67 passed / 0 failed |
| `cargo test --locked --lib provider_registry` | 11 passed / 0 failed |
| `cargo test --locked --lib provider_availability_gate` | 7 passed / 0 failed |
| `cargo test --locked --lib provider_trust` | 31 passed / 0 failed |
| `cargo test --locked --lib`(全量) | **4327 passed / 0 failed / 7 ignored** |
| `cargo build --locked --lib` | 成功;新增代码 0 warning(遗留 20 个 warning 均为本任务之前已存在) |

## 三、提交

按 plan Step 5 显式列表(6 文件,git add 逐个列出,不用 `-A`):

```bash
git add -- src/cross_cutting/streaming_provider/mod.rs src/cross_cutting/provider_availability_gate.rs src/product/logical_codebase/provider_projection.rs src/product/logical_codebase/provider_gateway.rs src/product/logical_codebase/provider_gateway_tests.rs src/cross_cutting/streaming_provider/tests.rs
git commit -m "fix: B0a2 TTL single-flight与public fullprepare合同"
```

commit hash:`4372de28`(6 files changed, 1651 insertions(+), 6 deletions(-);删除 6 行为开工前遗留 fmt,见 §四.1)。

## 四、边界与说明

1. **provider_gateway_tests.rs 搭车 6 行 fmt**:开工前该文件已有未提交 rustfmt 改动(task13 两处 `.expect` 换行,非本任务产物);该文件在本任务独占清单内且改动不冲突,随本次提交一并入库,特此报备。
2. **parking_lot 规则偏离**:项目规则要求 lock-unwrap 处用 `parking_lot::Mutex`;本 crate 无该依赖(plan 冻结"沿现依赖,不新增锁体系";`provider_gateway_tests.rs:548` 既有注释明确此约定),故沿 crate 现状 `std::sync::Mutex`+`unwrap()`。
3. **future-prelude 规则**:新 type alias 已用 prelude `Future`;旧 `cached_cli_version`/`LegacyBridgeStart` 的 `std::future::Future` 全路径为既有代码,未动。
4. **`cfg_attr(not(test), allow(dead_code))`**:LcVersionCache 族/转换面为 B0a2 先落合同,生产消费方为 3.3–3.6 四家 adapter 与 B1/B2 caller(kimi `session.rs:79` 同款先例);测试面已真实消费,非死代码保留。
5. **三口径计数**:①version probe(fake probe 计数)、②adapter start(fake start/start_validated 计数)、③session child(fake 握手后 child 计数)在测试中分列;真实四家 kill/reap 断言归 3.3–3.6/3.7 写面。
6. **claim 计数映射**:3.2 的 gateway 层无 durable claim API(B1/2.5 写面),负例以 supersede/stream/sync/审计 sink 计数作为"零新 claim/零写"观测面,测试注释已说明。
7. **trust 维度装配语义**:注入 readonly source 时——需 workspace trust 的 provider 用 `fingerprint_facts`(缺/不可信 typed 拒,沿 B0.1 合同);无需者冻结 `lc-trust-v1:{provider}:workspace_trust_not_required` 哨兵(冻结"不适用"这一事实,非伪造记录);未注入 source 的裸 gateway 空串现状零回归(既有 31 个 provider_trust 测试全绿佐证)。
8. 独立审查(k3)与 owner 验收窗口由 controller 调度;服务 4317 未动;e2e/ 未动;临时证据均在 /tmp。

## 五、fix-loop round 1(P2:single-flight winner panic 滞留)

**finding**:Won 臂 winner panic(probe factory/future panic、锁中毒级联)时,unwind 只丢弃 winner 侧 sender 克隆;HashMap 槽内克隆使 watch 通道不关闭,同 key 后续 caller 走 Waiting 永久阻塞——无 typed 错误、无日志直至重启。

**红**(命令 `cargo test --locked --lib anchor_t09_winner_panic`,证据 `/tmp/anchor_t32_r1_red.txt`):

```
thread '…anchor_t09_winner_panic_evicts_inflight_slot_for_next_caller' panicked at …:2135:6:
next same-key caller must not hang on the panicked in-flight slot: Elapsed(())
test result: FAILED. 0 passed; 1 failed
```

复现机制:注入 panic 的 probe future 经 tokio 任务边界捕获(`JoinError::is_panic` 断言通过),随后同 key 新 caller 在死通道上等待 1s 超时——精确复现审查所述永久滞留。

**修**:`LcVersionInFlightDropGuard`(streaming_provider/mod.rs,Won 臂首行构造)——drop(含 unwind)时以与既有错误结算路径相同的 `same_channel` 语义逐出仍属 winner 的 InFlight 槽;槽内 sender 克隆随移除释放,已订阅等待者经通道关闭得到 typed `Unavailable` 而非挂起;成功路径在 guard 丢弃前已写 `Fresh` 不受影响;锁中毒走 `into_inner` 强制逐出。guard 持 owned 克隆(inner Arc/key/sender),避免借用冻结 winner 段的 `key` 移动。

**绿**(同命令):`anchor_t09_winner_panic_evicts_inflight_slot_for_next_caller ... ok`;`cargo test --locked --lib anchor_t09` 6/6 passed(原 5 + 新 1)。

**回归**:`cargo test --locked --lib 'streaming_provider::'` 35 passed / 0 failed(原 34 + 新 panic 用例;初跑曾因并行四 provider worker 半成品文件全 crate 编译阻塞,错误定位确认全部在 kimi/pi/claude 写面、本任务写面零错误,待并行方修复后复跑通过)。

**提交**:`fix: B0a2 single-flight winner panic Drop guard`(仅 streaming_provider/mod.rs + tests.rs 两文件,未越界);commit hash:`2c6c7e1a`(2 files changed, 98 insertions(+))。

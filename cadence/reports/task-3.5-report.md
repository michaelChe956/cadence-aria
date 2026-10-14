# Task 3.5(B0d)实施报告:pi_provider 真实 facts 与 full 投影接线

- 日期:2026-10-11
- Worker:ImplAnchorB035Pi(glm53-task)
- Worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 依据:plan v1.1 §Task 3.5、tasks.md 3.5、设计 §5.4(B0 完整 projection/三口径计数)+§7.1-B0a、owner 2026-10-11 `--version` TTL 裁决
- 前置:B0a2 已落库(commit `4372de28`:trait `prepare_lc_facts` 默认 unsupported+`LcVersionCache`/`LcVersionProbeKey`/`ProviderLcPrepareFacts`/gateway `prepare_lc_streaming_candidate`);本任务只做 Pi 真实现,未动 publiccore 与其它 provider 写面

## 一、交付面(Files=plan 冻结的 pi_provider 三文件;projection.rs 零改动)

| 合同名 | 位置 | 说明 |
|---|---|---|
| `PiProvider::prepare_lc_facts` 真实现 | `src/cross_cutting/pi_provider/mod.rs` | 只读观测本家 facts:①exact version 经 `LcVersionCache` 60 秒 TTL/single-flight;②approval=`PI_LC_APPROVAL_POLICY`(auto-aria-ask-extension);③MCP bundle digest=空串(pi 无可信 MCP 配置面);④boundary=`lc_boundary_plan(envelope)` 不可伪造派生;⑤同版本 `PiPolicyProjector`。全程零 extension 写/零 child/零 durable 写;守卫早于版本观测 |
| `guard_lc_pi_launch`(共用前置守卫) | 同上 | run-context guard+adapter 匹配(Pi+PiRpcV1),`prepare_lc_facts`/`start_validated` 同源调用,早于 probe/extension/child |
| `lc_version_probe_key`/`lc_executable_identity` | 同上 | key=provider+canonical 路径+可执行身份(真实 CLI 为内容 SHA-256,同路径替换即换 key)+argv+supplier scope;supplier 注入与真实 CLI 不同 key 不串值(scope 按实例原子分配,clone 共享,0 留给真实 CLI) |
| `observe_lc_version` | 同上 | supplier seam 优先;默认真实 `--version` 有界探测沿既有 `probe_pi_version_with_timeout`+`pi_policy_version`(不复制版本解析算法);失败/超时/取消不缓存。direct `start` 的无过期 `cached_cli_version` 原样保留 |
| `with_extension_cache`(extension 缓存目录 seam) | 同上 | 测试可观测面;缺省沿 `$HOME/.cache/cadence-aria`,direct 路径字节语义不变 |
| extension 准备移序 | 同上 `start_validated` | 从「guard 后、9a mismatch 判定前」移至「resume mismatch 判定之后」——mismatch 拒绝路径零 extension 写(设计 §5.4「Pi extension准备不得放到mismatch前读事实阶段」) |
| `start_validated` 消费 prepared projection | 同上 | `launch.lc_projection()`(Task 9a full-context resume 冻结候选投影)为 Some 时:先复验本家 mutable fact(exact version,TTL 观测)——漂移 typed 拒;一致则**直接消费冻结投影**为 audit 字面量(prepared 与 audit 同 full fingerprint)。裸 validate(None)沿现状以同版本 projector 现算,旧行为零变化 |
| 红测试 `anchor_t35_provider_prepare_and_audit_share_full_fingerprint` | `tests/policy_session.rs` | 真实 Pi prepare→gateway 冻结 full projection→full-matching resume(`resume_or_start_with_projection` 五参指纹)→start_validated:audit `lc_projection` 五字段(digest/capability/boundary/action/wire)与冻结 full projection 同源相等;RPC/session-id 真实同 id;spawn 前版本复验命中同一 TTL 观测(①不再增加);child 落 canonical root;extension 判定后恰写一次 |
| 红测试 `anchor_t35_real_drift_zero_session_child` | 同上 | leg A:同路径替换 CLI(0.83.0→0.84.0,identity 换 key)→重观测①+1→`assert_frozen_version` typed `PolicyDrift{provider_version}` 拒;判定后②adapter start=0、③零 child(pid marker 未出现)、extension 判定前零写、零 durable。TTL 内重观测命中①不增加。leg B:prepared resume 启动后 get_state 应答陌生 id→Err 含双 id「session NOT resumed」、child kill/reap(t09c 同款 pid 轮询)、零 provider_start(不冒称零 spawn) |
| T35 测试基建 | 同上 | 计数版 fake pi(fixture 计数①+pid marker③)、②计数装饰器(转发 `prepare_lc_facts`)、真实 PiProvider 装配的 LC gateway、对齐真实 CLI 观测的能力源(version=`pi 0.83.0`、action row digest 以真实 projector 预计算)——`admission_verdict` 的 capability↔facts 同源门在生产两处本就来自真实探测,fixture 需对齐 |

不改动:`pi_provider/projection.rs` 零改动(同版本 projector 合同 Task 4b 已落,本包直接复用);`tests/mod.rs` 不在写面内(共享 fixture 未动,新基建全部内聚 policy_session.rs);跨家组合守卫/公共汇合未写(归 3.7);gateway/publiccore 未动(3.2 已交付,本包只消费)。

## 二、TDD 红→绿证据

### 红(新接口初次可编译红,plan §0.2;同 3.2 先例格式)

命令:`cargo test --locked --lib anchor_t35`(隔离 worktree `/tmp/t35-verify`,基于 HEAD `4372de28`+本包两文件;共享 worktree 因 3.3/3.4/3.6 并行半成品无法整仓编译,plan §0.2「不在worker半成品/并行方同文件写中运行」)

- 证据文件:`/tmp/anchor_t35_red_errors.txt`
- 错误面全部为新接口符号缺失:3×E0599 `PiProvider` 无 `with_extension_cache` 方法(policy_session.rs:1359/1511/1593)——测试先行、seam 未实现的真实红

### 绿

命令:`cargo test --locked --lib anchor_t35`

```
running 2 tests
test cross_cutting::pi_provider::tests::policy_session::anchor_t35_provider_prepare_and_audit_share_full_fingerprint ... ok
test cross_cutting::pi_provider::tests::policy_session::anchor_t35_real_drift_zero_session_child ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 4334 filtered out
```

证据文件:`/tmp/anchor_t35_green.txt`

### 范围化回归(隔离 worktree,全绿)

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib pi_provider::tests::policy_session`(plan 8.1 表 3.5 行) | 14 passed / 0 failed |
| `cargo test --locked --lib pi_provider::` | 68 passed / 0 failed |
| `cargo test --locked --lib lcg_t09` | 19 passed / 0 failed |
| `cargo test --locked --lib anchor_t09`(3.2 prepare 合同) | 5 passed / 0 failed |
| `cargo test --locked --lib tool_policy_audit` | 23 passed / 0 failed |
| `cargo test --locked --lib streaming_provider::tests` | 34 passed / 0 failed |
| `rustfmt --edition 2024 --check` 两改动文件 | 零偏差 |

旧合同保持的直接证据:同文件 t04(无通用策略 Executor 仍全 audit/原生会话)、t09a(LC resume audit 门零 child)、t09c(缺/错 id 不 fresh)、direct 策略会话族(预生成 id/supersede/append 失败 kill 链/进程内缓存计数)全部原绿。

按 plan Step 5 显式列表(projection.rs 零改动,git add 无操作,列出以完全对齐 plan):

```bash
git add -- src/cross_cutting/pi_provider/mod.rs src/cross_cutting/pi_provider/projection.rs src/cross_cutting/pi_provider/tests/policy_session.rs
git commit -m "fix: B0d pi_provider真实facts与full投影接线"
```

commit hash:`be140540`(2 files changed, 859 insertions(+), 124 deletions(-);其中 mod.rs 含开工前遗留 fmt 归位,见 §四.2)。

## 四、边界与说明

1. **隔离验证理由**:共享 worktree 在本包窗口内因 3.3/3.4/3.6 并行半成品(如 codex 通报的 kimi E0624/E0277、claude E0507)整仓编译失败,红绿/回归均在 `/tmp/t35-verify`(`git worktree add --detach 4372de28`+仅同步本包两文件+web/dist 静态产物)执行,证据不含任何并行方改动。
2. **mod.rs 搭车 fmt**:开工前 `mod.rs` 已有未提交 rustfmt 归位改动(19+/29-,import 排序与既有代码换行,非本任务语义产物);该文件在本包独占清单内且 fmt --check 零偏差,随本次提交一并入库,特此报备(3.2 报告 §四.1 同款先例)。
3. **admission_verdict 同源门与 fixture**:prepare 链中 `capability.version == projection.exact_version`、`capability.action_capability.projection_digest == projection.capability_projection_digest()` 为 gateway 冻结门(3.2/1.2 合同)。共享 `LcStaticCapabilitySource` 的 version/digest 是占位字面量,与真实 CLI 观测天然不同源;故本包测试内聚实现 `T35StaticCapabilitySource`(version 对齐 `pi 0.83.0`、action row digest 以真实 `PiPolicyProjector` 预计算,Coding canonical 形态)。capability 摘要不随单次 role/target/config 变化已由 lcg_t04 断言组证明。未改 tests/mod.rs(写面纪律)。
4. **裸 validate 路径零变化**:`launch.lc_projection()==None`(既有 t04/t09a/t09c 与 direct 全部测试)时 projector 现算、TTL 版本观测替代旧无过期缓存(仅 LC validated 路径);supplier 测试 seam 语义保持(每实例独立 scope,不与真实 CLI 串值)。
5. **version TTL 合同分工**:59.999s/60s 边界、barrier single-flight、失败不缓存等缓存本体合同由 3.2 的 `anchor_t09`/streaming_provider 测试覆盖(本次回归全绿);本包贡献 provider 侧消费证明(prepare ①恰一次、reobserve/TTL 命中 ①=0、identity 变更 ①=1、start_validated 同 key 命中不再观测)。
6. **三口径计数**:①fixture 计数文件;②T35CountingAdapter(start/start_validated);③fixture pid marker。错 id leg 的 kill/reap 为「已启动 child 后 runtime 失败」口径(③=1 后被收口),与 drift mismatch 的 ②③=0 分列,未冒称零 child(owner 裁决口径)。
7. **报告文件不随包提交**:沿 task-3.2-report.md 先例(untracked,由 controller 统一文档线);服务 4317 未动;e2e/ 未动;临时证据均在 /tmp(/tmp/anchor_t35_red_errors.txt、/tmp/anchor_t35_green.txt、/tmp/t35-verify)。
8. 独立审查(k3)与 owner 验收窗口由 controller 调度;findings 回本包 fixloop。

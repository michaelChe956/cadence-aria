# Task 3.4(B0c)实施报告:codex_provider 真实 facts 与 full 投影接线

- 日期:2026-10-11
- Worker:ImplAnchorB034Codex(glm53-task)
- Worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 依据:plan v1.1 §Task 3.4、tasks.md 3.4、设计 §5.4(版本缓存合同)+§7.1-B0a1/a2、owner 2026-10-11 `--version` 裁决
- 前置:B0a2(3.2,commit `4372de28`)的 `prepare_lc_facts` trait/TTL 合同与 `prepare_lc_streaming_candidate`/`reobserve_lc_prepare_facts` gateway 面;本包只消费、未改公共面(publiccore 零触碰)

## 一、交付面(Interfaces 冻结逐项)

| 合同名 | 位置 | 说明 |
|---|---|---|
| `CodexProvider { lc_version_cache, lc_supplier_scope }` 新字段 | `src/cross_cutting/codex_provider/mod.rs` | 实例级 `LcVersionCache`(60 秒 TTL/single-flight);supplier scope 每次注入领取唯一序号(静态 `LC_SUPPLIER_SCOPE_SEQ`,0 保留真实 CLI 面),测试 supplier 与真实 CLI 不串值 |
| `CodexProvider::lc_version_probe_key/lc_executable_identity` 私有 | 同上 | key=provider+命令路径+可执行身份(size+mtime_ns,二进制替换即换 key;supplier 面为稳定标记)+`--version` argv+scope |
| `CodexProvider::observe_lc_exact_version` 私有 | 同上 | LC 权威版本观测唯一入口:`cached_lc_cli_version`(supplier seam 优先,默认 `probe_codex_version` 有界探测);prepare 与 spawn 前复验共用(不双矩阵);direct 无过期 `cached_cli_version` 原样不动 |
| `CodexProvider::lc_version_unavailable` 私有 | 同上 | 版本不可得 fail-closed 映射(Coder 危险形态→危险稳定码;原 start 内闭包提取,prepare/start 同源) |
| `CODEX_LC_PREPARED_VERSION_DRIFT` 稳定码 | 同上 | spawn 前 facts 复验:冻结 full 投影 exact version ≠ 当前 TTL 观测 → child 前拒绝 |
| `StreamingProviderAdapter::prepare_lc_facts`(Codex 真实现) | 同上 | guard(role/危险门)先于 ①观测;envelope 派生 boundary;approval=`lc_approval_policy` 冻结映射;MCP 空串(与既有 adapter 侧投影输入同源);同版本 `CodexPolicyProjector`;全程零写零 spawn |
| `start_lc_validated_with_phase` 增加 `frozen_lc_projection` 参数 | 同上 | gateway full 上下文路径(`resume_or_start_with_projection` Resume 携带的冻结候选投影)直接消费:sandbox/audit 与 prepared full 投影同源+版本复验;None(裸 validate)沿 adapter 侧自建投影,既有测试锁定分支零变化;步骤 5 版本观测切换为 TTL 入口 |
| `start_validated` 接线 `launch.lc_projection().cloned()` | 同上 | trait 实现透传冻结投影;`start_lc_validated`(cfg(test) 4 参)签名零变化传 None |
| `lc_approval_policy` 升 `pub(crate)` | `src/cross_cutting/codex_provider/projection.rs` | prepare facts 的 approval fact 单一来源;仅可见性变化,投影/digest 零变化 |

不改动:direct `start`(策略会话)字节语义原样(supplier/无过期缓存路径未动);`tests/streaming.rs` 不在写面未触碰(其 3 行 M 为开工前遗留 fmt,不 stage);gateway/publiccore 未触碰(跨家汇合归 3.7)。

## 二、TDD 红→绿证据

### 红(真实行为红,非编译红、非 default-unsupported 绕过)

因共享树被并行 worker 半成品阻塞,红绿均在隔离验证 worktree `/tmp/t34-red`(**干净基线 4372de28** + 仅本包四文件)取证,不占共享 crate 编译。

命令:`cargo test --locked --lib anchor_t34`(cwd=/tmp/t34-red)

- 证据文件:`/tmp/anchor_t34_red.txt`
- 过滤命中 2 个新用例,双双 FAIL 于默认 trait 拒绝:`streaming provider lc prepare facts is not supported by this adapter`(typed unsupported 直接挡住正例,符合"真实 adapter 正例可观察 prepare,不能默认 unsupported 虚绿"的反面锚定)

### 绿(基线 4372de28)

命令:`cargo test --locked --lib anchor_t34`

```
test cross_cutting::codex_provider::tests::approval_policy::anchor_t34_provider_prepare_and_audit_share_full_fingerprint ... ok
test cross_cutting::codex_provider::tests::bridging::anchor_t34_real_drift_zero_session_child ... ok
test result: ok. 2 passed; 0 failed
```

证据文件:`/tmp/anchor_t34_green.txt`

### 绿(新 HEAD 6109e35f,含 2c6c7e1a TTL Drop guard/be140540 pi/6109e35f kimi)

同命令重跑:2 passed(codex 65 passed 全绿,见下),证据 `/tmp/anchor_t34_green_newhead.txt`、`/tmp/anchor_t34_regression_newhead.txt`。

### 两用例覆盖的合同断言(plan Step 1 逐项)

1. `anchor_t34_provider_prepare_and_audit_share_full_fingerprint`(approval_policy.rs):真实 gateway 全链(capability/policy/target/registry/gate 组装,先例同 engine 侧 validated_input fixture)——supplier ①观测恰一次、facts 冻结本家 version/probe key;受限 approval="on-request"、sandbox="read-only"(`assert_ne!` danger-full-access)、protocol cwd=root 映射;prepare 零 durable 写/零 child;full-matching resume→Resume;`start_validated` 消费 prepared 投影真实启动:thread/resume 确认同 ID、wire sandbox/approvalPolicy/cwd 与进程 cwd 全部同源自投影;**`assert_eq!(prepared_projection_digest, audit_projection_digest)`**(provider_start.lc_projection 与 prepared full 投影逐字节同源);恰一次 provider_start。
2. `anchor_t34_real_drift_zero_session_child`(bridging.rs):真实 CLI 面(combined fixture,`--version` 回显+app-server 同体)——prepare#1 冻结 v1;full-matching resume 真实启动确认同 ID 且 audit 同源;错 ID 应答(kill/reap+「session NOT resumed/not a zero-spawn refusal」如实记录+零新 provider_start,`assert!(wrong_native_child_killed_and_reaped)` 经 pid marker 终止断言);重写命令文件=identity 变化→TTL 换 key 重观测 v2→`assert_frozen_version` typed `PolicyDrift{provider_version}`;adapter 纵深(陈旧冻结投影直启)以 `codex_lc_prepared_version_drift` child 前拒绝:**`mismatch_adapter_starts==0`、`mismatch_children==0`**(独立 spawn marker+provider_start 计数,三口径分列:①probe/观测、②provider_start 落盘、③spawn marker)。

### 范围化回归(全绿,新 HEAD)

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib 'codex_provider::'` | 65 passed / 0 failed(含既有 t05/t09a/t09b/direct/Coding 合同) |
| `cargo test --locked --lib 'streaming_provider::'` | 35 passed / 0 failed(TTL 合同含 2c6c7e1a Drop guard) |
| `cargo test --locked --lib provider_gateway` | 67 passed / 0 failed |
| `cargo build --locked --lib`(隔离基线) | 成功;codex_provider 新代码 0 warning(grep 验证) |

## 三、提交

按 plan Step 5 显式列表(4 文件逐个 add,不用 `-A`;`tests/streaming.rs` 遗留 fmt 不带):

```bash
git add -- src/cross_cutting/codex_provider/mod.rs src/cross_cutting/codex_provider/projection.rs src/cross_cutting/codex_provider/tests/approval_policy.rs src/cross_cutting/codex_provider/tests/bridging.rs
git commit -m "fix: B0c codex_provider真实facts与full投影接线"
```

commit hash:`10f1b529`(4 files changed, 1083 insertions(+), 114 deletions(-);其中 mod.rs 含开工前遗留 fmt 约 88 行搭车,见 §四.3)。

## 四、边界与说明

1. **共享树 stash 事故**:3.3 worker 误用 `git stash` 致全树 WIP 一度只剩 stash;其已用快照 diff 恢复 41 文件。本人四文件恢复后与隔离验证副本逐字节核对一致(diff 校验+晚期改动逐项抽查:Ordering 导入/闭包块形态/capability 摘要助手/JSON 引号修正/candidate.input 克隆),且 controller 独立复核确认;无内容回退。红绿证据全程在隔离 worktree 取得,不受该事故污染。
2. **红绿隔离策略**:共享树先后被 pi/kimi/claude 半成品与 stash 事故阻塞,红(基线 4372de28)与绿(4372de28+新 HEAD 6109e35f 两轮)均在 `/tmp/t34-red`(仅含本包四文件的干净 checkout)取证;提交内容与取证副本逐字节一致。
3. **mod.rs 搭车 fmt**:开工前 `codex_provider/mod.rs` 已有他人未提交 rustfmt 改动(约 88 行,纯排版);该文件在本任务独占清单内,随本次提交一并入库,特此报备(先例:task-3.2-report §四.1)。
4. **TTL 与 supplier**:supplier seam 走 TTL 包装(sync supplier 包成 probe future),同实例同 key 60 秒内命中不重复观测;supplier 版本翻新属 key 内漂移,owner 裁决接受 ≤60 秒迟钝;真实漂移负例用命令 identity 变化(重写 fixture→size/mtime 变→换 key)驱动,非借 supplier 翻新。
5. **capabitlity 行对齐**:fixture `AnchorT34CapabilitySource` 的 `projection_digest` 以真实 `CodexPolicyProjector` 同源计算(`anchor_t34_capability_profile_digest`,profile 摘要与单次 role/cwd/target 无关),满足 `admission_verdict` 第 3 步对齐门——非硬编码假值。
6. 独立审查(k3)与 owner 验收窗口由 controller 调度;服务 4317 未动;e2e/ 未动;临时证据均在 /tmp(`/tmp/anchor_t34_red.txt`、`/tmp/anchor_t34_green*.txt`、`/tmp/t34-red` 用毕可清)。
7. 跨家组合守卫(四家 full path 不 unsupported、audit 全同源矩阵)归 3.7;本包测试未写任何 Pi/Claude/Kimi 专属负例。

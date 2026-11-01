# Task 3.3(B0b)实施报告:claude_code_provider 真实 facts 与 full 投影接线

- 日期:2026-10-11
- Worker:ImplAnchorB033Claude(glm53-task)
- Worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 依据:plan v1.1 §Task 3.3、tasks.md 3.3、设计 §7.1-B0a("readonlytrust fingerprint facts→asyncprepare/opaquecandidate/full复验两段;四provider跟进")
- 前置:3.2 已落库(`4372de28`+`2c6c7e1a`:`prepare_lc_facts` trait 默认 unsupported、`LcVersionCache` 60s TTL/single-flight、`PreparedLcStreamingCandidate`/`assert_frozen_version`、B0.1 只读 trust facts)——本任务只消费该合同,未改 public core 任何文件

## 一、交付面(Interfaces 冻结逐项)

**Consumes(3.2 合同,零改动):** `ProviderLcPrepareFacts`(crate 内构造/getter)、`LcVersionCache::cached_lc_cli_version(key, probe, cancel)`、`LcVersionProbeKey/LcVersionProbe/LcVersionObservation`、gateway `prepare_lc_streaming_candidate`/`resume_or_start_with_projection`(`ValidatedSessionLaunchPolicy::lc_projection` 冻结投影)、`guard_lc_validated_launch`。

**Produces(本包写面):**

| 交付 | 位置 | 说明 |
|---|---|---|
| `ClaudeCodeProvider::prepare_lc_facts` 真实现(trait 覆写) | `src/cross_cutting/claude_code_provider/mod.rs` | guard 先行(早于版本 probe/child)→ adapter 匹配错位 typed 拒(`ClaudeCode`+`ClaudeCodeCliV1`)→ 只读观测 facts;全程零 spawn/零写;默认 unsupported 自此在本 adapter 不可达 |
| `observe_lc_facts(&launch, cancel)`(私有单一观测实现) | 同上 | `prepare_lc_facts` 与 `start_validated` 共用同一提取(同源):supplier seam 优先;默认真实 CLI `--version` 有界探测(`probe_claude_version`,5s)经**实例内** `LcVersionCache`(60s TTL/single-flight,失败/取消不缓存);boundary=envelope 派生 `lc_boundary_plan`(与 argv 同源);projector=同版本 `ClaudePolicyProjector`(+root-recipe 相位);approval=`CLAUDE_LC_APPROVAL_POLICY`;MCP bundle digest 空串(Claude LC argv 无 MCP bundle 注入,与既有投影输入一致);不从 registry/capability 空 version 取证 |
| probe key 维度 | 同上 | provider=`ProviderName::ClaudeCode`、canonical executable 路径、`lc_executable_identity`(文件长度+mtime_ns 稳定文本;元数据不可得退化路径文本)、`argv=["--version"]`、`supplier_scope`(真实 CLI 恒 0;`with_version_supplier` 按进程级 AtomicU64 单调分配)——重写/替换同一 CLI 即换 key 再观测,不命中旧 TTL 缓存 |
| `start_validated` 消费 prepared projection | 同上 | 原"supplier/旧 `cached_cli_version` 探测+自投影"块替换为 `observe_lc_facts` 同源提取;`launch.lc_projection()` 为 Some(完整上下文 resume 判定)时**直接消费冻结 full projection 作审计投影**,并在 session child 前以当前 facts 复验 exact version(同 key TTL 内同观测),漂移 typed 拒(`frozen ... disagrees with observed facts ... drift before spawn; zero children`)、零 child;None(裸 validate/legacy fixture)路径按原语义以 facts 同版本 projector 自投影(投影输入逐字段等价:approval/MCP/boundary 改从 facts 取,值不变) |
| `ClaudeCodeProvider` 字段 | 同上 | 新增 `version_supplier_scope: u64`、`lc_version_cache: LcVersionCache`(实例内;`#[derive(Clone)]` 原样成立) |

**不改动:** direct `start` 的版本链(旧无过期 `cached_cli_version`)与 argv 字节语义原样;`projection.rs` 零改动(projector 合同本就满足:构造冻结实测 version、空串 fail-closed、provider owner 不变——独占清单含该文件但不需修改);`streaming_provider`/`provider_gateway` 等 public core 未动;Pi/Kimi/Codex 文件未动(跨家组合归 3.7)。

## 二、TDD 红→绿证据

### 红(实现前,命令 `cargo test --locked --lib anchor_t33`,cwd=worktree 根)

```
running 2 tests
test ...anchor_t33_real_drift_zero_session_child ... FAILED
test ...anchor_t33_provider_prepare_and_audit_share_full_fingerprint ... FAILED
---- stdout ----
gateway freezes the real claude full candidate: Adapter(ProviderAdapterError { code: ProviderExecutionFailed,
  details: "provider command exited unsuccessfully", stdout: "",
  stderr: "streaming provider lc prepare facts is not supported by this adapter", ... })
test result: FAILED. 0 passed; 2 failed; 4342 filtered out
```

证据:`/tmp/anchor_t33_red.txt`。过滤命中 2 新用例、失败点即 3.2 默认 unsupported——真实 facts 行为缺失的实质红(非 0 tests、非 default-unsupported 绕过正例)。

### 绿(同命令)

```
test ...anchor_t33_provider_prepare_and_audit_share_full_fingerprint ... ok
test ...anchor_t33_real_drift_zero_session_child ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 4342 filtered out
```

证据:`/tmp/anchor_t33_green.txt`。

### 两用例覆盖的合同断言(plan Step 1 逐项,仅 Claude 专属面)

1. `anchor_t33_provider_prepare_and_audit_share_full_fingerprint`:
   - 真实 CLI `--version` 驱动(无 supplier):gateway 经 registry gated wrapper 调真实 `prepare_lc_facts` 冻结 opaque 候选;facts 可观察——`exact_version==T33_CLI_VERSION`、probe key provider/argv/`claude-cli:` 前缀 executable identity、approval 与 argv 投影同源、`version_observed_at<=now`;
   - full-matching resume(`resume_or_start_with_projection`,五参指纹同源重算)→ Resume(validated 携带冻结 full projection);
   - `start_validated` 消费冻结投影真实 spawn:原生 `--resume` 同 ID 确认(`native_session_id==requested`)、流式完成;
   - **审计同源**:`prepared_projection_digest == audit_projection_digest` 逐字相等;`provider=="claude-code"`、`adapter_dialect=="claude-stream-json"`、`provider_version==CLI --version`、argv 含 `--resume`+id+`CLAUDE_LC_ALLOWED_TOOLS`(工具 translator 同源);
   - 错 ID(同版本冻结投影):child 后 runtime 失败——错误命名两侧 id+"session NOT resumed"、零新 provider_start、`wrong_native_child_killed_and_reaped==true`(pid 登记+kill/reap 轮询观测,不伪称零 child)。
2. `anchor_t33_real_drift_zero_session_child`:prepare 冻结 v1 → 重写同一 CLI(`--version` 改答 v2;长度+mtime 变 → executable identity 换 key 再观测,不命中旧缓存)→ `start_validated` child 前 facts 复验 typed 拒(错误含 "drift" 与两侧版本);`mismatch_children==0`(LC_CWD_MARKER 不存在)、`mismatch_adapter_starts==0`(零 provider_start)、sink 全空(零写)。

### 范围化回归(全绿)

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib claude_code_provider` | **58 passed / 0 failed / 1 ignored**(live_claude 需真实 CLI,既有 ignore) |
| `cargo test --locked --lib provider_gateway` | 67 passed / 0 failed |
| `cargo test --locked --lib 'streaming_provider::'` | 35 passed / 0 failed |

证据:`/tmp/anchor_t33_regress_claude.txt`、`/tmp/anchor_t33_regress_gateway.txt`。旧同文件行为(t09a resume 审计门、t09b 同 ID/错 ID/缺 ID、t04 无策略角色审计、t10 root-recipe 相位、direct 策略会话组)全部保留。

## 三、提交

按 plan Step 5 显式列表(projection.rs 在独占清单内但本次零改动,add 为 no-op 一并列入):

```bash
git add -- src/cross_cutting/claude_code_provider/mod.rs src/cross_cutting/claude_code_provider/projection.rs src/cross_cutting/claude_code_provider/tests/policy_session.rs
git commit -m "fix: B0b claude_code_provider真实facts与full投影接线"
```

commit hash:见下方"提交记录"补记(提交后回填)。

## 四、边界与说明

1. **独占清单外的共享树事件(已闭环)**:核对既有 warning 时误用 `git stash` 做快照对照,pop 与 codex 包并发写冲突中止,全树未提交改动一度只剩 stash。已按 path 精确恢复全部 41 文件(排除 codex mod.rs——保留其更新现行版),经 Pi/Kimi/两家 reviewer 自查确认零损失后 drop stash。事故全程未触碰任何提交历史、未 push。后续对共享树不再使用 stash 类操作。
2. **并行编译干扰**:本包红/绿运行多次被并行 provider 包半成品文件的全 crate 编译错误阻塞(错误均在 codex/kimi/pi 写面);已与 codex 双向通报修复(E0507 双重 Arc 包装 / Ordering 导入 / probe 闭包装箱),最终红绿与回归均在全树可编译态下取得。
3. **既有 fmt 搭车报备**:`claude_code_provider/mod.rs` 开工前已有未提交 rustfmt 改动(`SessionTailExit` match 臂换行,3 行);该文件在本任务独占清单内且与本任务改动不冲突,随本次提交一并入库。
4. **新增代码 0 warning**:本任务删净自身引入的死导入(`ProviderPolicyProjector`);`policy_session.rs:946` 的 unused imports(`LcProviderStartAudit` 等)为 HEAD 既有(已对照 `git show HEAD:` 验证,t09a 体内为全限定调用),非本任务产物,未顺手清理。
5. **supplier_scope 语义**:supplier 路径不进 TTL 缓存(同步 seam,每次直调,与既有 `start_validated` supplier 优先语义一致);scope 只用于 probe key 维度区分 supplier 实例与真实 CLI,防 facts 键碰撞误导审计。
6. **drift 拒绝与 Task 9a 门顺序**:版本漂移复验先于 9a 存档审计门与 spawn;漂移场景零写(不 supersede——旧 run 记录未被消费,归 B1/C 的恢复卡语义),错误显式记录"未启动任何 child"。
7. **跨家组合守卫归 3.7**:本包未写任何 Pi/Kimi/Codex 判据或文件;`web/state.rs` 生产装配(projector 第四参现状)未动,归 B0f/后续任务。
8. 独立审查(k3)与 owner 验收窗口由 controller 调度;服务 4317 未动;e2e/ 未动;临时证据均在 /tmp。

## 提交记录

- commit:`3f30e806`(2 files changed, 713 insertions(+), 66 deletions(-);projection.rs 在独占清单内但零改动,未入 diff;按 plan Step 5 显式列表 add,未用 `-A`)

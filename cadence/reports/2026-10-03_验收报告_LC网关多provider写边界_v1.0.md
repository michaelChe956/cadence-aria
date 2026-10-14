# 验收报告:LC 网关多 provider 写边界(Task 6c)v1.0

- 日期:2026-10-04(现场探测时间 2026-10-03T19:40–19:42 UTC)
- worktree:`.worktrees/feat-b-0808-add-monorepo`,BASE=`3f2c5161`,HEAD=`b9480120`
- 交付:Task 6c(probe 模块+四家真实写边界 probe 与证据签发);前置 6a launcher/probe 通道、2c shape validator、四家 projection(4/5)全部在场
- 本报告与全部 evidence 工件**写盘不入 git**

## 1. 交付摘要

| 项 | 内容 |
|---|---|
| Create | `src/product/logical_codebase/provider_boundary_probe.rs`(`ProviderBoundaryProbe::run` 冻结签名 + `BoundaryFixture` + 内嵌测试) |
| Modify | `src/product/logical_codebase/mod.rs`(单 owner 接线:`pub mod` + `pub use`) |
| commits | `3f03c3a3`(阶段1 RED 失败断言)→ `f02114bd`(阶段2 GREEN,Step5 逐字命令)→ `b9480120`(fixture 成员随 root 布局修复) |
| 测试命令 | `cargo test --locked --lib lcg_t06` → **19 passed / 0 failed / 4 ignored(live 门)**;`cargo test --locked --lib cross_target` → 14 passed |

签发语义:`write_boundary=Confirmed` 只由「完整防护(6a launcher 实测 bwrap+userns)+ 真实正探针成功落盘 + 真实负探针全部带证据拒绝 + 受保护面 pre==post 零漂移 + 观测完整未超预算 + 证据工件落盘」签发。缺防护/版本漂移/观测缺失(哨兵/FIFO)/超预算/工件未落盘一律 Unknown;负向落盘/正向失败/漂移为 Denied。预算沿 HEAD:快照 20_000 条/64 MiB(`RootRecipeSnapshotBudget::DEFAULT`)、观测面 8192B(`COMPACT_INVENTORY_HARD_BUDGET_BYTES`),超限拒签不截断。

## 2. 四家现场结果(每家 Coding+Planning 两个 action)

运行方式:每家串行 `LC_GATEWAY_E2E=1 cargo test --locked --lib lcg_t06_live_boundary_probe_<家> -- --ignored --nocapture`(不并跑 Cargo)。证据目录:`cadence/reports/lc-gateway-multi-provider/<族>/boundary/<action>-<时间戳>/`(含 `evidence.json` 与 `shape-validation.json`)。

| 家 | CLI 版本(actual `--version`) | write_boundary | Coding 正/负探针 | Planning 正/负探针 | 2c shape 校验 |
|---|---|---|---|---|---|
| claude-code | `2.1.283 (Claude Code)` | **Confirmed**(两 action) | 2 正 / 10 负 | 1 正 / 8 负 | passed(可导入 2d) |
| codex | `codex-cli 0.155.1` | **Confirmed**(两 action) | 2 正 / 10 负 | 1 正 / 8 负 | passed(可导入 2d) |
| pi | `0.86.1` | **Confirmed**(两 action) | 2 正 / 10 负 | 1 正 / 8 负 | passed(可导入 2d) |
| kimi-code | `2.0.2` | **Confirmed**(两 action) | 2 正 / 10 负 | 1 正 / 8 负 | passed(可导入 2d) |

环境:`Linux 6.18.25-x64v3-xanmod1` x86_64;`/usr/bin/bwrap` `bubblewrap 0.11.2` + user namespace(实测可用)。

### 最终证据会话(修复布局后;每会话 evidence.json + shape-validation.json)

- claude-code:`boundary/coding_target_write-20261003T194051Z/`、`boundary/planning_read_only-20261003T194051Z/`
- codex:`boundary/coding_target_write-20261003T194109Z/`、`boundary/planning_read_only-20261003T194109Z/`
- pi:`boundary/coding_target_write-20261003T194130Z/`、`boundary/planning_read_only-20261003T194130Z/`
- kimi-code:`boundary/coding_target_write-20261003T194148Z/`、`boundary/planning_read_only-20261003T194148Z/`

(同目录下 `*-20261003T1937*`~`1938*` 为布局修复前的首轮现场记录,成员负向当时为 tmpfs 遮蔽 ENOENT;保留作历史,验收以 `1940*`+ 会话为准。)

## 3. 证据形态(actual argv / mount / errno / 工具 channel / 版本)

每份 `evidence.json`(`lc-boundary-probe-evidence-v1`)记录:

- **actual argv**:probe spawn 与受控 commit 的完整 bwrap argv(`launcher_argv`/`controlled_commit.argv`),payload 与逐 attempt 明细另列;
- **mount 序列**(顺序即安全语义):`--ro-bind / /` → `--proc/--dev/--tmpfs /tmp` → `--die-with-parent` → root 只读原路径 → target rw bind(原路径)+ 授权 git-dir bind → provider 运行时窄面(`~/.claude|~/.codex|~/.pi|~/.kimi-code`,只绑存在的)→ 后置只读遮蔽 target `.git` 指针与 `.aria` → `--chdir <root>`;
- **errno**:每条负向拒绝带真实报文,如 `sh: line 3: …/lc-root/rogue-builtin-probe: Read-only file system`、`aria-terminal: …/member-a/rogue-probe: Read-only file system`、`aria-mcp-inner: …/member-a/.git/rogue-probe: Read-only file system`、`aria-extension: …/member-b/rogue-probe: Read-only file system`、target `.git` 指针 `Read-only file system`;
- **工具 channel**:Builtin/Terminal/Mcp/Extension 四通道全覆盖(Coding 10 负、Planning 8 负;MCP 为两层后代模拟);
- **版本**:provider 族名、CLI 程序名、`cli_version_argv`(如 `["claude","--version"]`)、`cli_exact_version`(与 projection.exact_version 比对,漂移拒签);
- **D4**:受保护面 pre/post 快照 digest 相等,`drift=none`;观测面 `evidence_bytes≈1.1–1.4KB / 8192B`、`complete=true`、快照预算 20_000/64MiB;
- **正向**:provider 运行时窄面写(两 action)与 target 写 + 受控 `git commit`(subject `aria-boundary-probe-commit`,宿主复核)真实落盘。

## 4. 缺陷与修复记录

1. **fixture 成员布局**(commit `b9480120`):首轮成员目录置于 root 兄弟位置,`--tmpfs /tmp` 遮蔽下沙箱内成员不可见,负向探针得到 ENOENT 而非真实 EROFS——证据不足以证明成员写面受保护。修复为成员随 root 子目录(与真实 LC 拓扑及 6a 先例同构)后重跑四家,成员负向均为真实 `Read-only file system` 拒绝。
2. **resume 段时序**(commit `29bc5747`):首轮 pi 现场被 D4 门正确拦截(`.git/index` +1)——宿主侧 LLM 会话在 cwd git 仓库做项目探测会物化 index,不是沙箱写边界违规。修复:resume 段前置到 pre 快照之前,D4 只度量沙箱窗口;四家以最终代码统一重跑。
3. 无其它阻断;四家 CLI 均可用,无 Unknown/Denied 结果。

## 5. 后续(E9:6c → 2d)

四家两 action 的 evidence 已通过 2c `validate_probe_shape` 三方一致性校验(shape-validation.json),标记**可导入**:2d `record_verified_probe` 消费 `ProviderCapabilityProbeService` + 本证据将 `write_boundary` 写为 durable Confirmed。真实五阶段 fresh/resume E2E 与越界写验收复用本 probe 的命令/fixture/证据形态,归 Task 10/11。

## 6. resume 面验收(r24 工单,2026-10-05)

**probe resume 段设计**(commits `ba16a706` RED→`1b356da4` GREEN→`29bc5747` 时序修正):`BoundaryFixture::with_resume(ResumeProbeSpec)` 附规格后,`ProviderBoundaryProbe::run` 在写边界探测之外执行 resume 段——宿主侧(LLM 会话需真实 HOME/网络/登录态)fixture root 内「真实 launch(记录 native session id)→同 id 真实 resume→native id 确认同 id→错 id 负探针(不得回显原 id)」。四家通道(现场实测口径):claude `-p --output-format json`+`--resume <id>`;codex `exec --json`+`exec resume <id> --json`(thread.started.thread_id);pi `--mode json -p`+`--session-id <id>`(产品 adapter 同通道);kimi `-p --output-format stream-json`+`-S <id>`。结果写入 evidence.json `resume` 段(launch/resume/错 id 的完整 argv、exit、native id、应答摘录);`ProviderBoundaryProbe::artifact_resume_state(artifact_ref)` 签发三态:同 id 确认+错 id 拒绝→Confirmed;CLI 不支持/同 id 续开 id 不符/错 id 回显原 id→Denied(带真实错误);超时/未探测→Unknown。resume 面与写边界正交,不影响写面签发。2c `validate_probe_shape` 对 resume=Confirmed 的行兼容(实测通过),2d `record_verified_probe` 按行导入 resume 格。

**四家 resume 验证结果**(最终会话,含 shape-validation.json):

| 家 | 通道 | launch native id | 同 id resume | 错 id 负探针 | resume 列 |
|---|---|---|---|---|---|
| claude-code 2.1.283 | claude_print_json | session_id 记录 | 同 id 确认 | exit 1「No conversation found」 | **Confirmed** |
| codex 0.155.1 | codex_exec_json | thread.started.thread_id | 同 id 确认 | exit 1「no rollout found」 | **Confirmed** |
| pi 0.86.1 | pi_session_id | session 事件 id | 同 id 确认 | exit 0 新建**另一** id(≠原 id) | **Confirmed** |
| kimi-code 2.0.2 | kimi_stream_json | 事件 session_id | 同 id 确认 | exit 1「Session … not found」 | **Confirmed** |

最终证据会话(20261005T17*):claude-code `171139/171148`、codex `171215/171247`、pi `171033/171038`、kimi-code `171104/171109`;单元层以可编程假 CLI(宿主真实进程)锁定 launch→resume→错 id 全链路(round-trip Confirmed/id 绑定失效 Denied/缺省未探测 Unknown)。回归:`lcg_t06` 22 passed、`lcg_t02` 21 passed。

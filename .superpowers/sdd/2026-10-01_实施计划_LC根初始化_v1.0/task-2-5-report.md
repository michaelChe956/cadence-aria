# Task 2.5 报告：cwd/target 分离的跨层字段合同

状态：DONE　|　worktree: feat-b-0808-add-monorepo　|　基线 HEAD: 414401a5

## 交付合同（四层字段）

| 层 | 字段 | 形态 | 兼容策略 |
|---|---|---|---|
| `SessionLaunchRequest`（provider_gateway.rs） | `working_directory: PathBuf` | **必填** | 13 处字面量逐一补值（映射 target worktree，零行为变化）；`planning()` 构造器内部派生 `target.worktree`（17 处调用零改动） |
| `SessionPolicyEnvelope`（policy.rs） | `working_directory: PathBuf` | 必填 + `#[serde(default)]` | `new()` 增第 4 参（target 之后）；存量 JSON 无键 → `PathBuf::default()`（与 authority_root 同策略） |
| `SessionResumeFingerprint`（provider_gateway.rs） | — | digest 新维度 | `from_envelope` 哈希 `envelope.working_directory`：cwd 漂移 ⇒ fingerprint 不等 ⇒ `resume_or_start` supersede |
| `StreamingProviderInput`（streaming_provider/mod.rs） | `working_directory: Option<PathBuf>` | 新字段 | `effective_working_directory() -> &Path`：Some→root，None→回填 `working_dir`；legacy bridge 跨层透传 |
| `AdapterInput`（protocol/contracts.rs） | `working_directory: Option<PathBuf>` | 新字段 + `#[serde(default)]` | `effective_working_directory() -> Option<PathBuf>`：Some→root，None→回填 `worktree_path` |

gateway 消费（仅 effective 口径，**cwd==target 复验等式未动**——拆除归 2.2/2.8）：
- `run_sync`：cwd 取 `input.effective_working_directory()`（优先新字段，否则回填 target；`worktree_path` 仍是 target）
- `start_streaming`：复验 cwd 取 `input.effective_working_directory()`
- `revalidate_before_spawn` 5b 重建请求携带 `envelope.working_directory`（resume 同 cwd，设计 §2.4）

## 构造点 grep 计数（词边界精确匹配）

| 类型 | 计数 | 处置 |
|---|---|---|
| `StreamingProviderInput {` 字面量 | 81 行匹配（含 4 行 struct/impl 定义）≈77 构造点 | 45（ast-grep 裸名）+3（全限定名）机械 `working_directory: None`；bridge 透传 1；fake_for_test None 1；新测试显式值 2 |
| `AdapterInput {` 字面量 | 32 行匹配 ≈31 构造点 | 18（src，ast-grep）+9（tests/，ast-grep）+1（全限定）`None`；engine.rs legacy `None` 1 + LC gateway 路径 `Some(repository.path)` 1；新测试显式 2 |
| `SessionLaunchRequest` 构造 | 字面量 13 + `planning()` 调用 17 = 30 处 | 13 字面量逐一补值；17 处 planning() 经构造器派生零改动 |
| `SessionPolicyEnvelope::new` | 10 调用点 | 11 处插参（9 policy.rs 测试、validate_inner、新测试） |
| 计划口径「284/191」 | 引用计数（含类型引用/调用），非字面量 | — |

## 裁决记录

1. **SLR/Envelope 必填 `PathBuf` 保留**：构造点共 30 处（字面量 13+planning 17），未触发 controller 授权的回退条件（">40 处且多数与 LC 无关"）——该类型仅存在于 LC gateway 域，全部与 LC 相关。
2. **必要偏差（计划假设修正）**：计划称「284/191 存量构造不动」——Rust struct literal 必须命名新增字段，Option 字段亦不能免；实际以 ast-grep 批量注入 `working_directory: None` 共 **78 处**（语义零变化，全 lib 3959 测试绿佐证）。rustfmt 仅限改动文件；误碰的 `prompts/human_gate_revision.rs`（存量未格式化）已还原。

## 四个新测试（engine_gateway_guard.rs，逐字命名）

- `sync_input_preserves_root_working_directory_and_member_worktree` ✅（LC cwd≠target 字段层合法；target 不回退）
- `resume_fingerprint_changes_when_working_directory_drifts` ✅（漂移⇒指纹不等⇒supersede 维度；同 cwd 稳定；serde 往返保留）
- `legacy_input_defaults_working_directory_from_existing_path` ✅（None 回填 working_dir/worktree_path；legacy JSON 无键反序列化 None）
- `logical_input_explicit_working_directory_overrides_legacy_fallback` ✅（显式 root 优先；legacy 字段原样保留）

## 验证（实际执行）

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib sync_input_preserves_root_working_directory_and_member_worktree` | 先红（15 错误：E0560/E0599/E0609/E0061 全部指向新合同）→ 后绿 |
| `cargo test --locked --lib working_directory` | 4 passed（四个新测试） |
| `cargo test --locked --lib engine_gateway_guard` | **0 matched**（include! 进 tests.rs，无模块路径段，命令空跑绿）；已补跑文件实际测试：`repository_generate` 3 passed |
| `cargo test --locked --lib resume_or_start_supersedes_when_policy_digest_drifts` | 1 passed |
| `cargo test --locked --lib` | **3959 passed / 0 failed**（3 ignored）——存量零行为变化 |
| `cargo test --locked --test it_web provider_gateway_envelope` | 5/6；1 失败 `cross_target_baseline_capture_failed` 为**存量**（git stash 基线复现） |
| `cargo test --locked --test it_core/it_provider/it_task_run` | 186/187；1 失败 `large_file_guard`（1200 行守卫）为**存量**（基线 16 文件超限复现；本改动使 internal_pr_review.rs 净 -2 行） |
| `cargo test --locked --test it_web web_provider_health_api` | 3 passed |

## 范围边界

- 仅迁移 LC 入口矩阵 Split sync 行（engine.rs `invoke_provider_via_gateway` + `prepare_sync_launch`）；其余矩阵行文件（provider_run/gateway_start/review drive/coding retry 等）仅做编译所需机械补值，深层接线归 2.1–2.4/2.6–2.8。
- gateway cwd==target 等式（provider_gateway.rs `canonical_cwd != canonical_target`）未改动。

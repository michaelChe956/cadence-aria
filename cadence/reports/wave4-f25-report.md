# wave4-f25：scope_allows_path 中文顿号分隔修复报告

日期：2026-09-20 · 线：门重测阻塞修复（kimi author 方言）· commit `bdfeb45e`

## 根因

门重测现场（kimi author，attempt 9298fe0b）：author 把 `exclusive_scopes` 产成
**顿号单串**而非 codex 式数组：

```
"server/**、data/levels.json、tests/backend/**"
```

`scope_allows_path`（`src/cross_cutting/worktree.rs`）把整个单串当一个 glob 解析：
`ScopePattern::parse` 按 `*` 切分取首段 → `base = "server/"`。于是
`data/levels.json`（第二段、本应精确放行的合法改动）在 final_confirm diff 门
（`validate_changed_files_for_work_item` → `validate_write_path` /
gates.rs `scope_allows_path` 检查）被判为 scope 外 →
`WorkItemDiffScopeViolation` 楔死。

同方言也削弱 forbidden 方向：顿号单串 forbidden scope 整体解析后只钉住首段 base，
后续段的违规改动不会被标红。

## 修法（对照弯引号归一化 c60dbae9 同族保守边界）

`src/cross_cutting/worktree.rs` `scope_allows_path` 入口加确定性机械修：

- scope 含顿号（、U+3001）→ 按顿号切分、**逐段独立匹配**，任一段放行即放行；
- **空段永不放行**（连续/首尾顿号产生的空串不是通配授权）；
- 无顿号输入走原路径**零变化**；glob/大小写/路径归一语义不动；
- 顿号只作为分隔符解释，不 trim、不归一路径字符——与
  `artifact_projection/fields.rs::split_values`（`[',', ';', '，', '；', '、']`
  分隔方言族）同族，但按本次现场只加顿号一项，不扩大分隔集合。

消费方自动受益（单一实现）：`validate_write_path`（exclusive）、
gates.rs / schema_v2.rs 的 forbidden 检查、`group_review_material`。

## TDD 证据

- **红**（修复前，lib + it_core 双红，与现场同码）：
  - `dun_separated_scope_string_matches_each_alternative_independently`：
    `scope_allows_path("server/**、data/levels.json、tests/backend/**", "data/levels.json")` → false
  - `dun_separated_forbidden_scope_flags_each_alternative`：
    forbidden 单串 `"docs/**、secrets.json"` 对 `secrets.json` 不命中
  - `dun_separated_scope_string_allows_in_scope_paths_through_final_confirm_gate`
    （it_core，复刻 final_confirm 的 `validate_write_path` 链路）→ `ScopeDenied("data/levels.json")`
- **绿**：3 单测 + 1 集成全过；且锚定不扩大授权面（`web/index.html` 段外仍拒、
  `server` 裸目录不匹配 `server/**`）、空段安全（`、`、`server/**、、web/**`、
  `、server/**` 均不放行）。

## 回归

| 面 | 结果 |
|---|---|
| `cargo test --lib --locked worktree` | 62/62 绿（含 3 新） |
| `cargo test --test it_core --locked worktree_locking` | 3/3 绿（含 1 新） |
| `cargo test --lib --locked gates` | 19/19 绿（final_confirm 消费方） |
| `cargo test --lib --locked group_review` | 137/137 绿（同函数消费方） |
| `cargo fmt --check` 两文件 | 通过 |
| `cargo clippy --lib --locked --tests` | 本次改动 0 warning（sandbox.rs:702 为存量告警，非本次文件） |

## 影响

kimi author 顿号单串 scope 与 codex 数组在写授权/禁写检查上语义等价：
final_confirm 不再错拒段内合法改动；forbidden 顿号单串逐段 fail-closed 收紧。
无顿号路径行为逐字节不变。

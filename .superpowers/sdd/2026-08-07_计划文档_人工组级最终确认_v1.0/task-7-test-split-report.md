# Task 7 测试文件拆分报告

- 日期：2026-08-07
- 基线提交：`5e46f639c464db2afc4959dcd9be5889aa64cb43`
- 目的：将超过 1200 行守卫限制的组级最终就绪测试拆分为职责清晰的小模块，不修改生产代码。

## 变更

- 新增 `group_final_readiness_support.rs`：集中维护 `ReadinessFixture`、共享 seed helper、`review_report` 和 `complete_other_units`，供原 runner/legacy 测试与 builder 测试共同使用，避免复制。
- 新增 `group_final_readiness_builder.rs`：迁移 6 个 Task 2 builder 测试。
- 保留 `group_final_readiness.rs` 中的 legacy recovery、runner 与最终确认流程测试。
- 在 `tests.rs` 注册 builder 和 support 子模块。

## 行数

| 文件 | 行数 |
| --- | ---: |
| `group_final_readiness.rs` | 629 |
| `group_final_readiness_builder.rs` | 383 |
| `group_final_readiness_support.rs` | 441 |

所有拆分后的文件均低于 1200 行限制。

## 验证摘要

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked --lib group_final_readiness` | 25 passed |
| `cargo test --locked --lib group_final_readiness_builder` | 6 passed |
| `cargo test --locked --lib readiness_` | 20 passed |
| `cargo test --locked --test it_core large_file_guard` | 1 passed |
| `cargo fmt --check` | passed |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | passed |
| `cargo check --locked` | passed |

## 范围与风险

仅重组测试模块和共享测试 helper；未修改任何生产代码或测试断言。拆分通过 module-private `pub(super)` 可见性共享 fixture 与 helper，避免扩大到生产模块 API。

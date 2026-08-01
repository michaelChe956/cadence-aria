# Tasks: add-upgrade-flag-to-pre-check-bootstrap

## 工作包 1：pre_check 命令追加 `--upgrade` 参数

> 映射 requirement：`non-interrupt-repository-bootstrap` / 无中断 Claude Code 初始化命令

- [ ] 1.1 TDD：将引用 `/pre-check --no-interrupt 用大陆镜像` 的 Rust 单元/集成测试与前端测试断言更新为 `/pre-check --no-interrupt --upgrade 用大陆镜像`，确认先失败
- [ ] 1.2 实现：`RepositoryInitializationStepKind::command()` 的 `PreCheck` 分支改为 `/pre-check --no-interrupt --upgrade 用大陆镜像`，其余三条命令不变

## 工作包 2：全量验证

> 映射 requirement：工作包 1 的全部 requirement

- [ ] 2.1 执行仓库标准四命令：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked`
- [ ] 2.2 执行前端验证：`cd web && pnpm tsc -b`、`cd web && pnpm test`

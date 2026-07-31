# Tasks: use-mainland-mirror-for-bootstrap

## 工作包 1：Cadence-skills 源切换 Gitee 与存量克隆 origin 迁移

> 映射 requirement：`repository-initialization-progress` / Cadence-skills 准备步骤可见

- [x] 1.1 TDD：为 `CadenceSkillsManager` 补充失败测试——克隆 URL 为 Gitee 地址；存量克隆 origin 不匹配时请求序列包含 `remote get-url` 与 `remote set-url` 后再 fetch/pull；origin 已匹配时不发出 set-url；get-url/set-url 失败映射为 `update_failed`
- [x] 1.2 实现：`REPOSITORY_URL` 切换为 Gitee 地址；`update_source` 前置 origin 检测与 set-url 迁移
- [x] 1.3 更新 `manager.rs` 内既有测试断言中的 GitHub 地址

## 工作包 2：pre_check 命令追加"用大陆镜像"参数

> 映射 requirement：`non-interrupt-repository-bootstrap` / 无中断 Claude Code 初始化命令

- [x] 2.1 TDD：更新引用 `/pre-check --no-interrupt` 的 Rust 单元/集成测试与前端测试断言为 `/pre-check --no-interrupt 用大陆镜像`，确认先失败
- [x] 2.2 实现：`RepositoryInitializationStepKind::command()` 的 `PreCheck` 分支改为 `/pre-check --no-interrupt 用大陆镜像`

## 工作包 3：全量验证

> 映射 requirement：工作包 1、2 的全部 requirement

- [x] 3.1 执行仓库标准四命令：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked`
- [x] 3.2 执行前端验证：`cd web && pnpm tsc -b`、`cd web && pnpm test`

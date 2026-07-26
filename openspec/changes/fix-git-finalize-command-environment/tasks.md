# Tasks: fix-git-finalize-command-environment

## 1. 失败测试先行（TDD RED）

- [ ] 1.1 新增真实 git 回归测试：TempDir HOME + 含身份 `.gitconfig`，真实 runner + 注入环境驱动 add/commit，断言提交成功且作者匹配；同时覆盖"未注入时 commit 因缺身份失败"的对照。确认新测试 RED（当前实现无注入）。
- [ ] 1.2 新增注入 map 组装单测：默认构造含 `LC_ALL` 且仅在进程环境存在时含 `HOME`/`SSH_AUTH_SOCK`；`with_git_environment` 覆盖生效。

## 2. 实现（TDD GREEN）

- [ ] 2.1 `registration.rs`：coordinator 新增 `git_environment` 字段、构造默认值与 `with_git_environment`；`run_git` 改用该字段。确认 1.1/1.2 转 GREEN。
- [ ] 2.2 `web/handlers/repository_registration.rs`：builder 构造 coordinator 后链式注入 `{LC_ALL, HOME(验证过的), SSH_AUTH_SOCK?}`（含 fake 路径）。

## 3. 验证

- [ ] 3.1 `cargo fmt --check` 与 `cargo clippy --all-targets --all-features --locked -- -D warnings` 通过。
- [ ] 3.2 `cargo test --locked --lib repository_store` 全绿。
- [ ] 3.3 `cargo test --locked --test it_web web_repository_initialization` 全绿。
- [ ] 3.4 `openspec validate fix-git-finalize-command-environment --strict` 通过。

## 4. 交付

- [ ] 4.1 勾选 tasks 1.1–3.4 并提交；提醒操作者：真实验证路径为重新执行一次代码库初始化（或手动在目标仓库补齐 commit/push），确认 git_finalize 不再报身份错误。

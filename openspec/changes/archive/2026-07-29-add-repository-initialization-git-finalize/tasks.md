## 1. 后端 git_finalize 步骤与终态语义

- [ ] 1.1 `RepositoryInitializationStepKind` 新增 `GitFinalize`（`ALL` 末尾，`command()` 为 `None`），operation 创建六步、状态机与终态语义调整（前 5 步 + Repository 持久化成功即 `completed`）。（需求：repository-initialization-progress）
- [ ] 1.2 coordinator 在 Repository 持久化后执行 `git_finalize`：git add/commit/push 各分支（无改动跳过、无 remote、无上游、push 成功、push 失败），成功结果透出 `git_finalize_warning`。（需求：repository-initialization-progress、non-interrupt-repository-bootstrap）
- [ ] 1.3 HTTP DTO 与查询透出第六步状态与 `git_finalize_warning`。（需求：repository-initialization-progress）

## 2. 前端六步面板与失败提示

- [ ] 2.1 进度面板扩展为六步，新增 `git_finalize: "提交并推送"` 标签；operation `completed` 时仍刷新代码库列表。（需求：repository-initialization-progress）
- [ ] 2.2 `git_finalize` 失败时第六步红色展示 + "自动提交推送未完成，请在目标仓库手动执行 git commit / git push" 提示与无障碍播报。（需求：repository-initialization-progress）

## 3. 测试与验证

- [ ] 3.1 Rust 单元/集成测试：六步顺序、git add/commit/push 各分支、git_finalize 失败仍 completed、持久化不变量。（需求：repository-initialization-progress）
- [ ] 3.2 前端测试：六步渲染、git_finalize 失败红色 + 提示、列表刷新、无障碍。（需求：repository-initialization-progress）
- [ ] 3.3 执行项目规定的格式化、静态检查、Rust/前端测试与 OpenSpec 验证。（需求：repository-initialization-progress、non-interrupt-repository-bootstrap）

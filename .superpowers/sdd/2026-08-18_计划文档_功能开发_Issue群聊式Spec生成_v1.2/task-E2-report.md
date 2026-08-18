# Task E2：全量验证 + 文档收尾报告

## 文档与实现收尾

- 根目录 `README.md`「核心能力」新增「群聊式 Spec 生成」，说明建室、讨论、定稿、版本回看和 Work Item 派生能力，并注明开关位置：主工作台右上角「设置 → Spec 生成模式」。
- 新增 `cadence/notes/2026-06-30_群聊式Spec生成手动验证清单.md`，覆盖流水线模式回归及群聊模式建室、讨论、定稿、看板版本与 WorkItem 派生入口；所有项目均标注「待用户手动验证」，不启动真实 Provider。

## 全量验证

以下命令均在当前 worktree 执行，未使用 `-j 1`：

- `cargo test --locked`：通过；315 passed、0 failed、12 ignored；Doc-tests 1 passed。
- `cargo clippy --all-targets --locked -- -D warnings`：通过；编译完成且无 Clippy 警告。
- `cd web && pnpm test`：通过；115 个测试文件通过，879 tests passed。
- `cd web && pnpm tsc -b`：通过；无输出、退出码 0。

## 手动验证

未启动真实 Provider。请按 `cadence/notes/2026-06-30_群聊式Spec生成手动验证清单.md` 逐项执行，清单项目均为「待用户手动验证」。

## 疑虑与残余风险

- 真实 Provider、浏览器交互和生产数据迁移未在本任务中执行，需由用户在目标环境完成手动清单。
- 自动化测试覆盖 fake/provider-free 场景，不能替代真实 Provider 的网络、权限和凭据验证。

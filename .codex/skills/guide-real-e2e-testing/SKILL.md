---
name: guide-real-e2e-testing
description: Use when guiding Cadence Aria real end-to-end product workbench tests, issue lifecycle validation, manual browser checkpoints, target repository verification, or resuming after reported test failures.
---

# 真实 E2E 测试指导

## 核心原则

用于指导用户按真实产品路径测试 Cadence Aria：先了解当前 worktree、服务、目标仓库和 issue，再一步一步让用户在页面执行；用户反馈问题后，先修复并验证，再回到中断的测试步骤继续。

## 启动前检查

1. 读取当前 worktree 内 `AGENTS.md`、`CLAUDE.md`、`.claude/rules/` 和 `cadence/project-rules/README.md`。
2. 确认分支、未提交改动、目标仓库状态，不回退用户已有改动。
3. 默认在当前 worktree 启动 Aria 服务。除非用户明确要求，不要把被测业务仓库作为 `aria web --workspace` 参数；业务仓库应由用户在页面里添加为代码库。
4. 启动服务时使用 `start-cadence-aria-service` skill；后端默认：

```bash
cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
```

前端默认：

```bash
pnpm dev --port 5173
```

5. 健康检查必须通过：

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

## 指导方式

- 一次只给用户 1 个测试阶段的操作，不要一次性倾倒完整流程。
- 每个阶段说明：操作步骤、期望页面状态、失败时请用户反馈什么信息。
- 用户完成当前阶段并反馈后，再给下一阶段。
- 如果流程不确定，先问流程问题；不要猜测会改变测试结论的关键路径。
- 页面测试优先让用户真实操作；只有用户要求或需要辅助定位时再用浏览器自动化。

## 标准测试阶段

1. **服务可用**：确认 `/workbench` 打开，显示 `Issue 生命周期工作台`。
2. **Project 与代码库**：用户创建 Project，并添加真实业务仓库路径，例如 `/Users/michaelche/Documents/git-folder/github-folder/naruto`。
3. **Issue 创建**：用户创建真实 issue，确认它出现在 Issue 列。
4. **生成 Story Workspace**：从 Issue 卡片点击生成 Story Spec，进入 Workspace。
5. **PrepareContext**：确认阶段为 `准备中`，Provider 配置可见，可补充上下文。
6. **开始生成**：点击 `开始生成`，确认阶段进入 `运行中`，Timeline 出现 author 节点，Header 显示 Provider 锁定。
7. **人工/权限节点**：如出现 permission 请求，指导用户审批或拒绝，并记录预期结果。
8. **审核与确认**：按页面阶段处理 reviewer verdict、返修或确认。
9. **目标仓库验收**：回到目标仓库检查源码、测试、git diff 和测试命令结果。
10. **恢复/回归**：刷新页面或重连后确认 Timeline、Prompt、权限、Artifact 仍可恢复。

## 爬楼梯真实场景

默认测试 issue：

```text
实现爬楼梯问题：给定 n 阶楼梯，每次可以爬 1 或 2 阶，返回到达楼顶的不同方法数。请使用 python 实现函数 climb_stairs(n: i32) -> i32，并补充测试覆盖 n=1、n=2、n=3、n=5、n=10。测试目录为：/Users/michaelche/Documents/git-folder/github-folder/naruto
```

目标仓库默认路径：

```text
/Users/michaelche/Documents/git-folder/github-folder/naruto
```

最终验收至少确认：

- 目标仓库出现 Python 实现，函数名为 `climb_stairs`，签名语义为 `climb_stairs(n: int) -> int`。
- 测试覆盖 `n=1,2,3,5,10`，期望值分别为 `1,2,3,8,89`。
- Python 项目测试优先使用 `uv`，不要建议 `pip`。如果仓库没有 Python 测试框架配置，先根据实际文件判断，再决定是否需要补齐最小配置。
- `git status --short` 和 `git diff` 能说明 Aria 在目标仓库的真实改动。

## 用户反馈问题后的处理

收到失败、报错、页面异常或目标仓库结果不符合预期时：

1. 复述当前测试阶段和失败现象，确认要修的是产品代码、测试脚本、服务环境还是目标仓库产物。
2. 先收集证据：浏览器状态、后端/前端日志、目标仓库 `git status`、相关 API 或 WS 信息。
3. 对产品代码问题，使用系统化调试和 TDD：先写或定位失败测试，再修复，再跑聚焦回归。
4. 对目标仓库产物问题，不直接覆盖用户改动；先说明差异，再按用户确认修复或让 Aria 重新生成。
5. 修复后重新启动或刷新服务，给出验证命令结果。
6. 回到失败前的测试阶段，继续指导用户完成后续步骤。

## 汇报格式

阶段完成时简短汇报：

- 当前阶段
- 通过的检查
- 下一步操作

问题修复后汇报：

- 问题原因
- 修改文件
- 验证命令和结果
- 继续测试应从哪一步恢复

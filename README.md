# cadence-aria

Cadence-Aria 一期最小闭环实现，覆盖 formal flow、冻结引用、dispatch/exec 骨架，以及恢复与诊断命令的基础投影。

## 开发

```bash
pnpm install
pnpm build
pnpm check
pnpm test
```

## 关键目录

- `src/`：一期运行时代码与 CLI 命令
- `tests/`：单元、集成与 E2E 测试
- `cadence/cache/aria/`：运行时任务数据与配置模板

## 最小闭环

```bash
pnpm build
node dist/src/index.js aria:intake "一期闭环"
node dist/src/index.js aria:start --task-id <task-id>
node dist/src/index.js confirm-spec --task-id <task-id>
node dist/src/index.js confirm-plan --task-id <task-id>
node dist/src/index.js aria:run --task-id <task-id>
node dist/src/index.js aria:status --task-id <task-id>
node dist/src/index.js aria:result --task-id <task-id>
node dist/src/index.js aria:doctor
```

## 一期真实验收

除 `pnpm check`、`pnpm test`、`pnpm build` 外，还必须满足：

- `aria:start` 生成的 `spec-artifact.md` / `plan-brief.md` 带有 `producer: claude-code`
- `confirm-plan` 后存在合法 `execution-context-bundle.yaml` 与 `dispatch-contract-exec-01.yaml`
- `aria:run` 后存在合法 `exec-result-exec-01.yaml`、`review-report.yaml`、`test-report.yaml`
- `aria:doctor` 返回 `claude_code`、`codex`、`OpenSpec`、`superpowers`

## 检查与验证

人工检查表：

```bash
cat cadence/checklists/aria-real-integration-checklist.md
```

只读验证脚本：

```bash
scripts/verify-real-integration.sh --task-id <task-id>
```

如果不传 `--task-id`，脚本会自动检查最新任务。脚本只读取已有任务和构建产物，不会创建任务，也不会推进状态。

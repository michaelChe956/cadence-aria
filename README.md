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

# 节点文档：N00 session_bootstrap

## 1. 节点标识

- 节点 ID：`N00`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：主流程入口
- 版本：v1.0

## 2. 节点目的

为当前仓库建立或恢复一个可用的 `ProjectSession`，让后续节点始终运行在已连接的本地 runtime 上。

## 3. 进入条件

- 用户在 Git 仓库根目录或其子目录执行 `aria`
- 本地 daemon 可启动或可连接

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| repo root | 本地文件系统 | 是 | 绝对路径 | 中止启动 |
| daemon registry | 本地 runtime | 否 | 现有 daemon ref | 不存在则新建 |
| local invocation context | REPL | 是 | cwd、timestamp | 中止启动 |

## 5. Aria 驱动动作

1. 解析当前仓库根路径。
2. 检查是否已有可复用 daemon。
3. 如无则隐式启动 daemon。
4. 加载或创建 `ProjectSession`。
5. 生成 bootstrap snapshot。
6. 写 checkpoint。

## 6. Provider 执行契约

无。该节点不调用 Claude Code 或 Codex。

## 7. 输出产物

- `runtime_snapshot:N00.session_bootstrap`
- `ProjectSession` 引用

## 8. 输出产物最小格式

`runtime_snapshot:N00.session_bootstrap` 至少包含：

- `sessionId`
- `repoRoot`
- `daemonPid`
- `bootstrapMode`（attach/new/recover）
- `timestamp`

## 9. 完成判定

当 session 已可用、snapshot 已落盘、REPL 已拿到 `sessionId` 时视为完成。

## 10. 失败与回流

- daemon 启动失败：进入 `X08 manual_intervention_hold`
- session 恢复失败：进入 `X07 runtime_recover`

## 11. 交接到下一节点

成功后将 `sessionId`、`repoRoot`、`effectivePolicyDefaults` 交给 `N01 intake_capture`。

## 12. 关联横切能力

- `checkpoint_and_recovery`
- `policy_mode_and_override`
- `manual_intervention`


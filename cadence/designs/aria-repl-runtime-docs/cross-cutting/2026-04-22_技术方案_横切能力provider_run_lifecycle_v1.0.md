# 横切能力文档：provider_run_lifecycle

## 1. 能力标识

- 能力 ID：`CC03`
- 能力名称：`provider_run_lifecycle`
- 类型：调度 / 记录
- 适用范围：全部 Agent 业务节点

## 2. 能力目的

统一派发、记录、收集 Claude/Codex 调用，避免节点直接依赖 CLI 输出格式。

## 3. 触发条件

- Agent 节点进入执行阶段
- 原节点已具备 provider 输入包

## 4. 前置状态与输入

- `taskId`
- `nodeId`
- provider role
- provider context package
- optional session/thread refs

## 5. Aria 执行动作

1. 创建 `ProviderRun`
2. 通过 adapter 派发 `spawn + CLI`
3. 收集 stdout/stderr
4. 写入 run record
5. 提取结构化输出引用供节点消费

## 6. 状态变化与副作用

- 新增 `ProviderRun`
- 节点进入 `running`
- 输出收集成功后可生成产物

## 7. 输出与记录

- provider run record
- raw output refs
- structured extraction refs

## 8. 完成判定

run 已完成、输出已收集、run record 已落盘。

## 9. 失败与恢复

- 非零退出：标记 `provider_run_failed`
- 收集中断：保留已收输出并触发 retry/gate
- daemon 重启：running run 统一进入 recovering

## 10. 与节点文档的关联规则

所有 Agent 业务节点必须引用本能力，并在节点文档内声明默认 provider 与输入包。


# 横切能力文档：policy_mode_and_override

## 1. 能力标识

- 能力 ID：`CC02`
- 能力名称：`policy_mode_and_override`
- 类型：控制 / 调度
- 适用范围：全部节点

## 2. 能力目的

统一决定 Aria 在不同阶段是保守、平衡还是激进推进，并支持阶段级覆写。

## 3. 触发条件

- session 创建
- 用户执行 `policy set`
- 节点进入前应用阶段覆写

## 4. 前置状态与输入

- session 默认策略
- 阶段覆写配置
- 当前节点 ID

## 5. Aria 执行动作

1. 读取 session 级策略
2. 读取阶段覆写
3. 计算 `effectivePolicy`
4. 写入交接包
5. 在必要时改变是否自动进入 gate

## 6. 状态变化与副作用

- 更新 session 配置或节点有效策略
- 影响后续 gate、retry、自动推进行为

## 7. 输出与记录

- policy resolution snapshot
- policy override event

## 8. 完成判定

有效策略被写入交接包且被原节点消费后视为完成。

## 9. 失败与恢复

- 策略解析失败：回退到 `conservative`
- daemon 恢复后：重新根据 session 配置计算

## 10. 与节点文档的关联规则

全部节点都应引用该能力，但只在节点文档中说明本节点依赖的决策结果，不重复定义计算规则。

## 策略行为映射表

### 三种策略模式定义

| 策略 | 语义 | 核心理念 |
|------|------|---------|
| `conservative` | 保守模式 | 宁可多确认，不做未经审核的操作 |
| `balanced` | 平衡模式 | 自动处理低风险操作，中等风险确认 |
| `aggressive` | 激进模式 | 尽可能自动推进，仅高风险操作确认 |

### 节点级行为映射

| 节点 | 行为维度 | conservative | balanced | aggressive |
|------|---------|-------------|----------|------------|
| N04 clarification | open_questions 非空时 | 必须挂 gate | 仅 high severity 挂 gate | 自动填入假设推进 |
| N06 spec_gate_review | spec 有 open_items 时 | 必须挂 gate | open_items > 3 挂 gate | 自动标记为 accepted |
| N08 design_review | review 触发 | 必须执行 review | 必须执行 review | 仅重大设计执行 review |
| N10 readiness_check | 阻塞项 | 阻塞项 > 0 必须回流 | 阻塞项 > 2 回流 | 自动标记为 ready |
| N16 coding | provider 失败重试 | 1 次 | 2 次 | 3 次 |
| N17 testing | 测试失败 | 立即 rework | partial pass 允许推进 | partial pass 允许推进 |
| N18 code_review | review 触发 | 必须执行 | 必须执行 | 仅高风险文件执行 |
| N19 rework | rework 上限 | 3 次 | 3 次 | 5 次 |
| N22 integration_prepare | 冲突预检 | 任何冲突挂 gate | 仅 high 冲突挂 gate | 自动尝试合并 |
| N25 final_review | followup 需求 | 任何缺口挂 gate | 仅 high 缺口挂 gate | 自动派生补丁任务 |

### 策略变更对运行中任务的影响

- 策略变更仅影响尚未进入执行的节点
- 已进入 `running` 状态的节点继续使用进入时的 `effectivePolicy`
- 策略变更后，下一个 checkpoint 将记录新的 `effectivePolicy`
- 用户通过 REPL 执行 `policy set` 后，Aria 更新 session 级策略，并在下一个节点进入前重新计算 `effectivePolicy`

### 阶段级覆写格式

阶段级覆写存储在 session 配置中，格式如下：

````json
{
  "policy": {
    "default": "conservative",
    "overrides": {
      "design": "balanced",
      "execution": "aggressive",
      "integration": "conservative"
    }
  }
}
````

每个阶段对应一组节点：
- `intake`：N01-N03
- `clarification`：N04
- `spec`：N05-N06
- `design`：N07-N10
- `plan`：N11-N12
- `execution`：N13-N19
- `integration`：N20-N24
- `closing`：N25-N28

### effectivePolicy 计算规则

1. 获取当前节点所属阶段
2. 查找该阶段是否有覆写
3. 若有覆写则使用覆写值，否则使用 session 默认值
4. 若策略解析失败（如值非法），降级为 `conservative`
5. 计算结果写入交接包的 `effectivePolicy` 字段


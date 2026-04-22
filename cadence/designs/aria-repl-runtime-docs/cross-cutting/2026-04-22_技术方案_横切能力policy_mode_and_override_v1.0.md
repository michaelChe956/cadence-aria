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


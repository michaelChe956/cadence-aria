# 修复 LC 初始化失败关闭与配置边界

## Why

2026-11-16客户证据显示：pre-check返回failed/rc1仍因14处文件变化被标成功；规则配置成功后模型补建索引耗尽1800秒；OpenSpec2.0共享技能布局不再生成Pi/Kimi旧路径而被误判失败。

## What Changes

- 明确失败报告优先于变更事实并保留receipt。
- LC配置调用rule-config显式配置-only参数，索引由既有后续步骤负责。
- Cadence-skills兼容OpenSpec2共享技能及旧布局。
- 增加对应回归，更新契约/说明；不改provider选择、超时与持久化格式。

## Impact

Aria：non-interrupt-repository-bootstrap消费判定与prompt；Skills：pre-check、rule-config。新增向后兼容可选参数，Skills先发布。无需数据迁移；不自动修改客户状态；不涉及前端代码。

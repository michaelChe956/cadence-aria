# Proposal: improve-multi-repo-onboarding-ux

## Why

人工测试首轮 UI 反馈三条：①创建 project 的「多仓库模式」是独立 checkbox，与原「添加代码库」弹窗割裂；②启用多仓库应通过选择模式本身表达，无需二次勾选确认；③登记向导需要手填全部候选路径，体验差。

## What Changes

1. CreateProjectDialog 改为模式单选（单仓库默认 / 多仓库）：选择多仓库即 multi_repo=true，选择单仓库即原流程；移除独立 checkbox 确认环节。API 不变。
2. 登记 preflight 端点新增可选 auto_discover：服务端以聚合根下子目录（深度 1，发现 git 仓）为候选自动执行预检分类；前端登记向导填完聚合根后自动拉取候选并以勾选列表展示（含分类徽标，eligible 默认勾选），用户不再手填路径列表。

## Capabilities

### New Capabilities
（无）

### Modified Capabilities

- `multi-repo-project-mode`：创建弹窗的模式表达（单选替代 checkbox）。
- `logical-codebase-registration`：preflight auto_discover 与向导候选自动发现交互。

## Impact

- 前端：CreateProjectDialog、LogicalCodebaseRegistrationWizard、API types。
- 后端：preflight handler/DTO 增加可选 auto_discover（向后兼容，默认 false=现行为）。

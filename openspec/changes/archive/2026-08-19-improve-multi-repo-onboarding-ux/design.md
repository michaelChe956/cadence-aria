# Design: improve-multi-repo-onboarding-ux

## 决策

- D1：CreateProjectDialog 的 multi_repo 表达从 checkbox 改为单选（单仓库默认 / 多仓库）。选多仓库即提交 multi_repo=true；无独立勾选确认。API 不变（1.1 已落地字段）。
- D2：preflight 端点 `RegistrationPreflightRequest` 增加可选 `auto_discover: bool`（serde default=false，向后兼容）。true 时服务端扫描聚合根直接子目录发现 git 仓作为候选（忽略 candidate_paths），复用既有 AggregateRootPreflight→coordinator.preflight 分类与快照冻结；discovery 失败（根不存在/无权限）→ 422 aggregate_root_missing 族。
- D3：向导交互：填聚合根 → 自动 auto_discover 预检 → 勾选列表（eligible 默认勾选；needs_attention 显式勾选=确认；其余默认不勾选）；保留手填兜底。

## 实现要点

- 后端：candidate discovery 归属 coordinator 或 handler 层最小实现（listdir 根 + 判 .git 存在，含 linked worktree 的 .git 文件）；发现集合为空→返回空 items（前端兜底手填）。
- 前端：wizard 第一步根输入确认后自动请求（带 loading），第二步候选列表勾选（checkbox + class badge），第三步提交（沿用同步 loading）。

## 测试

- 后端：it_web auto_discover 用例（根下混合 eligible/非 git/脏仓，断言自动分类）+ 兼容性（不传 auto_discover 行为不变）。
- 前端：向导单测（自动拉取→默认勾选规则→提交 confirmed_paths 仅含勾选项）+ dialog 单选断言。

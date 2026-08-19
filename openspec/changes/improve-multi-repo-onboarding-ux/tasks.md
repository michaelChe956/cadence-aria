# Tasks: improve-multi-repo-onboarding-ux

## 1. 后端 auto_discover

- [x] 1.1 RegistrationPreflightRequest + auto_discover（serde default=false）；handler true 分支扫描聚合根直接子目录发现 git 仓作为候选（含 .git 文件的 linked worktree），分类与快照沿用既有链路；发现为空→空 items；it_web：auto_discover 分类用例 + 兼容回归 + TDD 先红后绿

## 2. 前端模式单选与向导自动发现

- [x] 2.1 CreateProjectDialog checkbox→单选（单仓库默认/多仓库），提交 multi_repo 对应值；单测断言两态
- [x] 2.2 登记向导自动发现：根确认→自动 auto_discover 预检（loading）→候选勾选列表（eligible 默认勾选/needs_attention 显式勾选/其余不勾选）→提交仅含勾选项；空结果兜底手填；vitest + tsc

## 3. 门禁

- [ ] 3.1 全门禁（fmt/clippy/check/定向+全量测试/it_core/前端/tsc/openspec validate 两 change）

# Delta: repository-initialization-progress

## MODIFIED Requirements

### Requirement: Cadence-skills 准备步骤可见
系统 SHALL 将 Cadence-skills 的下载/更新、离线回退和三层 Skills 软链同步作为第一个固定步骤 `cadence_skills` 的实际工作内容。Cadence-skills 源仓库地址 SHALL 为 Gitee 地址 `https://gitee.com/michaelChe-World/Cadence-skills.git`。对已有本地克隆执行更新前，系统 SHALL 检测其 `origin` 远程地址，与目标地址不一致时先将其修正为目标地址再执行更新，且 SHALL NOT 删除已有克隆目录。

#### Scenario: Cadence-skills 准备成功
- **WHEN** Cadence-skills 源准备和软链同步成功
- **THEN** 系统 SHALL 将 `cadence_skills` 标为 `completed`，并保留现有 source mode、Git 更新、软链同步和 warning 摘要供最终结果使用

#### Scenario: 新克隆使用 Gitee 源
- **WHEN** 本地不存在 Cadence-skills 源目录，系统执行首次克隆
- **THEN** 系统 SHALL 从 `https://gitee.com/michaelChe-World/Cadence-skills.git` 克隆

#### Scenario: 存量克隆 origin 迁移
- **WHEN** 已有 Cadence-skills 本地克隆的 `origin` 地址不是目标 Gitee 地址
- **THEN** 系统 SHALL 先执行 `git remote set-url origin https://gitee.com/michaelChe-World/Cadence-skills.git`，再执行 fetch/pull 更新，且不得删除该克隆目录

#### Scenario: 存量克隆 origin 已匹配
- **WHEN** 已有 Cadence-skills 本地克隆的 `origin` 地址已是目标 Gitee 地址
- **THEN** 系统 SHALL 直接执行 fetch/pull 更新，不重复设置 origin

#### Scenario: Cadence-skills 准备失败
- **WHEN** Cadence-skills 无法下载、更新、验证或同步软链
- **THEN** 系统 SHALL 将 `cadence_skills` 标为 `failed`，不得开始任一 Claude Code 命令，并提供既有可恢复错误信息

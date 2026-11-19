## MODIFIED Requirements

### Requirement: 同一实例与限定记录

归档 MUST 含与该实例 `GET /api/runtime-info` 同源且值相同的 `runtime-info.json`。Aria 记录 MUST 仅来自宿主 `$HOME/.aria/projects` 下的普通 `.json`：目录分量为 `aggregate-initializations` 或 `aggregate-recipe-receipts` 的树仍按既有规则收录；新增一类仅限 `<project>/logical-codebase/aggregate-indexes/<id>.json` 与 `<project>/logical-codebases/<lc>/aggregate-indexes/<id>.json` 的直接世代摘要。新增投影只允许已核实的世代身份、状态、时间、成员版本、可识别的失败码/事实类型、实测覆盖数量、耗时及逐成员盘检证据。不导出任意 warning 正文、stderr、SDK 输出、成员路径或 Git 快照正文；授权、大小预算与不跟随链接的约束不变。

#### Scenario: 嵌套回执与其他旁文件

- **WHEN** 回执树存在 `commands/` 下的逐命令 JSON，项目目录同时含锁文件、其它状态、配置和符号链接
- **THEN** 收录允许的普通 JSON，包括嵌套命令回执，保留相对层次；不收录无关旁文件，也不跟随链接

#### Scenario: 聚合索引重建失败

- **WHEN** 索引世代保存了失败事实与盘检诊断
- **THEN** 下载包能对上同一次重建的 ID、状态、时间、已知失败类型和诊断；缺字段的旧世代不得伪造原因，未知字段不原样回退，清单区分未收录与收录
- **AND** 清单应说明这类条目是诊断摘要而非原始 JSON。保留实际世代状态；被写入 LKG 的另一代错误不是该 LKG 自身的盘检证据。

## ADDED Requirements

### Requirement: Pi 会话来源可解释

Pi 枚举 SHALL 遵循启动 Pi 所用的有效环境，包括宿主覆盖与已经探测到的登录 shell 覆盖，兼容安全的平铺及按 cwd 分层的原生会话文件。仅将可核实会话身份的记录按现有规则分组、取最近最多三个；不得扩展到整台机器搜索或用未知文件填满名额。

#### Scenario: Pi 使用非默认根或没有有效会话

- **WHEN** Pi 使用覆盖目录，或目录中有候选文件但无法识别其原生会话身份
- **THEN** 用有效根枚举，清单区分无可用文件、候选未匹配、不可读；仅导出白名单诊断轨迹，且不为导出运行 Pi。
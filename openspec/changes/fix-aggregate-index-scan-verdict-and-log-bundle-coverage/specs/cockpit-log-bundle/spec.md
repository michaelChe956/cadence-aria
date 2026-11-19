## MODIFIED Requirements

### Requirement: 同一实例与限定记录

归档 MUST 含与该实例 `GET /api/runtime-info` 同一来源、字段和值一致的 `runtime-info.json`。Aria 记录只读取宿主 `$HOME/.aria/projects` 下允许的普通 JSON：`aggregate-initializations` 和 `aggregate-recipe-receipts` 树按原规则；另对项目作用域下 `aggregate-indexes` 的世代文件仅导出诊断字段白名单投影。不得导出任意 warning 正文、成员路径、Git 快照正文、配置或命令；原有授权、大小预算、manifest 和不跟随链接的约束不变。

#### Scenario: 嵌套回执与其他旁文件

- **WHEN** 回执树存在 `commands/` 下的逐命令 JSON，项目目录同时含锁文件、其它状态、配置和符号链接
- **THEN** 收录允许的普通 JSON，包括嵌套命令回执，保留相对层次；不收录无关旁文件，也不跟随链接

#### Scenario: 聚合索引重建失败

- **WHEN** 索引世代保存了失败事实与盘检诊断
- **THEN** 下载包能对上同一次重建的 ID、状态、时间、已知失败类型和诊断；缺字段的旧世代不得伪造原因，未知字段不原样回退，清单区分未收录与收录

## ADDED Requirements

### Requirement: Pi 会话来源可解释

Pi 枚举 SHALL 遵循启动 Pi 所用的有效环境，包括宿主覆盖与已经探测到的登录 shell 覆盖，兼容安全的平铺及按 cwd 分层的原生会话文件。仅将可核实会话身份的记录按现有规则分组、取最近最多三个；不得扩展到整台机器搜索或用未知文件填满名额。

#### Scenario: Pi 使用非默认根或没有有效会话

- **WHEN** Pi 使用覆盖目录，或目录中有候选文件但无法识别其原生会话身份
- **THEN** 用有效根枚举，清单区分无可用文件、候选未匹配、不可读；仅导出白名单诊断轨迹，且不为导出运行 Pi。
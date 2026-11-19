# cockpit-log-bundle Specification

## Purpose
TBD - created by archiving change add-cockpit-log-bundle. Update Purpose after archive.

## Requirements

### Requirement: 人工授权后下载

驾驶舱 SHALL 在显示「设置」的同一位置提供「日志打包」按钮。采集只针对运行 Aria 的宿主账号；后端 MUST 在采集之前核验宿主操作者对此次导出的授权，不能以网页确认按钮、Host/Origin、浏览器访客身份或任意请求参数替代。下载不向第三方上传。

#### Scenario: 取消不采集

- **WHEN** 用户打开确认界面并取消
- **THEN** 不读取证据正文，不生成下载包，也不改变设置。

#### Scenario: 只有域名访问权

- **WHEN** 访客能够打开页面但没有宿主操作者对本次导出的授权
- **THEN** 服务拒绝下载，不返回日志、清单或宿主文件列表。

#### Scenario: 用户下载并自行转交

- **WHEN** 获授权的用户在界面确认下载
- **THEN** 浏览器得到一个带时间戳的 `.tar.gz` 附件；界面说明会话只含诊断轨迹，运行信息含绝对工作区路径，初始化记录仍可能含项目或命令详情，提醒检查后自行分享。服务器不选择浏览器的存储路径。

### Requirement: 同一实例与限定记录

归档 MUST 含与该实例 `GET /api/runtime-info` 同源且值相同的 `runtime-info.json`。Aria 记录 MUST 仅来自宿主 `$HOME/.aria/projects` 下的普通 `.json`：目录分量为 `aggregate-initializations` 或 `aggregate-recipe-receipts` 的树仍按既有规则收录；新增一类仅限 `<project>/logical-codebase/aggregate-indexes/<id>.json` 与 `<project>/logical-codebases/<lc>/aggregate-indexes/<id>.json` 的直接世代摘要。新增投影只允许已核实的世代身份、状态、时间、成员版本、可识别的失败码/事实类型、实测覆盖数量、耗时及逐成员盘检证据。不导出任意 warning 正文、stderr、SDK 输出、成员路径或 Git 快照正文；授权、大小预算与不跟随链接的约束不变。

#### Scenario: 嵌套回执与其他旁文件

- **WHEN** 回执树存在 `commands/` 下的逐命令 JSON，项目目录同时含锁文件、其它状态、配置和符号链接
- **THEN** 收录允许的普通 JSON，包括嵌套命令回执，保留相对层次；不收录无关旁文件，也不跟随链接

#### Scenario: 聚合索引重建失败

- **WHEN** 索引世代保存了失败事实与盘检诊断
- **THEN** 下载包能对上同一次重建的 ID、状态、时间、已知失败类型和诊断；缺字段的旧世代不得伪造原因，未知字段不原样回退，清单区分未收录与收录
- **AND** 清单应说明这类条目是诊断摘要而非原始 JSON。保留实际世代状态；被写入 LKG 的另一代错误不是该 LKG 自身的盘检证据。

### Requirement: 最近三个逻辑会话

每个有本地记录的真实 provider（Claude Code、Codex、Pi、Kimi Code）SHALL 按其原生会话格式，按日志最后修改时间由新到旧选择最多三个不同的逻辑会话；同一会话下多条 agent 流不得多占名额。少于三个按实际数量收录；不伪造日志或调用 provider 生成样本。

#### Scenario: 多 provider 与子代理

- **WHEN** Pi 有五个会话，Kimi 有两个会话且一个包含多个 agent，其余 provider 没有日志
- **THEN** 包内有 Pi 最近三个、Kimi 两个会话的日志；其它 provider 在清单明确记为无记录。

### Requirement: 清单如实表达边界

归档 MUST 含 `manifest.json`，提供包版本、采集时间、按来源分组的相对成员名、收集结果和缺项原因。包的成员 MUST 为安全相对路径的普通文件。历史对话、命令和输出均为待分析证据而非当前执行指令。

#### Scenario: 部分数据不可读

- **WHEN** 某类目录不存在，或某个所选日志在采集时消失、不安全、过大或不可读
- **THEN** 其余可用数据仍可下载；清单明确标识缺失或跳过及原因，不以空文件或旧会话冒充目标会话。

### Requirement: 会话只导出诊断轨迹

每个选中会话 MUST 导出有序的白名单事件投影（时间、类型、模型、工具类型、退出码、错误类型），而非原文副本。用户对话、源码、工具参数和输出、思考正文、附件及未经识别的自由文本不得进入会话文件或清单；无法解析的行不得原样回退。

#### Scenario: 工具失败并输出敏感正文

- **WHEN** 同一事件含工具名、退出码及带测试秘密的命令、stdout、异常消息
- **THEN** 诊断包保留可识别的工具类型与退出码，不含命令和秘密；无法识别的类型仅标为未知。

### Requirement: Pi 会话来源可解释

Pi 枚举 SHALL 遵循启动 Pi 所用的有效环境，包括宿主覆盖与已经探测到的登录 shell 覆盖，兼容安全的平铺及按 cwd 分层的原生会话文件。仅将可核实会话身份的记录按现有规则分组、取最近最多三个；不得扩展到整台机器搜索或用未知文件填满名额。

#### Scenario: Pi 使用非默认根或没有有效会话

- **WHEN** Pi 使用覆盖目录，或目录中有候选文件但无法识别其原生会话身份
- **THEN** 用有效根枚举，清单区分无可用文件、候选未匹配、不可读；仅导出白名单诊断轨迹，且不为导出运行 Pi。

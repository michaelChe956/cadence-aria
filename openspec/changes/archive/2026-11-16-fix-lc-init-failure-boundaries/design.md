# LC 初始化失败边界修复

## 目标与授权

用户在完成现场取证后明确要求修复。范围为：Aria失败报告错误放行、配置阶段重复索引耗尽预算、OpenSpec2共享技能布局不兼容。不操作客户环境，不发版或改既有操作状态。

## 方案选择

采用最小显式边界：保留provider/五步/receipt/超时；先判断脚本失败，再判断零变更。新增rule-config可选 `--defer-codegraph-index`，LC追加上下文要求调用关联脚本时透传，普通独立运行不变。OpenSpec按真实共享目标marker和有效核心技能识别Pi/Kimi，旧布局保留。

不采用：增加超时、自动重试索引、通过制造文件变化绕过判定、把failed改为success、全面重写provider执行器或报告协议。

## 契约

1. receipt照常生成并保留；工具报告Failed无论有无文件变化都使步骤Failed，后续Pending，cause=`root_recipe_script_failed`，诊断携带provider/step及真实输出。Success+零变更仍成功。Absent+有变更保留旧语义，未开展报告协议迁移。
2. 四条skill调用文本不变；LC追加执行上下文是关联脚本选项的明确来源，不改共享RepositoryInitializationStepKind及receipt命令身份。
3. defer默认false；S8启用时只补双MCP配置并记录deferred，不运行CodeGraph子进程、不触碰索引。其余步骤与既有配置错误语义不变，备份/发布失败不允许吞掉。dry-run报告计划但不写入。
4. 既有后续aggregate_index仍负责真实建索引、覆盖验收和失败记录；配置成功不代表索引ready。LC模型不自行init/index/sync或后台等待索引。prompt不是进程级安全沙箱，不声称绝对阻止模型违规。
5. OpenSpec共享marker逐行首token列出pi/kimi时，只认六项core SKILL.md可读非空，不回退损坏shared状态；marker未列当前客户端时仍允许完整旧布局，兼容1.x单id marker。支持上游own-files后缀。
6. 不自动修复/重写客户现有失败记录；更新后明确人工发起新操作。发布依赖Skills先支持参数、Aria后消费；不允许旧Skills拒绝参数后退回无参数init。

## 验证

现场日志作为缺陷重现证据；增加有变更failed的端到端fixture回归、defer新建/已有索引及失败关闭测试、OpenSpec共享布局正负例。controller跑定向测试，独立审查。无真实客户/macOS/provider验证时如实保留限制。

# Spec Delta

## MODIFIED Requirements

### Requirement: 统一宿主执行与受信环境（REQ-ENV-HOST-01）

四家 provider 的全角色、Direct/LC、同步/流式及 ACP/MCP 子执行 SHALL 继承当前用户宿主环境执行，平台不构造、不覆盖子进程环境（调用方显式 env map 仅作覆盖层），不施加应用层 bwrap/netns/no-new-privs。执行合同由平台受信装配并冻结，不能被自报 role、prompt 或环境升级；HOME 为用户宿主 HOME，凭据随宿主环境可达，安全由用户自管，平台不作凭据隔离或物理防写承诺。

受信 provider 控制变量（provider 官方声明的推理档位、单次工具输出上限或前缀缓存 TTL 变量）MAY 由平台仅在调用方显式 env map 覆盖层内设定为该 provider 的受控值；MUST NOT 构造或覆盖其余子进程环境，MUST NOT 涉及凭据、隔离、网络命名空间、宿主路径、模型与账号选择、代理与路由、遥测开关；档位类 MUST NOT 依模型标签或宿主设置重算，输出上限类 MUST 只收紧，缓存 TTL 类 MUST NOT 短于 provider 内置默认。原「平台不构造、不覆盖」义务对该类以外环境不变。

#### Scenario: 无 bwrap 的全角色启动
- **WHEN** 宿主没有 bwrap 而具备该 provider 的必要 CLI 与运行前提
- **THEN** Author、规划、Reviewer 与 Coder SHALL 不因缺 bwrap 失败；全部记录宿主非隔离事实，不声称物理防写或凭据不可达

#### Scenario: 运行环境与清理边界
- **WHEN** 平台构造或清理 provider 运行环境
- **THEN** 系统 SHALL 记录环境策略版本而不泄密、不将环境具体值写入摘要或日志；子进程环境继承宿主，平台不注入（**受信 provider 控制变量除外**，见本条描述段与「受信 provider 控制变量只在覆盖层内设定」场景）；只能清理本轮运行前创建并登记的私有目录和资源，绝不依据 HOME/XDG 值递归清除 owner 目录、共享缓存或用户容器

#### Scenario: 平台受信 Git 不执行仓库注入代码
- **WHEN** 平台启动 Git 操作，包括快照、跨 target 检查、交付与补偿
- **THEN** 系统 SHALL 禁用仓库 hooks 与 fsmonitor 桥，不因 provider 写入本地配置而自动执行它；Coder 窗口外合法提交保持可用，受信 Git 与 provider 自己执行命令的信任边界如实区分

#### Scenario: 受信 provider 控制变量只在覆盖层内设定
- **WHEN** 平台为某个 provider 出口设定该 provider 官方声明的受信 provider 控制变量（推理档位、单次工具输出上限或前缀缓存 TTL 类，如 `CLAUDE_CODE_EFFORT_LEVEL` / `MAX_MCP_OUTPUT_TOKENS` / `BASH_MAX_OUTPUT_LENGTH` / `CLAUDE_CODE_PROMPT_CACHE_TTL`）为受控值
- **THEN** 该变量 SHALL 只在调用方显式 env map 覆盖层内被设定；其余子进程环境 SHALL 继续继承宿主环境且不被平台构造或覆盖，凭据、隔离与网络命名空间 SHALL NOT 受该例外影响；推理档位类 MUST NOT 依模型标签或宿主设置重算，输出上限类 MUST 只收紧，缓存 TTL 类 MUST NOT 短于 provider 内置默认

#### Scenario: 受信 provider 控制变量清单与三类约束
- **WHEN** 平台在某个 provider 出口的调用方显式 env map 覆盖层内设定受信 provider 控制变量
- **THEN** 变量清单 SHALL 限于 provider 官方声明的三类：推理档位类（如 `CLAUDE_CODE_EFFORT_LEVEL`）、单次工具输出上限类（如 `MAX_MCP_OUTPUT_TOKENS`、`BASH_MAX_OUTPUT_LENGTH`）、前缀缓存 TTL 类（如 `CLAUDE_CODE_PROMPT_CACHE_TTL`）；推理档位类 MUST NOT 依模型标签或宿主设置重算，输出上限类 MUST 只收紧，缓存 TTL 类 MUST NOT 短于 provider 内置默认；三类以外的变量不属于本例外

#### Scenario: 输出上限只收紧不放宽
- **WHEN** 平台尝试把输出上限类变量（如 `MAX_MCP_OUTPUT_TOKENS`、`BASH_MAX_OUTPUT_LENGTH`）设定为**高于**该 provider 内置默认的值
- **THEN** 该设定 MUST 被拒绝；输出上限类例外仅允许收紧（低于内置默认），MUST NOT 借 env 把工具输出上限调大或放开到无限

#### Scenario: 缓存 TTL 不得短于内置默认
- **WHEN** 平台尝试把缓存 TTL 类变量（`CLAUDE_CODE_PROMPT_CACHE_TTL`）设定为**短于**该 provider 内置默认的值
- **THEN** 该设定 MUST 被拒绝；缓存 TTL 类例外 MUST NOT 用于缩短 provider 内置默认 TTL（防误伤前缀缓存）

#### Scenario: 例外不涉及凭据隔离网络命名空间与宿主路径
- **WHEN** 平台尝试经例外类设定凭据类（API key、auth token）、隔离类（沙箱、容器隔离开关）、网络命名空间类或宿主路径类（HOME、XDG 路径）环境变量
- **THEN** 该设定 MUST 被拒绝；该例外 MUST NOT 涉及凭据、隔离、网络命名空间或宿主路径；凭据仍随宿主环境可达，平台仍不施加应用层 bwrap/netns/no-new-privs，HOME 仍为用户宿主 HOME

#### Scenario: 例外不涉及模型账号代理路由与遥测开关
- **WHEN** 平台尝试经例外类设定模型选择、账号选择、代理或路由（proxy、base URL）、遥测开关（telemetry、metrics）类环境变量
- **THEN** 该设定 MUST 被拒绝；模型与账号选择、代理与路由、遥测开关 MUST NOT 经该例外进入 env 覆盖层

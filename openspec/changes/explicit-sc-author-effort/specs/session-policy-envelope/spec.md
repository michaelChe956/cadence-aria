# Spec Delta

## MODIFIED Requirements

### Requirement: 统一宿主执行与受信环境（REQ-ENV-HOST-01）

四家 provider 的全角色、Direct/LC、同步/流式及 ACP/MCP 子执行 SHALL 继承当前用户宿主环境执行，平台不构造、不覆盖子进程环境（调用方显式 env map 仅作覆盖层），不施加应用层 bwrap/netns/no-new-privs。执行合同由平台受信装配并冻结，不能被自报 role、prompt 或环境升级；HOME 为用户宿主 HOME，凭据随宿主环境可达，安全由用户自管，平台不作凭据隔离或物理防写承诺。

受信执行策略变量（provider 官方声明的单一档位变量，如 `CLAUDE_CODE_EFFORT_LEVEL`）MAY 由平台在**调用方显式 env map 覆盖层**内设定为该 provider 的受控值；该例外 MUST NOT 构造或覆盖其余子进程环境、MUST NOT 涉及凭据、隔离或网络命名空间、且 MUST NOT 依模型标签或宿主设置重算。原「平台不构造、不覆盖子进程环境」的义务对本变量以外的环境保持不变。

#### Scenario: 无 bwrap 的全角色启动
- **WHEN** 宿主没有 bwrap 而具备该 provider 的必要 CLI 与运行前提
- **THEN** Author、规划、Reviewer 与 Coder SHALL 不因缺 bwrap 失败；全部记录宿主非隔离事实，不声称物理防写或凭据不可达

#### Scenario: 运行环境与清理边界
- **WHEN** 平台构造或清理 provider 运行环境
- **THEN** 系统 SHALL 记录环境策略版本而不泄密、不将环境具体值写入摘要或日志；子进程环境继承宿主，平台不注入（**受信执行策略变量除外**，见本条描述段与「受信执行策略变量只在覆盖层内设定」场景）；只能清理本轮运行前创建并登记的私有目录和资源，绝不依据 HOME/XDG 值递归清除 owner 目录、共享缓存或用户容器

#### Scenario: 平台受信 Git 不执行仓库注入代码
- **WHEN** 平台启动 Git 操作，包括快照、跨 target 检查、交付与补偿
- **THEN** 系统 SHALL 禁用仓库 hooks 与 fsmonitor 桥，不因 provider 写入本地配置而自动执行它；Coder 窗口外合法提交保持可用，受信 Git 与 provider 自己执行命令的信任边界如实区分

#### Scenario: 受信执行策略变量只在覆盖层内设定
- **WHEN** 平台为某个 provider 出口设定该 provider 官方声明的受信执行策略变量（单一档位变量，如 `CLAUDE_CODE_EFFORT_LEVEL`）为受控值
- **THEN** 该变量 SHALL 只在调用方显式 env map 覆盖层内被设定；其余子进程环境 SHALL 继续继承宿主环境且不被平台构造或覆盖，凭据、隔离与网络命名空间 SHALL NOT 受该例外影响，该变量 MUST NOT 依模型标签或宿主设置重算

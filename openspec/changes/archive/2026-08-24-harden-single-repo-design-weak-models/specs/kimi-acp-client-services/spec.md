## MODIFIED Requirements

### Requirement: 权限与路径沙箱（角色与 fs）（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

所有 terminal/fs 请求 SHALL 经过当前 `ProviderPermissionMode` 与角色 action policy，不因 initialize 声明能力即执行：supervised 模式下经 ApprovalBridge 征求用户；reviewer 角色默认拒绝 terminal 执行与 fs 写；coding 角色限制在目标 worktree；planning/author 角色（Orchestrator）SHALL 允许 FsRead 与 Terminal 只读执行（auto 模式仍强制 bwrap 沙箱与命令白名单、supervised 仍走 ApprovalBridge），FsWrite SHALL 拒绝——以解除项目规则 read-gate（要求 author 读取规则文件）与客户端服务全拒之间的死锁。fs 读写路径 SHALL 在授权根内，并用 no-follow 原子语义（`openat`+`O_NOFOLLOW` 等价封装，处理新建文件父目录校验）杜绝 TOCTOU；拒绝绝对越界、`..`、symlink 逃逸；权限拒绝与越界路径返回错误，不执行。

#### Scenario: reviewer 只读
- **WHEN** reviewer 会话的 kimi 发起 terminal/create 或 fs/write
- **THEN** 默认拒绝并返回错误

#### Scenario: 越界拒绝
- **WHEN** fs 请求路径经 symlink 指向授权根外
- **THEN** 拒绝，不读取；同类用例覆盖新建文件父目录越界与 symlink 竞态

#### Scenario: planning/author 可读不可写
- **WHEN** planning/author（Orchestrator）会话的 kimi 发起 fs/read 或白名单内 terminal 只读命令
- **THEN** auto 模式经 bwrap 沙箱执行、supervised 经 ApprovalBridge，正常返回结果

#### Scenario: planning/author 禁止写
- **WHEN** planning/author（Orchestrator）会话的 kimi 发起 fs/write
- **THEN** 拒绝并返回错误

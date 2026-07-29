## 1. 初始化 operation 后端契约与持久化

- [x] 1.1 实现持久化的代码库初始化 operation、固定五步状态机、终态恢复与同路径互斥语义。（需求：repository-initialization-progress）
- [x] 1.2 将 Cadence-skills 准备、Claude 初始化、Git 状态采集和 Repository 持久化接入 operation 的真实状态转换与失败诊断。（需求：repository-initialization-progress）
- [x] 1.3 将四个 Claude Code 初始化命令改为完整 `--no-interrupt` 提示词，并保留交互请求失败保护。（需求：non-interrupt-repository-bootstrap）

## 2. 异步 HTTP API 与客户端数据契约

- [x] 2.1 提供初始化 operation 的启动与查询 HTTP 契约，返回初始/轮询快照及成功或失败终态。（需求：repository-initialization-progress）
- [x] 2.2 更新前端 API 类型和客户端，以支持 operation 启动、查询和最终结果消费。（需求：repository-initialization-progress）

## 3. 五步初始化进度交互

- [x] 3.1 将添加代码库弹窗改为可访问的真实五步进度面板，并在非终态期间禁止关闭或取消。（需求：repository-initialization-progress）
- [x] 3.2 实现 operation 轮询、终态停止、短暂查询错误处理，以及成功后刷新代码库列表、失败后保留恢复信息的流程。（需求：repository-initialization-progress）

## 4. 回归测试与验证

- [x] 4.1 补充 Rust 单元与 Web 集成测试，覆盖 operation 创建/查询、步骤顺序、Cadence-skills 或命令失败、持久化失败、进程中断恢复和同路径互斥。（需求：repository-initialization-progress）
- [x] 4.2 补充 Claude 初始化器测试，确认四个独立命令均带 `--no-interrupt`，且交互事件失败。（需求：non-interrupt-repository-bootstrap）
- [x] 4.3 补充 React 组件与工作台测试，覆盖五步渲染、运行中不可关闭、轮询终态、成功刷新、失败恢复及无障碍状态播报。（需求：repository-initialization-progress）
- [x] 4.4 执行项目规定的格式化、静态检查、Rust/前端测试与 OpenSpec 验证，修复发现的问题。（需求：repository-initialization-progress、non-interrupt-repository-bootstrap）

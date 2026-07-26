## Context

Work Item Group 的初始编译已经将 Canonical Contract、Work Item Revision、验证计划 revision 与三种投影写入 revision store。现有 Workspace 上下文构建及部分执行链路仍读取 Lifecycle Store 的运行期 Work Item 记录。当前 Final Compile 在写入 revision 后直接提交 Plan 并创建子 Workspace，未生成该运行期投影，因而子 Workspace 的启动上下文在确认后失败。

本变更只面向之后新生成的 Work Item Group。已确认案例不自动回填、不修改其状态，也不调用 Provider。

## Goals / Non-Goals

**Goals:**

- 以已成功编译的 revision 为输入，生成可供既有 Workspace 与 Coding 链路消费的运行期 Work Item 投影。
- 在对外报告 Plan 已确认前，保证每个子 Workspace 都能完成启动上下文初始化。
- 使初始编译及其重试具备幂等性，避免重复运行时产生重复记录或悄然覆盖来源不一致的记录。
- 用端到端确认链路验证 Work Item，并确认 Story、Design Workspace 的既有行为不受影响。

**Non-Goals:**

- 不把 Lifecycle Store 重新定义为 Work Item 内容或版本的权威来源。
- 不迁移所有既有消费者到 revision store。
- 不回填、删除或修复本变更前已确认的 Work Item Group。
- 不改变 Provider Prompt、Provider 输出契约或前端 API。

## Decisions

### 1. 以运行期投影补齐兼容边界

Final Compile SHALL 从已发布的 Work Item revision、其 projection、Plan 来源以及 Issue/Repository 元数据构造运行期 Work Item 记录。正式 revision 保持唯一权威；运行期记录只承载既有执行链路需要的标识、来源、依赖、写入约束、验证引用和生命周期状态。

选择此方案是因为现有多处 Coding 与 Workspace 代码仍消费 Lifecycle Work Item。只让上下文构建直接读取 revision 会修复当前报错，但会把同一断链推迟到后续执行阶段。

备选方案：

- 只在上下文构建中回退读取 revision：影响小，但不能保证后续执行可用，拒绝。
- 一次性将全部消费者迁移到 revision store：长期更彻底，但影响面大，超出本次修复范围，拒绝。

### 2. 将投影、子 Workspace 与上下文就绪纳入确认门禁

确认路径必须在持久化 Plan 的已确认状态并向客户端发出成功结果前，完成运行期投影、子 Workspace 的确保创建和每个子 Workspace 的启动上下文初始化。任何一步失败都必须返回明确错误，并保持确认未完成或可恢复的状态，不得只因已写入 revision 就对外报告成功。

### 3. 运行期投影采用来源校验的幂等确保

同一初始编译重试时，已存在的运行期记录必须与计划、逻辑 Work Item、正式 revision 的来源标识一致；一致时复用，不一致时失败关闭。该规则避免恢复流程产生重复 Work Item，也避免静默覆盖其他来源的运行状态。

## Risks / Trade-offs

- [运行期投影与正式 revision 出现漂移] → 投影仅在发布边界生成，并保存可验证的来源标识；后续修订链路须复用同一投影边界或显式拒绝不兼容状态。
- [上下文初始化失败后留下部分记录] → 确保操作幂等，确认状态在全部子 Workspace 就绪前不提交；重试复用来源一致的记录。
- [修复范围漏掉共享 Workspace 行为] → 回归测试明确覆盖 Work Item 成功链路，并验证 Story、Design 继续按原有路径初始化。

## Migration Plan

1. 发布代码后，仅新的 Initial Final Compile 使用运行期投影。
2. 不自动处理历史已确认案例；历史案例维持现状，另行获得显式授权后才可提供独立恢复工具。
3. 若部署后发现投影来源校验错误，停止确认并保留可诊断错误；回滚代码不会修改已存在的正式 revision。

## Open Questions

无。当前范围、权威边界和不回填历史案例的策略已确认。

# 规范增量：多仓评审身份与评审窗口范围

<!-- 来源：cadence/designs/2026-10-22_技术方案_story-design-workitem评审修正_v1.2.md（commit 24fedc80）。
     本 delta 全部为 ADDED：不改 REQ-ENV-01/02/03/04/06/07/08/09（provider-desandbox-full-open、
     lc-root-initialization、fix-planning-session-anchor-recovery 正在改），不改 REQ-ENV-10/11/12
     （分别由 lc-root-initialization、aggregate-policy-root-publication 新增），不改 REQ-ENV-HOST-01/
     REVIEW-B/CHECK-01（provider-desandbox-full-open 新增）。
     REQ-ENV-14 与 REQ-ENV-REVIEW-B 的关系见本 change 的 design.md「与在途 change 的重叠面与归档顺序」。 -->

## ADDED Requirements

### Requirement: 多仓评审与 author 的会话唯一身份（REQ-ENV-13）

多仓库逻辑代码库下，评审角色与 author 角色的运行身份 MUST 为每个 workspace 会话新生成的唯一 id（UUID 形态），与该会话一一绑定。同一会话跨评审轮次、跨 repair MUST 使用同一个 id；存量会话缺少该身份时 MUST 补生成一次并持久化，MUST NOT 每次启动重新生成。会话删除即解绑，MUST NOT 依赖独立身份映射存储的清理。

评审身份 MUST 参与 `reviewer-verify` 的 canonical 摘要域；author 的 `resource_scope_id` MUST 由同一会话身份派生。

单仓库代码库 MUST 沿用既有窗口编号身份（`{attempt}-{role_run}-w01`）与既有 author 派生方式，canonical 摘要与改造前逐字节一致。

#### Scenario: 多仓同会话跨轮身份恒定

- **WHEN** 多仓会话先后启动第 1 轮评审与第 2 轮评审（含 repair）
- **THEN** 两轮 reviewer 摘要域取值 MUST 相同，且 MUST 为该会话的 UUID

#### Scenario: author 身份与评审身份同源

- **WHEN** 多仓会话启动 author run
- **THEN** 其 `resource_scope_id` MUST 由同一会话 UUID 派生

#### Scenario: 存量会话来自身份只生成一次

- **WHEN** 多仓存量会话首次被读取且尚无身份值
- **THEN** 系统 MUST 生成一个身份并持久化，后续读取 MUST 复用该值，MUST NOT 生成第二个

#### Scenario: 会话删除即解绑

- **WHEN** 多仓会话被删除
- **THEN** 该身份 MUST 不再被任何持久记录引用，MUST NOT 存在需单独清理的身份映射

#### Scenario: 单仓身份不变

- **WHEN** 单仓会话进入评审并启动 author run
- **THEN** reviewer 身份 MUST 仍为窗口编号，author 身份 MUST 仍为既有 session 派生，canonical 摘要 MUST 与改造前逐字节一致

### Requirement: 多仓文档环节评审不开评审窗（REQ-ENV-14）

多仓库逻辑代码库下，Story / Design / Work Item 三环节评审 MUST NOT 开启评审窗、MUST NOT 生成评审窗记录、MUST NOT 执行基线快照 / 对账 / 恢复，MUST NOT 产出 `plan_review_window_*` 类错误码或窗口注意力提示。

该类评审是只读材料评审，不进入需要验证执行面的审查窗口档；coding 侧审查仍受既有审查窗口约束。

多仓评审无法解析材料集合或材料渲染失败时，系统 MUST 以 `plan_review_material_unresolved: <原因>` 显式阻断该轮，MUST NOT 启动 provider。

单仓库代码库 MUST 保留既有评审窗（开启、关闭、注意力提示、崩溃后的零锁恢复）与 `plan_review_window_*` 错误码，行为逐项不变。

#### Scenario: 多仓无窗口记录与窗口错误码

- **WHEN** 多仓会话完成任意一轮三环节评审
- **THEN** MUST NOT 产生评审窗记录、窗口事件、窗口注意力提示或 `plan_review_window_*` 错误码

#### Scenario: 多仓材料缺失显式阻断

- **WHEN** 多仓评审无法解析材料集合或材料渲染失败
- **THEN** 系统 MUST 以 `plan_review_material_unresolved: <原因>` 阻断该轮，MUST NOT 启动 provider，MUST NOT 静默以空材料继续

#### Scenario: 多仓做基线对账恢复

- **WHEN** 多仓文档环节评审开始或中断后恢复
- **THEN** 系统 MUST NOT 建立基线快照、MUST NOT 执行对账与恢复，恢复 MUST 直接复用既有多仓材料解析路径

#### Scenario: 单仓窗口与零锁恢复不变

- **WHEN** 单仓会话进入评审，或在评审中途崩溃后恢复
- **THEN** 评审窗记录与 window id、窗口早返回路径、注意力提示、`plan_review_window_*` 错误码与零锁恢复 MUST 与改造前一致

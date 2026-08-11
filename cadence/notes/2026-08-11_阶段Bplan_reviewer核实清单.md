# 阶段 B Implementation Plan Reviewer 核实清单

## 审核对象
`cadence/plans/2026-08-11_计划文档_聚合规划Web接入_阶段B_v1.0.md`（10 Task）

## 设计依据（已 Approved）
`cadence/designs/2026-08-11_方案设计_聚合规划Web接入_阶段B_v1.0.md`（v1.2，方案 X 两阶段创建）

## 审核重点（只看这些，勿发散）

### 1. Task 分解与依赖是否合理
- 10 Task 的顺序与依赖（Task 1-4 B1 基础 → 5-7 接线 → 8 B2 → 9 B3 → 10 B6）是否正确？
- 有无循环依赖或缺失前置？
- 每个 Task 是否自包含（有独立测试周期、可独立 review）？

### 2. 代码准确性（关键，plan 不能有错的代码指引）
重点核实这几个 Task 的代码块是否与实际代码结构吻合：
- Task 1：`validate_aggregate_story_scope`（spec.rs:390）改签名传 status，create_story_spec（:45）/ create_design_spec（:129）调用点
- Task 4：`provider_drive.rs:775-790` 的 Story|Design 分支是 append_version 的精确落点吗？
- Task 5：issue_lifecycle（lifecycle.rs:14-75）:58 确实无条件要求 repo_id？generate_story_specs（:350）:379 确实 aggregate_codebase: None？
- Task 8：compile_support.rs:278-390 的 depends_on 构建处

### 3. spec 覆盖（Self-Review 清单是否属实）
- REQ-PLN-04 → Task 5
- REQ-PLN-05 → Task 5+6+8
- REQ-TGT-01 → Task 7
- REQ-TGT-03 → Task 7+8
- REQ-TGT-04 → Task 8
- REQ-TGT-05 → Task 9
有无遗漏？

### 4. 方案 X 4 实施约束是否落实进 Task
- 约束1（validate 传 status）→ Task 1
- 约束2（Story+Design 都放宽）→ Task 1
- 约束3（nonce+serde）→ Task 3
- 约束4（回写不吞错）→ Task 4

### 5. 潜在风险
- 有无 Task 代码块引用了不存在的类型/方法（占位符或臆造）？
- 有无遗漏的关键步骤（如 Routing 三态判定、单仓兼容性保护）？
- 前端 Task 9 与后端 work_item_repository_groups 的 DTO 形状是否对齐？

## 输出（简短）
1. Task 分解/依赖：合理/有问题（指出）
2. 代码准确性：逐个核实点 OK/有错（指出 Task 号 + 错误）
3. spec 覆盖：完整/遗漏（指出）
4. 约束落实：4 个都落实/有遗漏
5. 新风险：无/列出
6. 总判定：Approved（可执行）/ 需修改

**约束：只读 plan + 上述代码位置核实，不要重读设计文档全文，不要跑 cargo。**

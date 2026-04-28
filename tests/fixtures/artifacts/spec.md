# Spec

## 功能需求

| ID | Text | Priority |
|----|------|----------|
| REQ-001 | 用户可以通过 REPL 创建任务。 | must |

- [REQ-002] daemon 必须生成稳定 change_id。Priority: must

## 成功标准

- [AC-001] 输入 `new_task` 后返回 task_id、phase、intake_ref、change_id。Refs: REQ-001, REQ-002

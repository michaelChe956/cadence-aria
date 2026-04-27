# Design

## 设计决策

- [DD-001] REPL 只作为客户端，daemon 是 runtime truth。Refs: REQ-001

## 公共组件

| ID | Name | Responsibility |
|----|------|----------------|
| CMP-001 | repl_wire | JSON envelope schema |

## 风险

- [RISK-001] REPL 断连后 daemon 状态不一致。Severity: high; Refs: DD-001

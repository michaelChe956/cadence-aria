# Tasks: single-candidate-lowering-safety

**全局边界**：不改 validator coverage 口径/prompt/options/fingerprint（C1 域）；不改 freshness 语义；历史 IR/publication 不迁移。

## 1. lowering 累积语义

- [ ] 1.1 `lower.rs` outputs/inputs 重复字段 append+first-seen-dedup helper + contract 边界 flush；TDD：多行能力/完全重复/相邻 contract/字段错位/空值/未知字段负向场景（REQ-WSC-02 新场景 6-9）。
- [ ] 1.2 F-52 真实形态 fixture（CT-001 6 行）端到端：不再产生 missing_capabilities 假阳性；真实缺能力仍 Error（场景 5 红绿）。
- [ ] 1.3 重编译 IR 差异仅源于重复字段语义的断言（场景 10）；freshness 零回归。
- [ ] 1.4 change strict + compiler/validator 目标测试 + fmt/clippy；记录证据。

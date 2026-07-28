## 1. 清理语义回归测试

- [x] 1.1 为单 unit 已认领 handoff 的 attempt 删除编写失败测试，断言删除后 lineage 中该 handoff revision 不存在。
- [x] 1.2 为多 unit 均已认领 handoff 的 attempt 删除编写失败测试，断言全部对应 handoff revision 被删除。
- [x] 1.3 为无 unit 认领 handoff 的 attempt 删除编写测试，断言删除正常完成且不删除任何 handoff revision。
- [x] 1.4 为归属校验编写测试，断言 unit 指针指向的 handoff revision 归属不符时不被删除。
- [x] 1.5 为计划编译产物编写回归测试，断言删除 attempt 后 plan revision、work item revision、projection bundle、verification plan revision 与 dependency graph revision 全部保持存在且内容不变。
- [x] 1.6 为删除后重建编写测试，断言重跑同一 work item 可成功发布 handoff revision 且不返回冲突错误。

## 2. 生产实现

- [x] 2.1 在 handoff revision 存储中新增删除能力，含归属校验，且不改动既有不可变写入与读取路径。
- [x] 2.2 在 attempt 删除流程中于 attempt 记录删除之前调用清理，遍历各 unit 的已认领 handoff revision。
- [x] 2.3 确认未改动 handoff revision 发布路径、ID 派生规则、group completion 判定，未引入跨 attempt 引用扫描，未暴露通用删除 API。

## 3. 验证与交付

- [x] 3.1 运行本 change 相关定向测试与既有 group completion、attempt 删除、lineage 存储回归，并区分既有失败基线。
- [x] 3.2 严格校验 OpenSpec change 并完成代码审查。
- [ ] 3.3 经用户确认后重启后端，由用户验证删除 attempt 后重建可正常完成 work item。

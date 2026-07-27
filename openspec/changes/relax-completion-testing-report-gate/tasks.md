## 1. 完成门禁回归测试

- [ ] 1.1 新增 schema v2 group 回归测试，证明 required verification check 存在但无 testing report 时仍可通过完成门禁。
- [ ] 1.2 处理断言旧 testing-report 门禁语义的 legacy 集成测试，显式记录其退役状态与既有 fixture 阻塞原因。
- [ ] 1.3 确认非 testing 完成门禁回归用例继续覆盖文件范围、handoff/unit 状态与 worktree 清洁性。

## 2. 放宽完成判定

- [ ] 2.1 从 schema v2 group、legacy group 与 single-attempt 完成路径移除 testing report 强制校验。
- [ ] 2.2 删除专用于完成判定的 testing 校验函数，并保留 `VerificationGateResultMissing` 公共错误变体及 Testing 基础设施。
- [ ] 2.3 确认 Internal Reviewer、final confirm、handoff 与其他完成状态流转未被改变。

## 3. 验证与运行时交付

- [ ] 3.1 运行定向 TDD、全量 lib、格式化、clippy 及相关集成测试，记录既有无关失败与本 change 结果。
- [ ] 3.2 严格校验 OpenSpec change，并完成代码审查。
- [ ] 3.3 经用户确认后重启后端，在现有或新 attempt 上手动验证 internal PR review 通过后可显示执行成功。

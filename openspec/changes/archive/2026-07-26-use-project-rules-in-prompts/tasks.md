## 1. 集中项目规则引用

- [x] 1.1 将集中规则 prompt 段改为要求直接读取当前目标仓库的 `AGENTS.md` 与 `CLAUDE.md`，并在任一文件不可读时失败关闭。（需求：project-rule-aware-prompts）
- [x] 1.2 为集中规则段补充正反向契约测试，覆盖项目规则、直接读取、阻塞语义及外部路径/知识库禁令移除。（需求：project-rule-aware-prompts）

## 2. 生命周期 Prompt 契约

- [x] 2.1 更新 Workspace、Coding、Tester、Work Item Draft、Web Workspace Context 与运行时模板的复用断言，确保既有业务阶段和输出契约保持不变。（需求：project-rule-aware-prompts）
- [x] 2.2 检索并移除所有运行时 prompt 与测试中对旧外部规则标签、绝对路径及知识库禁令的依赖。（需求：project-rule-aware-prompts）

## 3. 验证

- [x] 3.1 运行定向 Rust prompt 契约测试、格式与静态检查，并执行 OpenSpec 严格验证与 diff 检查。（需求：project-rule-aware-prompts）

## 1. 落盘位置回归测试

- [x] 1.1 为 adapter 提供流日志目录的场景编写失败测试，断言流日志写入所提供目录，且 provider 工作目录下不出现流日志目录。
- [x] 1.2 为未提供流日志目录的场景编写测试，断言不写任何流日志文件且不回退到 provider 工作目录。
- [x] 1.3 为命名与追加写入编写测试，断言 stdout 与 stderr 按 provider 名、子进程标识与流名区分；追加模式在同一路径上直接验证（跨进程执行 child_id 不同，集成测试无法区分 append 与 truncate）。
- [x] 1.6 为非绝对路径（空串、相对路径）编写测试，断言按未提供处理且不回退到任何工作目录。
- [x] 1.4 为 coding attempt 的流日志目录解析编写测试，断言其位于该 attempt 根下且与 `provider-raw` 同根。
- [x] 1.5 为无 attempt 上下文的执行路径编写测试，断言不向其工作目录写入流日志。

## 2. 生产实现

- [x] 2.1 在 `AdapterInput` 新增可选流日志目录字段，并补齐全部构造点。
- [x] 2.2 改造 `provider_stream_path` 与其调用点，改为消费传入目录，移除基于 provider 工作目录的推导。
- [x] 2.3 在 coding attempt store 新增 attempt 级流日志目录解析。
- [x] 2.4 在全部持有 attempt 上下文的执行路径填入该 attempt 的流日志目录（handoff、coder、code review、rework、internal review、testing plan/report），并提取共用 helper；确认无 attempt 上下文的调用点不提供该字段。
- [x] 2.6 对非绝对流日志目录按未提供处理，避免退化为写入 Aria 进程的当前工作目录。
- [x] 2.5 确认未改动流日志命名与写入模式、`provider-raw` 落盘、provider 工作目录与 streaming provider 路径。

## 3. 验证与交付

- [x] 3.1 运行本 change 相关定向测试与 cli adapter、coding provider 执行、attempt 存储既有回归，并区分既有失败基线。
- [x] 3.2 运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings` 与 `cargo test --locked`。
- [ ] 3.3 严格校验 OpenSpec change 并完成代码审查。
- [ ] 3.4 经用户确认后重启后端，由用户验证目标代码库不再生成流日志目录。

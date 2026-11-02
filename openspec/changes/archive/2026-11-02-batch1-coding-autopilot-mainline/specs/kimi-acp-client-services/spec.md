# Spec Delta

> 基线：`openspec/specs/kimi-acp-client-services/spec.md` 现有 wire 契约冻结、终端生命周期、终端执行策略、终端 OS 级隔离等 requirement。本 delta 只新增一项 requirement，不修改既有 requirement；「终端 OS 级隔离（auto 模式）」的既有 host 只读与 target 绑定语义保持不变，本 delta 只修正挂载顺序与新增可选只读上游根。

## ADDED Requirements

### Requirement: 终端沙箱只读上游挂载顺序（REQ-KACS-IDLE-RO）

Kimi Auto terminal（含 LC）经 bwrap 构造沙箱时，系统 SHALL 按「host 只读 → proc/dev/tmpfs `/tmp` → authority 只读（`readable_root`）→ target 绑定（唯一可写）→ git binds → cwd fd」的顺序生成 argv，使被 tmpfs 遮蔽的 authority/上游根恢复只读可见。`readable_root` SHALL 仅取可信 boundary 的 authority root；direct（无沙箱）模式传 None，但同样 MUST NOT 把 target 之外的路径变为可写。native fs 读面（authority 读、target 写）保持既有行为。

#### Scenario: 上游只读且 target 唯一可写

- **WHEN** Auto terminal 在含上游依赖的聚合 fixture 中启动并执行读取与写入
- **THEN** terminal 与 native Read 均能读到真实上游代码；对上游路径写入失败；对本仓 target 写入成功
- **AND** bwrap argv 顺序断言为 host ro → proc/dev/tmpfs → authority ro → target bind（纯 argv 单测，无需真跑沙箱）

#### Scenario: direct 模式不扩写面

- **WHEN** Kimi direct 模式（无 Bubblewrap）在聚合 fixture 中运行
- **THEN** `readable_root` 为 None，角色读/写判定与 cwd fd 行为与既有一致，不新增可写根

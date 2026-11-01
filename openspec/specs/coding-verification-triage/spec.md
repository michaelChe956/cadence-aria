# coding-verification-triage Specification

## Purpose

定义 coding 阶段验证类问题的产品化分诊：并列展示计划命令与实际执行命令、支持重跑原计划命令，并以独立、受绑定、需人批准的验证处理／受限豁免入口替代直写 context note 或清空 findings 的旁路。

## Requirements

### Requirement: 计划命令与实际命令并列证据（REQ-CVT-01）

当 coding 阶段因验证失败、验证证据不完整或 coder 输出需分诊而停等时，产品面 SHALL 对每个相关验证 check 并列展示：计划合同中的 check 标识与命令（或人工检查说明）、coder 实际执行的命令、cwd、退出码、测试执行数量与环境摘要（来自该运行已记录的工具调用与输出证据）。实际命令与计划命令不一致时，系统 SHALL 明确标注“实际执行命令与计划不一致”，MUST NOT 把该情形归类为计划缺陷。证据缺失的字段 SHALL 显示为“未记录”，MUST NOT 以推测值填充。

#### Scenario: coder 在错误目录执行命令

- **WHEN** 计划 check 的命令合法，但 coder 实际在不同 cwd 或以不同参数执行导致失败
- **THEN** 产品面并列显示计划命令与实际命令、cwd 和退出码，并标注“实际执行命令与计划不一致”，不显示为计划缺陷

#### Scenario: 实际命令证据缺失

- **WHEN** 该运行没有记录到对应 check 的实际命令
- **THEN** 实际命令栏显示“未记录”，系统不推断或补写命令

### Requirement: 重跑原计划命令（REQ-CVT-02）

在 REQ-CVT-01 的等待面上，系统 SHALL 提供“重跑原计划命令”操作：该操作以返修／重试既有路径让 Coder 在当前 attempt 与当前 Work Item 写入策略下按计划合同字面命令与 cwd 重新执行验证，并把计划命令作为明确指令纳入实际 prompt。该操作 SHALL 携带 `command_id`、gate 与 check 身份；MUST NOT 改写计划合同、清除 finding 或跳过 Code Review。系统 MUST NOT 为此新增服务端通用命令执行器。

#### Scenario: 重跑后通过

- **WHEN** 用户对“实际命令与计划不一致”的等待项点击“重跑原计划命令”，Coder 按计划命令执行且验证通过
- **THEN** 原链继续进入后续阶段，原 finding 与原运行证据保留可查

#### Scenario: 重复点击

- **WHEN** 同一 `command_id` 的重跑请求被重复提交
- **THEN** 系统只触发一次返修运行并返回同一结果

### Requirement: 独立验证处理入口（REQ-CVT-03）

当计划字面验证命令在当前环境不可满足时，系统 SHALL 从原门提供受控转入独立“验证处理”面的入口；该入口 MUST NOT 改变原门动作集合：`coding_output_human_triage` 门 SHALL 仍恰好提供 `retry_coding` 与 `abort`，三个 Code Review 分诊门 SHALL 仍恰好提供 `send_to_coder`、`retry_review`、`manual_continue` 与 `abort`。验证处理记录 SHALL 绑定 finding、check、plan revision、原命令、替代命令（如有）、cwd、执行结果、测试执行数量、环境与适用 scope。同一 finding／check／plan revision 在已有未决验证处理时 SHALL 拒绝重复转入。

#### Scenario: 从 coder 输出门转入

- **WHEN** coder 输出门因计划字面命令不可满足而打开，用户点击“进入验证处理”
- **THEN** 系统创建唯一的验证处理记录并绑定上述字段，原门动作集合保持 `retry_coding`／`abort` 不变

#### Scenario: 从 Code Review 门转入

- **WHEN** Code Review 验证不完整门打开，用户点击“进入验证处理”
- **THEN** 系统创建或返回同一验证处理记录，原门四动作集合不变

#### Scenario: 重复转入被拒

- **WHEN** 同一 finding／check／plan revision 已存在未决验证处理，用户再次转入
- **THEN** 系统返回既有记录，不创建第二条

### Requirement: 验证处理结论需人批准且受限（REQ-CVT-04）

验证处理 SHALL 只接受三类由用户明确批准的结论：批准计划修订（经既有计划修订／amendment 路径）、接受可信等价证据、授予限域环境例外。可信等价证据 SHALL 包含替代命令、cwd、非零退出以外的明确结果与测试执行数量；当 check 要求非零测试执行时，测试执行数量为零的证据 SHALL 被拒绝。限域环境例外 SHALL 只覆盖所绑定的 check 与 scope，SHALL 记录理由、操作者与时间。缺少批准、plan revision 不匹配、scope 宽于所绑定 check、证据不完整时，系统 SHALL 停等并返回具体原因。批准后系统 SHALL 追加审计记录并使原链继续；MUST NOT 清除或改写历史 finding、旧 execution、旧 hash、旧门证据，MUST NOT 让模型隐瞒或删除失败事实。FinalConfirm SHALL 仍由人执行，并可见全部验证处理记录。

#### Scenario: 接受可信等价证据

- **WHEN** 用户提交替代命令、cwd、通过结果与非零测试数量，并批准为可信等价证据
- **THEN** 系统记录审计、原 finding 保留并标注已由验证处理覆盖，原链继续

#### Scenario: 零测试证据被拒

- **WHEN** check 要求非零测试执行，而提交的等价证据测试执行数量为零
- **THEN** 系统拒绝该结论并保持停等

#### Scenario: 错 revision 或过宽 scope 被拒

- **WHEN** 提交的结论绑定的 plan revision 已过期，或申请的豁免 scope 覆盖所绑定 check 之外的路径或 check
- **THEN** 系统拒绝该结论，验证处理与原门保持停等

#### Scenario: 无批准不能继续

- **WHEN** 验证处理尚无用户批准的结论
- **THEN** 原链保持停等，不自动推进

### Requirement: 验证与计划路径错误文案口径（REQ-CVT-05）

coding 与验证相关的错误与等待文案 SHALL 区分以下情形并给出对应下一步：“实际执行命令与计划不一致”（指向重跑原计划命令）、“计划未声明该路径／命令”与“计划路径不可执行”（指向计划反馈／修订或验证处理）。文案 MUST NOT 建议升级运行时版本、泛化重试或“忽略 finding”作为这些情形的处置。

#### Scenario: 计划未声明路径

- **WHEN** coding 因计划未声明所需路径而停等
- **THEN** 文案显示“计划未声明”并提供计划反馈／修订入口，不建议泛化重试

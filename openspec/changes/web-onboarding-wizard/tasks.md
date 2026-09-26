# Tasks

## 1. 配置、存储与向导基础（REQ：首次判定、配置化、跳过重开）

- [ ] 1.1 建立 onboarding 专用配置与类型，集中定义 13 个手动模式步骤的顺序、中文标题/说明、截图或示意资源、锚点 `data-testid` 与 feature-gate/late-bind；声明默认关闭的 `automation-mode-selection` 步骤，并为配置过滤后的进度计算提供可测试接口。
- [ ] 1.2 按 `useWhatsNew` 的静默降级模式实现 `aria-onboarding-seen` 设备级读写、首次自动展示、跳过/完成记忆与帮助入口重开；localStorage 异常不得阻断工作台，重开不得改变任何业务状态。
- [ ] 1.3 实现无第三方依赖的 wizard 呈现与锚点测量：说明卡、当前位置/总数、上一步/下一步、跳过/完成、截图/示意降级、缺失锚点提示、视觉高亮和可访问 testid；步骤切换和 resize/scroll 时重新测量并在卸载时清理。

## 2. Shell 接入与 13 步锚点契约（REQ：锚点高亮、非侵入边界）

- [ ] 2.1 在 `AppShell`/`WorkbenchSurface` shell 外围接入首次引导与可访问帮助入口（含稳定 `onboarding-help-trigger`），不进入生命周期状态机，不新增 API/协议；验证已有 WhatsNew 与工作台并存时的层级、关闭和键盘焦点行为。
- [ ] 2.2 为 Project 新建、单仓代码库添加、Issue 新建（含基准分支说明）、Story/Design/Work Item Plan 阶段与生成/确认入口补齐或固定 onboarding 锚点；其中 Work Item Plan 第一、第二道确认门必须是两个不同锚点。只增加稳定 `data-testid` 属性，不改变动作、请求或既有 testid。
- [ ] 2.3 为进入 Coding Workspace、启动 Coding、Coding 进度/完成确认区域补齐稳定锚点；若目标在独立 Coding Workspace 路由中不可见，按设计提供非阻塞缺失提示，不修改 Coding 业务动作。
- [ ] 2.4 明确并验证受保护边界：不得改动 `useCockpitAutopilot`、`workspace-cockpit-projection`、`CockpitInbox`、`ChatCockpitPage` 内部，也不得为自动化模式选择步骤添加绕行 UI；该步骤仅保留默认关闭的配置，待 P1 稳定锚点后 late-bind 启用。

## 3. 测试、回归与验收证据（REQ：全部行为场景）

- [ ] 3.1 覆盖配置顺序、13 步手动模式、自动化步骤 feature-gate/late-bind 过滤、锚点映射和双计划门锚点的单元/组件测试。
- [ ] 3.2 覆盖首次 localStorage 判定、已读不再自动弹出、存储异常静默降级、跳过/完成写入、帮助入口重开和业务状态不变的测试；覆盖缺失锚点仍展示说明且不抛错。
- [ ] 3.3 运行前端既有相关测试与 `openspec validate --strict`，并以实际工作台 surface 冒烟验证首次弹出、13 步导航、跳过、帮助重开、高亮/缺失提示及既有驾驶舱通知/收件箱未被改变。

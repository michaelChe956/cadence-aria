# Tasks：高层工作包与验收映射

关联 Plan：`cadence/plans/2026-11-13_计划文档_修复Provider安装无效果与聚合索引截断_v1.0.md`。

**状态：draft，WP0 为本次文档交付，其余均未实施。** 实施必须先完成接地审查 + 一致性裁决双审；同文件切片串行、独占文件派工，worker 不跑全量门禁，主 agent 汇总验证。

## 必做工作包

- [x] **WP0：OpenSpec change + Plan** — 持久化 D1/D2/D3/D4、根因、边界、测试、风险与清理；映射 REQ-PCI-02/03/05、REQ-IND-01/04。交付仅指定 7 个文档。
- [ ] **WP1：平台级 PATH overlay**（依赖 WP0 双审）— 单一线程安全合并函数，解析预检与 spawn 同一结果；WP1a 为组件/导出/异步底座（`process_manager.rs` 是所有有界命令的汇聚点，覆盖探针/安装/codegraph 调用），WP1b 为共享同步 turn 底座（`cli_adapter.rs`），均不触碰四家 provider 目录；映射 REQ-PCI-05。
- [ ] **WP2：前缀选择与真实可写性**（依赖 WP1）— 可见真实前缀优先，npm 全局 bin 必须已在服务有效 PATH，不能以 mode 位替代运行身份写能力；保持镜像/单包/无提权；映射 REQ-PCI-03。
- [ ] **WP3：绝对重探、Kimi 目录/提示与启动恢复**（依赖 WP2）— 安装定位单源、登记后绝对探测、health 刷新、Kimi 整句提示，bind 成功后首次探活前恢复实际安装目录；安装 ≠ 能力核验；映射 REQ-PCI-02/05。
- [ ] **WP4：前端完整提示与真实交互回归**（依赖 WP3 接口冻结，可与 WP5 并行）— 原样显示后端整句，保留 Kimi 来源确认与安装/更新/核验矩阵；终态重读卡片，codegraph 入口不无限重装；映射 REQ-PCI-02/05。
- [ ] **WP5：CodeGraph 完整文件清单**（依赖 WP0；实施顺序在 WP4 后，可独立并行）— **先在容器对 1.6.0 与 1.6.2 验证 `--no-metadata` 的数组/path/非 NDJSON 契约，失败立即停止上报 owner**，不得删旗标、改版本门或做单版本特判；前置通过后 TDD 实现 stdout_truncated、files 专用 64 MiB 与瘦身参数，并实现**能力探测式降级**（旗标被拒时同一次调用内去旗标重试一次、预算不变、降级入诊断，禁止版本号特判）；映射 REQ-IND-01/04。
- [ ] **WP6：专用截断文案与失败生命周期回归**（依赖 WP5）— 新增中文码表，warning 保留机器码/预算；验证首建 failed 与 LKG degraded；可选透传 bootstrap reason_code，不新增卡型；映射 REQ-IND-04。
- [ ] **WP7：podman A/B/C 全场景 + 页面验收**（依赖 WP1–WP6）— A 四家/npm 目标初始未装，B Kimi 官方脚本，C 真 codegraph 大负载与超限；使用宿主模型配置临时复制件只读挂载及本地网关，不碰宿主原件；只用确认空闲的 14317，不碰共享 4317；映射全部本 change requirements。
- [ ] **WP8：门禁与清理**（依赖 WP7）— 主 agent 一次汇总标准 Rust/前端门禁、页面证据和资源结果；按精确清单删除本次容器、镜像、tmp，诊断遗留待证据回填后清理，不动既有资源；映射全部本 change requirements。
- [ ] **WP9：OpenSpec sync/archive、CHANGELOG 与 Plan 结果回填**（依赖 WP8、独立终审通过）— 同步 delta，双审处理主规格版本文案的既有漂移，归档并记录实际命令/结果与清理状态；映射全部本 change requirements。

依赖主序：**WP0 → WP1 → WP2 → WP3 → WP4 → WP5 → WP6 → WP7 → WP8 → WP9**。WP4 与 WP5、已冻结接口的不同文件切片可并行；WP7 A/B/C 使用同一 host 端口，必须串行运行。

## UI 验收状态矩阵（WP4 / WP7，REQ-PCI-02/05）

| 状态 | 用户看到/能点什么 | 不得出现什么 |
|---|---|---|
| 未安装 | 「安装」+ 手动命令；点击发异步 POST | 「更新」 |
| Kimi 点击后未确认 | code.kimi.com 来源提示、确认/取消 | 未确认即发安装请求 |
| running | 安装中/数分钟提示，按钮禁用，轮询进度 | 重复提交 |
| completed，已可用未核验 | 真实版本 + 整句 PATH 提示；显式「核验」 | 仍「未安装」、自动 Confirmed |
| completed，已核验版本落后 | 「更新」+ 本地/最新版本 | 「安装」 |
| completed，已核验最新 | 不显示安装/更新 | 无意义重装 |
| failed | 失败分类、诊断、手动命令与可重试入口 | 伪报成功/吞掉错误 |
| codegraph 可用 | 入口刷新可用性，不再「安装」 | 因截断码显示安装入口（截断不是工具缺失） |

真实浏览器必须点击并观察上述分支，不得只用 HTTP 200 或源码断言代替页面证据；禁止调用 Playwright。开发态用 5173，容器重建 dist 后发布态回归用隔离端口 14317。

## 可选工作包

- [ ] **WP10：启动 stale running 安装作业对账**（依赖 WP9；**未纳入必做、未实施**）— `InstallJobStore` 没有启动扫描。单独授权后才设计并执行一次性终结/诊断，不自动重跑远程脚本；映射 REQ-PCI-05 durable 生命周期。不得用该可选项延后 WP1–WP9。

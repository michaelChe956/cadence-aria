# Tasks: spec-workbench-canvas-experience

## 1. 设计规范与 token 落地

- [ ] 1.1 ui-ux-pro-max 生成 design-system/MASTER.md（按 design §3 的 token 映射定制：蓝紫橙/奶油底/粗边框/圆角/胶囊/反模式清单）
- [ ] 1.2 `web/src/styles.css`：--aria-* 变量值更新（主色 #4F46E5、新增 --aria-cta #F97316、底色 #f5f3ee、边框加深加粗变量、圆角变量），tailwind.config 如需扩展同步

## 2. 基础组件形态收敛

- [ ] 2.1 按钮体系：btn-primary（蓝紫实心+边框）/btn-secondary（白底粗边框）标准类，ChatInputBar 与面板操作条按钮统一套用
- [ ] 2.2 卡片/面板/chip：clay 形态类（2-3px 深色边框 + rounded-xl/2xl + 柔和单层阴影），版本号/阶段标签改胶囊 chip

## 3. Canvas 产物审核面板

- [ ] 3.1 面板组件：ArtifactPane 放大为右侧滑出面板（工具条 + 改动摘要折叠条 + artifact 渲染 + 吸顶操作条），随 author_confirm 阶段滑出/收起（CSS transition 150-300ms）
- [ ] 3.2 阶段驱动接线：ChatWorkspacePage 按 stage 驱动面板开合（author_confirm 开、输入聚焦/运行开始收、送审定稿后收）；左侧节点栏保持常驻
- [ ] 3.3 三动作迁移：送审/定稿/采纳移入面板操作条（默认推荐主次样式），发送反馈留对话输入区，返回对话反馈钮

## 4. 测试与收尾

- [ ] 4.1 组件测试：面板阶段驱动开合、吸顶可见性、改动摘要条、三动作行为复测（既有断言随结构调整更新）
- [ ] 4.2 全量验证：cd web && pnpm tsc -b && pnpm test；cargo check --locked 保持绿；whats-new 更新

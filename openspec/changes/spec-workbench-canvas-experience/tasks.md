# Tasks: spec-workbench-canvas-experience

## 1. 设计规范与 token 落地

- [ ] 1.1 ui-ux-pro-max 生成 design-system/MASTER.md（按 design §3 的 token 映射定制：蓝紫橙/奶油底/粗边框/圆角/胶囊/反模式清单）
- [ ] 1.2 `web/src/styles.css`：--aria-* 变量值更新（主色 #4F46E5、新增 --aria-cta #F97316、底色 #f5f3ee、边框加深加粗变量、圆角变量），tailwind.config 如需扩展同步

## 2. 基础组件形态收敛

- [ ] 2.1 按钮体系：btn-primary（蓝紫实心+边框）/btn-secondary（白底粗边框）标准类，ChatInputBar 与面板操作条按钮统一套用
- [ ] 2.2 卡片/面板/chip：clay 形态类（2-3px 深色边框 + rounded-xl/2xl + 柔和单层阴影），版本号/阶段标签改胶囊 chip

## 3. Canvas 产物审核面板

- [ ] 3.1 面板组件：ArtifactPane 放大为右侧滑出面板（工具条 + 改动摘要折叠条 + artifact 渲染 + 吸顶操作条），随 author_confirm 阶段滑出/收起（CSS transition 150-300ms）；改动摘要取最近 completed revision 节点 summary，无摘要整条隐藏
- [ ] 3.1b 互斥 Tab 改造：`activePanel: "chat" | "artifact"`（ChatWorkspacePage.tsx:154,590）语义调整——author_confirm 时 chat 与面板并存，Tab 仅在非 author_confirm 阶段生效
- [ ] 3.2 阶段驱动接线：ChatWorkspacePage 按 stage 驱动面板开合（author_confirm 开含重连恢复、输入聚焦/送审/运行开始/定稿收）；「采纳 Review 意见」点击时预填同时自动收起面板；左侧节点栏保持常驻
- [ ] 3.3 三动作迁移：确认送审/确认定稿（文案保留现状）/采纳 Review 意见移入面板操作条（默认推荐主次样式），发送反馈留对话输入区，返回对话反馈钮

## 4. 测试与收尾

- [ ] 4.1 组件测试：面板阶段驱动开合（含重连恢复滑出）、吸顶可见性、改动摘要条渲染与隐藏、采纳预填自动收起、三动作行为复测（既有断言随结构调整更新）；样式人工审查项：无 emoji 装饰/无 hover 放大/无双层阴影/无衬线正文
- [ ] 4.2 全量验证：cd web && pnpm tsc -b && pnpm test；cargo check --locked 保持绿；其他工作台关键页冒烟（coding/image-create 渲染不炸、对比度不劣化）；whats-new 更新

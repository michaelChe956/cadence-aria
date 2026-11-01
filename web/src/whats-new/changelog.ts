export interface ChangelogEntry {
  version: string;
  date: string;
  title: string;
  highlights: string[];
}

export const CURRENT_VERSION = "0.1.1";

// 发布升版本时请裁剪数组，仅保留最新 4 条。
export const CHANGELOG: ChangelogEntry[] = [
  {
    version: "0.1.1",
    date: "2026-11-01",
    title: "版本更新说明面板接入产品内 changelog 数据源",
    highlights: [
      "「版本更新说明」面板此前读的是旧数据源，0.1.0 的更新内容在面板里看不到；本次接入产品内 changelog 数据源，0.1.0 的多仓全链路说明现已可见",
      "面板按既有约定只保留最近 4 个版本；仓库根目录 CHANGELOG.md 为长版全文（发版时两处需同步）",
    ],
  },
  {
    version: "0.1.0",
    date: "2026-11-01",
    title: "多仓全链路对齐：判定一致、失败可执行、通过率可查",
    highlights: [
      "多仓（逻辑代码库）全链路对齐：同一份候选在候选结构门与最终编译现在判定一致（此前一个说无环、一个说成环，导致「写 plan 成功但 review 后报依赖环」）",
      "依赖环不再让你去改 design：给出环上真实链、各 Work Item 所在仓的仓序位次、逆仓序那一跳，并给 2 个各带后果的合法修法，附保护项（不得为消环删除真实数据流、不得改动仓序）",
      "结构门拒收不再直接「生成失败」：改为落到人工门并带上可执行修法；模型另有一次自动重驱机会（带环的事实与修法回去，常能自己改对，owner 零点击）",
      "通过率可查：每次候选尝试都进 append-only 流水（落 issue 级，重跑不会擦掉历史），并提供 issue 级 / 项目级两个只读读数出口：首次通过率、同 issue 连续失败次数 P50/P90、失败码 Top N、lowering 类终态失败计数",
      "过期门卡不再误导：计划已确认完成后，对话流里残留的旧卡改为「这一版已确认完成，此卡是历史记录，无需处理」，不再让你去终止此门重新发起",
      "缺陷修复：pi ask 扩展并发时读到写了一半的文件（改为临时文件 + rename 原子发布）；macOS 编译修复（libc::mode_t 宽度随平台而变）",
      "构建与 CI：Rust 1.99 新 lint 对齐、真机 e2e 在 CI 显式跳过并打印、故障注入改为按注入根分槽、WS 集成用例时序假设修正、证据检索用例的真实 codegraph CLI 依赖（CI 安装 + 缺工具兜底跳过）",
    ],
  },
  {
    version: "0.0.9",
    date: "2026-08-26",
    title: "图片创作提速与修复",
    highlights: [
      "图片创作生成图片改为本地文件存储，历史会话打开不再受 base64 拖累",
      "修复 kimi code 提示词迭代失败问题",
    ],
  },
  {
    version: "0.0.8",
    date: "2026-08-17",
    title: "v0.0.8 更新",
    highlights: [
      "Story/Design 对话式修订：确认页直接输入修改意见，作者增量修订并附改动摘要，循环至满意；「重新编写」推倒重来已移除，改用反馈修订表达重写意图",
      "确认双出口与可选 Review：「确认并送审」进入交叉审核，「确认定稿」直接完成；reviewer 通过不再自动定稿，报告进入对话流可继续反馈；未启用 review 的会话可凭创建快照临时送审",
      "Canvas 产物审核面板：确认阶段右侧自动滑出产物全文与「本轮改动」摘要（断线重连恢复时同样自动滑出），操作条吸顶，左侧执行节点栏保留",
      "采纳 Review 意见：一键带入完整审核报告（含 findings 明细与可选建议）到反馈框，可编辑删减后发送；产出新版本后按钮自动隐藏",
      "对照编辑：≥1440px 聚焦输入框不收起面板，可对照 artifact 撰写反馈；<1440px 自动收起避免遮挡",
      "断线恢复与存量兼容：修订中断重连后保留产物并可重试本轮修订；升级前停在旧确认阶段的会话自动迁移到新确认页",
      "视觉升级：全站蓝紫主色 + 橙强调 + 奶油底（其他工作台色系随之联动）；spec 工作台 Claymorphism 形态，新增全站设计规范 MASTER.md",
    ],
  },
];

export function recentEntries(currentVersion: string, limit = 4): ChangelogEntry[] {
  const currentVersionIndex = CHANGELOG.findIndex((item) => item.version === currentVersion);
  if (currentVersionIndex === -1) {
    return [];
  }

  return CHANGELOG.slice(currentVersionIndex, currentVersionIndex + limit);
}

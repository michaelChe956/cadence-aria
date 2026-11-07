export interface ChangelogEntry {
  version: string;
  date: string;
  title: string;
  highlights: string[];
}

export const CURRENT_VERSION = "0.1.2";

// 发布升版本时请裁剪数组，仅保留最新 4 条。
export const CHANGELOG: ChangelogEntry[] = [
  {
    version: "0.1.2",
    date: "2026-11-08",
    title: "门卡不再让你误以为还要操作 · 编辑器不再依赖外网 · 删除工作项组不再被卡住",
    highlights: [
      "reviewer 分诊一旦收口（已终止 / 已留档 / 已确认），卡片不再以「等你操作」的形态出现：标题改为陈述式、可点选项被移除，并明确告诉你「这一轮已终止，之前的选择未生效，无需再操作」",
      "刷新或断线重连后，正在按反馈修订的门卡会显示「正在按反馈修订…」并锁住反馈框，不会再冒出一张要你重选的卡",
      "因取消而中断的修订现在记为「已中止」而不是「失败」，对话流会明说修订被中断、你的选择未生效",
      "产物审核页的代码编辑器改为本地加载，不再依赖外网 CDN：无外网或内网环境也能正常打开（此前整块空白）",
      "产物区加载时显示「正在加载内容…」，不再是一块没有提示的空白",
      "同一节点的详情请求不再反复发出（由重复 12 次降为 1 次），按钮点击延迟的已知成因之一被切断",
      "初始化引导新增第一步「管理 Provider（先核验机器级能力）」：provider 能力核验不到时生成弹窗会默认回落到 codex、失败面被误导，现在先管好机器级能力再往下走",
      "删除工作项组不再被「实体存在进行中的运行」阻塞：删除时会先取消该实体绑定的在途 run 并等它真实收口，不再报 409 让你重试",
    ],
  },
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
];

export function recentEntries(currentVersion: string, limit = 4): ChangelogEntry[] {
  const currentVersionIndex = CHANGELOG.findIndex((item) => item.version === currentVersion);
  if (currentVersionIndex === -1) {
    return [];
  }

  return CHANGELOG.slice(currentVersionIndex, currentVersionIndex + limit);
}

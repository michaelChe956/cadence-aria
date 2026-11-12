export interface ChangelogEntry {
  version: string;
  date: string;
  title: string;
  highlights: string[];
}

export const CURRENT_VERSION = "0.1.4";

// 发布升版本时请裁剪数组，仅保留最新 4 条。
export const CHANGELOG: ChangelogEntry[] = [
  {
    version: "0.1.4",
    date: "2026-11-12",
    title: "聚合初始化零变更失败带出 provider 真实输出 · CI 慢机竞态修复",
    highlights: [
      "聚合初始化的「零变更失败」现在带出 provider 的真实输出，不再只给一句失败",
      "聚合初始化的幂等重跑被判成功：同一请求重跑不再被当成新的失败",
      "两处 CI 慢机竞态修复：pid 文件空读、worktree 注册消失",
      "仓库初始化夹具轮询改用真实时间预算（CI 慢机竞态）",
      "「聚合初始化零变更诊断」的 OpenSpec change 已归档，主规格同步",
    ],
  },
  {
    version: "0.1.3",
    date: "2026-11-11",
    title: "LC 仓管理知识库模块全量交付 · macOS 构建修复 · CI 红灯链条补记",
    highlights: [
      "LC 仓管理知识库模块全量交付（含 WIG 优化的同期改动）",
      "「不替换改名」按平台拆分，修 macOS 构建失败",
      "CI 两个红 job 修复：fmt 与 release poll 帧门禁，并补两条文档证据",
      "CI 红灯的完整链条补记：4 层独立问题逐层定位，另记明 release.yml 不 needs Rust CI 的依赖缺口",
      "记录 0.1.2 tag 与 CI 的关系及不重打 tag 的理由",
      "根目录散落的 F58 教学注入报告归位到 cadence/reports/",
    ],
  },
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
];

export function recentEntries(currentVersion: string, limit = 4): ChangelogEntry[] {
  const currentVersionIndex = CHANGELOG.findIndex((item) => item.version === currentVersion);
  if (currentVersionIndex === -1) {
    return [];
  }

  return CHANGELOG.slice(currentVersionIndex, currentVersionIndex + limit);
}

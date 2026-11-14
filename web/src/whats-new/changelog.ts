export interface ChangelogEntry {
  version: string;
  date: string;
  title: string;
  highlights: string[];
}

export const CURRENT_VERSION = "0.1.6";

// 发布升版本时请裁剪数组，仅保留最新 4 条。
export const CHANGELOG: ChangelogEntry[] = [
  {
    version: "0.1.6",
    date: "2026-11-14",
    title: "点「安装」真的装得上 · 更新装回你正在用的那份 · 超大仓库的聚合索引不再误报 · 页面不再一直轮询",
    highlights: [
      "本机没装 provider 或 codegraph 时，点【安装】现在真的装得上并能用起来：安装目录会被纳入平台的命令可见范围（此前装到了平台看不见的位置，界面一直显示「未安装」，重复点击也没效果）",
      "Kimi Code 走官方脚本安装时也能被识别并探测到版本（此前脚本只改 shell 配置文件，正在运行的服务看不到它，而且不给出任何提示）",
      "新增「暴露到终端 PATH」：点一下就在你终端 PATH 上的目录里建一个软链，之后在终端里直接敲命令名也能用（只在你点它时才动作，不改你的 shell 配置文件）",
      "更新会装回你实际在用的那一份：当机器上同时存在多份同一个 CLI 时，更新此前可能写进你没在跑的那份，表现为「更新了却没变化」；现在更新后版本没变会明确报出来",
      "大型聚合根的索引验收不再把「输出超限」误报成「JSON 解析失败」：改为逐个成员目录判定，索引很大也能通过；确实超限时给出专用提示并保留上一份可用索引",
      "页面不再一直调用同一批接口：没有进行中任务时轮询间隔自动退到 60 秒、页面不可见时暂停、重新可见时立刻补一次；心跳不再触发整轮刷新，也不再对普通单仓发起必然失败的请求",
      "新增只读诊断接口 GET /api/providers/diagnostics：一次请求即可看到每个对象装到哪个目录、是否在你的终端 PATH 上、codegraph 平台包是否匹配、索引状态与最近失败原因",
    ],
  },
  {
    version: "0.1.5",
    date: "2026-11-12",
    title: "图片生成支持 GPT / MiniMax 多模型清单 · 连续修改自动带上上一张图 · 设置里可测供应方连通性 · 会话历史按时间顺序显示 · 模型没给出提示词时提示更醒目",
    highlights: [
      "图片生成的模型清单可以自己配：内置 GPT 家 8 个 + MiniMax 家 1 个推荐模型，每家一个复选下拉框；可取消勾选，也可添加网关没列出的自定义 id；点「测试连通」成功后清单会更新为网关返回的模型",
      "连续修改自动携带上一张生成图：默认开启「基于上一张图继续修改」，同一会话第二次及以后的生成自动把上一张生成图带上（本次上传的参考图优先）",
      "设置里可测供应方连通性：直接用弹窗里当前填写的值检查地址通不通、API Key 是否有效、清单里的模型网关是否都列出；不生成图片、不计费，测试失败也不影响保存",
      "会话历史按时间顺序显示：消息、建议 prompt、生成图与报错按各自发生的时间穿插成一条时间线，报错出现在它发生的位置",
      "模型没给出提示词时不再像「卡住了」：这一轮没给出提示词就一定有一句提示，并排在模型回复的前面；模型明确不给时，它的回复收进「查看模型说明」（默认收起）",
      "图片创作设置更安全：API Key 输入框不再回显可编辑的脱敏串（避免把真 Key 覆盖成 sk-****1234X）；「测试连通」只在 Base URL 与已保存的一致时才使用已保存的 Key",
      "重开设置弹窗不再重放上一次的探测结果：不会把你精简过的模型清单还原成网关全量清单",
    ],
  },
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
];

export function recentEntries(currentVersion: string, limit = 4): ChangelogEntry[] {
  const currentVersionIndex = CHANGELOG.findIndex((item) => item.version === currentVersion);
  if (currentVersionIndex === -1) {
    return [];
  }

  return CHANGELOG.slice(currentVersionIndex, currentVersionIndex + limit);
}

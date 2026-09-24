// F-50 裁决 5/6/7：协议错误呈现文案的单一事实源。面向用户主显中文（lead +
// 可选 body），错误码/英文原文只作为诊断信息由各错误面以 mono 副行与折叠详情
// 承载；未知码 fail-closed 落中性 lead，不猜语义、不藏原文（原文由调用方
// 主显或折叠）。页级/抽屉/投影三处共用，避免同一错误三套说法。
/**
 * 裸 driver 抢走租约后，本连接写操作被拒的协议码（F-11 恢复入口依据）。
 * 常量源头在本模块：投影（workspace-cockpit-projection）再导出既有引用。
 */
export const STALE_DRIVER_LEASE_CODE = "STALE_DRIVER_LEASE";

export interface ProtocolErrorDisplayCopy {
  /** 中文主显标题（页面级错误条/收件箱错误条目共用）。 */
  lead: string;
  /** 中文正文；null 表示该码没有可负责任的中文正文（原文由调用方呈现）。 */
  body: string | null;
}

export function protocolErrorCopy(code: string): ProtocolErrorDisplayCopy {
  if (code === STALE_DRIVER_LEASE_CODE) {
    return { lead: "连接租约已失效", body: "本连接已失去写入租约，当前操作未提交。" };
  }
  if (code === "OBSERVER_WRITE_REJECTED") {
    return { lead: "本连接为观察者连接，写操作已被拒收", body: null };
  }
  // C3/REQ-HTR-02：门态操作错误面指路（门态→门卡入口）；lease 类错误指路
  // （连接/租约→重新接管）由上方两码 + onRetakeLease 按钮既有承载。
  if (code === "abort_no_active_run") {
    return {
      lead: "当前没有进行中的运行可中止",
      body: "若人工门已开启，请在门卡上提交反馈、确认或终止此门。",
    };
  }
  if (code === "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID") {
    return {
      lead: "当前阶段不接受该操作",
      body: "会话停在人工门阶段时，请使用门卡上的操作：提交反馈 / 确认 / 终止此门。",
    };
  }
  return { lead: "操作被拒绝", body: null };
}

/** 错误详情折叠区的可达名称（错误码/原文/不可重试说明的统一入口）。 */
export const PROTOCOL_ERROR_DETAILS_LABEL = "查看诊断详情";

/** 裁决 5：无安全重放命令的错误在详情区说明原因（不渲染灰置重试）。 */
export const PROTOCOL_ERROR_NO_RETRY_NOTE = "该错误不支持安全重试";

/** 裁决 7：抽屉打开承载完整错误动作时，页级缩为引用面的提示语。 */
export const HARD_ERROR_DRAWER_REFERENCE_NOTE = "处理入口在待处理抽屉";

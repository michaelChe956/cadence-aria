export type LogicalCodebaseMemberStatus = "active" | "removed" | "tombstoned";

export type LogicalCodebaseMemberDto = {
  logical_repository_id: string;
  /// R8：primary 成员选择所需；旧数据/未投影时为 null。
  physical_repository_id: string | null;
  alias: string;
  status: LogicalCodebaseMemberStatus;
  /// REQ-MRE-01：服务端下发的结构化「无源码、未纳入代码搜索」标注
  /// （与卡片 `source_less_alert`、证据 `not_searched` 同构）。
  ///
  /// 缺省 / null = 无该事实（旧响应兼容）；**仅作提示**，不承载任何选择资格：
  /// 复选、提交、禁用、主仓校验一律不受它影响。
  index_alert?: { code: string; message_zh: string } | null;
};

export type LogicalCodebaseMembersResponse = {
  members: LogicalCodebaseMemberDto[];
};

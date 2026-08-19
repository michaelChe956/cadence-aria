export type LogicalCodebaseMemberStatus = "active" | "removed" | "tombstoned";

export type LogicalCodebaseMemberDto = {
  logical_repository_id: string;
  /// R8：primary 成员选择所需；旧数据/未投影时为 null。
  physical_repository_id: string | null;
  alias: string;
  status: LogicalCodebaseMemberStatus;
};

export type LogicalCodebaseMembersResponse = {
  members: LogicalCodebaseMemberDto[];
};

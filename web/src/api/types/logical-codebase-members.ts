export type LogicalCodebaseMemberStatus = "active" | "removed" | "tombstoned";

export type LogicalCodebaseMemberDto = {
  logical_repository_id: string;
  alias: string;
  status: LogicalCodebaseMemberStatus;
};

export type LogicalCodebaseMembersResponse = {
  members: LogicalCodebaseMemberDto[];
};

// 逻辑代码库登记 API 的后端 snake_case DTO 投影。
export type RegistrationPreflightRequest = {
  aggregate_root: string;
  candidate_paths: string[];
};

export type RegistrationPreflightItemDto = {
  path: string;
  class: string;
  reason: string | null;
};

export type RegistrationPreflightResponse = {
  preflight_id: string;
  created_at: string;
  items: RegistrationPreflightItemDto[];
};

export type RegistrationSubmitRequest = {
  aggregate_root: string;
  preflight_id: string;
  confirmed_paths: string[];
};

export type RegistrationBatchItemDto = {
  path: string;
  status: string;
  failure_reason: string | null;
};

export type RegistrationBatchDto = {
  batch_id: string;
  status: string;
  items: RegistrationBatchItemDto[];
};

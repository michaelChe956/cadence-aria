import { RotateCcw } from "lucide-react";
import { IMAGE_VENDOR_LABELS, type ImageVendor } from "../../api/types/image-create";
import {
  IMAGE_MODELS_LIMIT,
  type ImageVendorModelPool,
} from "../../state/image-create-model-pool";
import { useImageCreateStore } from "../../state/image-create-store";
import { ModelMultiSelect } from "./ModelMultiSelect";

/** 刷新候选池后需要提示用户的注意事项。 */
export type VendorModelNotice = {
  /** 已被另一家占用而跳过的模型 id。 */
  skipped: string[];
  /** 超过单家上限而截断：网关模型总数与实际保留数。 */
  truncated: { total: number; kept: number } | null;
};

export interface ModelListFieldProps {
  /** 供应方 id。 */
  vendor: ImageVendor;
  /** 供应方名称，用于无障碍标签。 */
  label: string;
  /** 已勾选的模型清单（= 要保存的清单）。 */
  models: string[];
  /** 该家的模型池：网关列表与候选池。 */
  pool: ImageVendorModelPool;
  /** 刷新候选池后的提示信息。 */
  notice: VendorModelNotice | null;
  /** 另一家供应方（用于「已被 X 使用」的提示）。 */
  otherVendor: ImageVendor;
  /** 内置推荐清单（用于「恢复推荐清单」）。 */
  recommendedModels: string[];
  disabled?: boolean;
  onChange: (models: string[]) => void;
}

/**
 * 模型清单字段：复选下拉框 + 恢复推荐清单 + 刷新候选池后的提示。
 */
export function ModelListField({
  vendor,
  label,
  models,
  pool,
  notice,
  otherVendor,
  recommendedModels,
  disabled = false,
  onChange,
}: ModelListFieldProps) {
  const mergeVendorModels = useImageCreateStore(
    (state) => state.mergeVendorModels,
  );
  // 网关未列出的候选：网关列表已知时，池里不在网关列表中的项。
  const notListed =
    pool.gateway.length > 0
      ? pool.candidates.filter((model) => !pool.gateway.includes(model))
      : [];

  return (
    <div className="mt-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-sm font-semibold text-[var(--aria-ink)]">
          模型清单
        </span>
        <button
          type="button"
          disabled={disabled}
          onClick={() =>
            onChange(mergeVendorModels(vendor, models, recommendedModels))
          }
          className="inline-flex cursor-pointer items-center gap-1 rounded-full border border-transparent bg-[var(--aria-primary-soft)] px-3 py-1 text-xs font-semibold text-[var(--aria-primary)] transition-colors duration-200 hover:border-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-60"
        >
          <RotateCcw aria-hidden="true" className="h-3.5 w-3.5" />
          恢复推荐清单
        </button>
      </div>
      <div role="group" aria-label={`${label} 模型清单`} className="mt-1.5">
        <ModelMultiSelect
          candidates={pool.candidates}
          selected={models}
          notListed={notListed}
          disabled={disabled}
          onChange={(next) =>
            // 自定义添加的 id 不在候选池里，并入候选池后再保存。
            onChange(mergeVendorModels(vendor, next, []))
          }
        />
      </div>
      <p className="mt-1.5 text-xs text-[var(--aria-ink-muted)]">
        默认从「测试连通」拉取的网关模型列表中选择，可取消勾选或添加自定义模型 id。
      </p>
      {notice?.truncated ? (
        <p className="mt-1.5 text-xs font-semibold text-[var(--aria-danger)]">
          {`网关返回 ${notice.truncated.total} 个模型，超过单家上限 ${IMAGE_MODELS_LIMIT}，已只保留前 ${notice.truncated.kept} 个，请自行精简`}
        </p>
      ) : null}
      {notice && notice.skipped.length > 0 ? (
        <p className="mt-1.5 text-xs font-semibold text-[var(--aria-danger)]">
          {`以下模型已被 ${IMAGE_VENDOR_LABELS[otherVendor]} 使用，已跳过：${notice.skipped.join("、")}`}
        </p>
      ) : null}
    </div>
  );
}

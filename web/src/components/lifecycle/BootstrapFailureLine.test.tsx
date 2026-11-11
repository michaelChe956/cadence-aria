// 测试点 #10：失败面渲染的唯一实现只消费结构化字段。
//
// - `triage="environment"` → 渲染中文分诊标签；
// - `title_zh` 在主行、`detail` 在折叠区、机器码在次要行（不是主标题）；
// - 布局失败必须同屏呈现「问题 / 原因 / 怎么做」三段；
// - `triage` 缺省 → 不渲染标签且不崩。
//
// 分诊按钮化（tasks 5.6）：环境类失败挂「重新检测」按钮 → 复用既有的
// `POST /api/providers/recheck`（`api/client.ts` 的 `recheckProviders()`），
// 有进行中态（防重复点击）与成功/失败反馈；反馈里的安装指引逐字复用后端
// `providers.rs` 的 `install_hint`，前端不自造安装文案。
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BootstrapFailureLine } from "./BootstrapFailureLine";
import { recheckProviders } from "../../api/client";
import type {
  BootstrapFailureDto,
  ProviderHealthEntry,
  ProviderHealthResponse,
} from "../../api/types";

vi.mock("../../api/client", () => ({
  recheckProviders: vi.fn(),
}));

const recheckProvidersMock = vi.mocked(recheckProviders);

function layoutFailure(
  overrides: Partial<BootstrapFailureDto> = {},
): BootstrapFailureDto {
  return {
    reason_code: "aggregate_index_layout_unsupported",
    detail: "问题：当前目录布局不被聚合索引支持\n机器码：aggregate_index_layout_unsupported",
    retryable: true,
    external_side_effect: "aggregate_index_cli",
    triage: "environment",
    copy: {
      title_zh: "当前目录布局不被聚合索引支持",
      reason_zh:
        "布局规则「每个成员必须是逻辑代码库根的一层直接子目录（<LC根>/<成员>/）」未满足。",
      action_zh: "让 LC 根与成员保持 <LC根>/<成员>/ 一层直挂，然后点「重试」。",
      triage: "environment",
      machine_line: "aggregate_index_layout_unsupported",
    },
    ...overrides,
  };
}

function providerEntry(
  overrides: Partial<ProviderHealthEntry> = {},
): ProviderHealthEntry {
  return {
    provider: "codex",
    display_name: "Codex",
    available: false,
    version: null,
    reason_code: "command_missing",
    reason: "codex command not found",
    checked_at: "2026-11-03T00:00:00Z",
    install_hint: "Install Codex CLI via npm (`npm i -g @openai/codex`).",
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function providerSnapshot(providers: ProviderHealthEntry[]): ProviderHealthResponse {
  return {
    schema_version: 1,
    generation: 2,
    checked_at: "2026-11-03T00:00:00Z",
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: false,
    test_provider_enabled: false,
    providers,
  };
}

describe("BootstrapFailureLine", () => {
  beforeEach(() => {
    recheckProvidersMock.mockReset();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });


  // 块2 11.10：codegraph 缺失面挂【安装】入口——复用 provider-cli-installation
  // 的同一套机制（ProviderInstallActionButton + GET /api/providers/install-targets）。
  // 【安装】（装 codegraph）与【重新检测】（复核可用性）是两个独立按钮。
  it("gives the codegraph-missing failure an install entry", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              targets: [
                {
                  target: "codegraph",
                  display_name: "codegraph",
                  cli_available: false,
                  local_version: null,
                  latest_version: "1.7.0",
                  latest_checked_at: "2026-11-04T00:00:00Z",
                  install_source: "npm @colbymchenry/codegraph",
                  action: "install",
                },
              ],
            }),
            { status: 200, headers: { "content-type": "application/json" } },
          ),
      ),
    );

    render(
      <BootstrapFailureLine
        failure={layoutFailure({
          reason_code: "codegraph_missing",
          copy: {
            title_zh: "未检测到 codegraph",
            reason_zh: "聚合索引依赖 codegraph CLI，但当前 PATH 上找不到它。",
            action_zh:
              "执行 npm i -g @colbymchenry/codegraph 安装（或把已有安装加入 PATH），装好后点「重试」。",
            triage: "environment",
            machine_line: "codegraph_missing",
          },
        })}
      />,
    );

    const install = await screen.findByTestId("provider-install-start-codegraph");
    expect(install).toHaveTextContent("安装");
    // 手动兜底命令逐字来自既有安装机制（npm 包名）。
    expect(
      screen.getByTestId("provider-install-manual-command-codegraph").textContent,
    ).toContain("@colbymchenry/codegraph");
    // 安装来源用后端安装目标面的原文，前端不自造安装文案。
    expect(
      screen.getByTestId("lc-codegraph-install-source").textContent,
    ).toContain("npm @colbymchenry/codegraph");
    // 「重新检测」是另一个独立按钮（语义不重叠）：装完后复核可用性。
    expect(screen.getByTestId("lc-bootstrap-failure-recheck")).toHaveTextContent(
      "重新检测",
    );
  });

  it("keeps the codegraph install entry off unrelated failures", () => {
    render(<BootstrapFailureLine failure={layoutFailure()} />);

    expect(screen.queryByTestId("lc-bootstrap-failure-codegraph-install")).toBeNull();
    expect(screen.queryByTestId("provider-install-start-codegraph")).toBeNull();
  });

  it("renders the three chinese segments and the machine code as a secondary line", () => {
    render(<BootstrapFailureLine failure={layoutFailure()} />);

    expect(screen.getByTestId("lc-bootstrap-failure-title").textContent).toBe(
      "当前目录布局不被聚合索引支持",
    );
    expect(
      screen.getByTestId("lc-bootstrap-failure-reason").textContent,
    ).toContain("布局规则");
    expect(
      screen.getByTestId("lc-bootstrap-failure-action").textContent,
    ).toContain("<LC根>/<成员>/");
    // 机器码只在次要行出现，绝不作为主标题。
    expect(
      screen.getByTestId("lc-bootstrap-failure-machine-line").textContent,
    ).toContain("aggregate_index_layout_unsupported");
    expect(screen.getByTestId("lc-bootstrap-failure-title").textContent).not.toContain(
      "aggregate_index_layout_unsupported",
    );
  });

  it("renders the triage label in chinese", () => {
    render(<BootstrapFailureLine failure={layoutFailure()} />);
    expect(
      screen.getByTestId("lc-bootstrap-failure-triage").textContent,
    ).toContain("环境问题");
  });

  it("keeps the raw detail inside a collapsed disclosure", () => {
    render(<BootstrapFailureLine failure={layoutFailure()} />);
    const details = screen.getByTestId("lc-bootstrap-failure-detail");
    expect(details.tagName).toBe("DETAILS");
    // 折叠区默认收起：没有 open 属性。
    expect(details.hasAttribute("open")).toBe(false);
    expect(details.textContent).toContain("问题：当前目录布局不被聚合索引支持");
  });

  it("does not crash without triage and renders no triage label", () => {
    render(
      <BootstrapFailureLine
        failure={layoutFailure({ triage: undefined, copy: null })}
      />,
    );
    expect(screen.queryByTestId("lc-bootstrap-failure-triage")).toBeNull();
    // 旧响应（无 copy）只降级展示机器码与诊断，不崩。
    expect(screen.getByTestId("lc-bootstrap-failure-title").textContent).toBe(
      "aggregate_index_layout_unsupported",
    );
    expect(screen.queryByTestId("lc-bootstrap-failure-reason")).toBeNull();
  });

  it("offers the recheck button for environment-class failures", () => {
    render(<BootstrapFailureLine failure={layoutFailure()} />);
    const button = screen.getByTestId("lc-bootstrap-failure-recheck");
    expect(button.textContent).toBe("重新检测");
    expect(button).not.toBeDisabled();
  });

  it("does not offer the recheck button for the other triage classes", () => {
    const failure = layoutFailure();
    render(
      <BootstrapFailureLine
        failure={{
          ...failure,
          triage: "product_defect",
          copy: { ...failure.copy!, triage: "product_defect" },
        }}
      />,
    );
    expect(screen.queryByTestId("lc-bootstrap-failure-recheck")).toBeNull();
  });

  it("rechecks providers and reuses the backend install hint in the feedback", async () => {
    recheckProvidersMock.mockResolvedValue(providerSnapshot([providerEntry()]));
    render(<BootstrapFailureLine failure={layoutFailure()} />);

    fireEvent.click(screen.getByTestId("lc-bootstrap-failure-recheck"));

    await waitFor(() => {
      expect(
        screen.getByTestId("lc-bootstrap-failure-recheck-feedback").textContent,
      ).toContain("重新检测完成");
    });
    expect(recheckProvidersMock).toHaveBeenCalledTimes(1);
    // 安装指引逐字复用后端 install_hint，不在前端另写一套。
    expect(
      screen.getByTestId("lc-bootstrap-failure-recheck-hint-Codex").textContent,
    ).toBe("Codex：Install Codex CLI via npm (`npm i -g @openai/codex`).");
  });

  it("states_that_one_available_provider_is_enough", async () => {
    // 真机反馈：用户看到「仍有 N 个 provider 不可用…按安装指引处理后点重试」，
    // 误以为四家都必须装。口径改成「有一家可用即可」，其余只是可选切换项。
    recheckProvidersMock.mockResolvedValue(
      providerSnapshot([
        providerEntry({
          provider: "pi",
          display_name: "Pi",
          available: true,
          version: "1.5.0",
          reason_code: null,
          reason: null,
          install_hint: "",
        }),
        providerEntry(),
      ]),
    );
    render(<BootstrapFailureLine failure={layoutFailure()} />);

    fireEvent.click(screen.getByTestId("lc-bootstrap-failure-recheck"));

    const feedback = await screen.findByTestId(
      "lc-bootstrap-failure-recheck-feedback",
    );
    expect(feedback.textContent).toContain("已具备执行条件");
    expect(feedback.textContent).toContain("不影响本次执行");
    expect(feedback.textContent).not.toContain("按下面的安装指引处理后点");
  });

  it("says_at_least_one_provider_is_required_when_none_is_available", async () => {
    recheckProvidersMock.mockResolvedValue(
      providerSnapshot([providerEntry()]),
    );
    render(<BootstrapFailureLine failure={layoutFailure()} />);

    fireEvent.click(screen.getByTestId("lc-bootstrap-failure-recheck"));

    const feedback = await screen.findByTestId(
      "lc-bootstrap-failure-recheck-feedback",
    );
    expect(feedback.textContent).toContain("至少要有一家可用");
  });

  it("labels_the_diagnostic_detail_as_the_real_failure_reason", () => {
    // 真实失败原因就在诊断段里：折叠标题点明它，用户知道该点开看什么。
    render(<BootstrapFailureLine failure={layoutFailure()} />);

    const detail = screen.getByTestId("lc-bootstrap-failure-detail");
    expect(detail.textContent).toContain("真实失败原因");
  });

  it("keeps the button busy while the recheck is in flight and does not resend it", async () => {
    const pending = deferred<ProviderHealthResponse>();
    recheckProvidersMock.mockReturnValue(pending.promise);
    render(<BootstrapFailureLine failure={layoutFailure()} />);
    const button = screen.getByTestId("lc-bootstrap-failure-recheck");

    fireEvent.click(button);

    await waitFor(() => expect(button).toBeDisabled());
    expect(button.textContent).toBe("正在重新检测");
    fireEvent.click(button);
    expect(recheckProvidersMock).toHaveBeenCalledTimes(1);

    await act(async () => {
      pending.resolve(providerSnapshot([]));
    });
    await waitFor(() => expect(button).not.toBeDisabled());
    expect(
      screen.getByTestId("lc-bootstrap-failure-recheck-feedback").textContent,
    ).toContain("重新检测完成");
  });

  it("reports a failed recheck", async () => {
    recheckProvidersMock.mockRejectedValue(new Error("network down"));
    render(<BootstrapFailureLine failure={layoutFailure()} />);

    fireEvent.click(screen.getByTestId("lc-bootstrap-failure-recheck"));

    await waitFor(() => {
      expect(
        screen.getByTestId("lc-bootstrap-failure-recheck-feedback").textContent,
      ).toContain("重新检测失败");
    });
    expect(
      screen.getByTestId("lc-bootstrap-failure-recheck-feedback").textContent,
    ).toContain("network down");
  });
});

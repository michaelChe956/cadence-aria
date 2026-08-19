import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ArtifactLine, RoleInstance } from "../../api/groupChat";
import { ApiRequestError } from "../../api/client";
import { finalizeGroupChat } from "../../api/groupChat";
import { ArtifactLinePanel } from "./ArtifactLinePanel";

vi.mock("../../api/groupChat", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api/groupChat")>();
  return { ...actual, finalizeGroupChat: vi.fn() };
});

const roles: RoleInstance[] = [
  {
    id: "role-author",
    role_key: "author",
    provider: "claude_code",
    display_name: "需求作者",
    permission_mode: "auto",
    seen_cursor: 0,
    injection_watermark: 0,
  },
  {
    id: "role-frontend",
    role_key: "frontend_design",
    provider: "codex",
    display_name: "前端设计师",
    permission_mode: "supervised",
    seen_cursor: 0,
    injection_watermark: 0,
  },
];

function line(
  kind: ArtifactLine["kind"],
  slots: Array<{
    key: string;
    markdown?: string;
    version?: number;
    author?: string;
    claim?: string;
  }>,
  finalizedVersions: string[] = [],
): ArtifactLine {
  return {
    kind,
    drafts: slots.map((slot) => ({
      slot_key: slot.key,
      current: slot.markdown
        ? {
            version: slot.version ?? 1,
            markdown: slot.markdown,
            author_role_id: slot.author ?? "role-author",
            based_on_events: 3,
          }
        : null,
      claim: slot.claim
        ? { holder_role_id: slot.claim, claimed_at: "2026-08-18T00:00:00Z" }
        : null,
    })),
    finalized_versions: finalizedVersions,
    entity_id: null,
    bridge_session_id: null,
  };
}

function artifactLines(overrides: Partial<Record<ArtifactLine["kind"], ArtifactLine>> = {}) {
  return [
    overrides.issue_refinement ?? line("issue_refinement", [{ key: "issue_full" }]),
    overrides.story_spec ?? line("story_spec", [{ key: "story_full" }]),
    overrides.design_spec ??
      line("design_spec", [
        { key: "design_summary" },
        { key: "design_frontend" },
        { key: "design_backend" },
      ]),
  ];
}

function renderPanel(
  overrides: Partial<ComponentProps<typeof ArtifactLinePanel>> = {},
) {
  return render(
    <ArtifactLinePanel
      sessionId="session-1"
      artifactLines={artifactLines()}
      roles={roles}
      onDraftSlot={vi.fn()}
      {...overrides}
    />,
  );
}

describe("ArtifactLinePanel", () => {
  it("在窄屏限制高度并由面板自身滚动", () => {
    renderPanel();

    expect(screen.getByTestId("artifact-line-panel")).toHaveClass(
      "min-h-0",
      "max-h-[50vh]",
      "overflow-y-auto",
      "lg:max-h-none",
      "lg:w-80",
      "lg:shrink-0",
    );
  });

  beforeEach(() => {
    vi.mocked(finalizeGroupChat).mockReset();
  });

  it("为三条产物线展示未开始、起草中、待审、可定稿和已定稿状态", () => {
    const { rerender } = renderPanel({
      artifactLines: artifactLines({
        issue_refinement: line("issue_refinement", [{ key: "issue_full" }]),
        story_spec: line(
          "story_spec",
          [{ key: "story_full", markdown: "# 故事", claim: "role-author" }],
          ["story-v1", "story-v2"],
        ),
        design_spec: line("design_spec", [
          { key: "design_summary", markdown: "# 概要" },
          { key: "design_frontend", markdown: "# 前端", claim: "role-frontend" },
          { key: "design_backend" },
        ]),
      }),
    });

    expect(screen.getByTestId("artifact-line-issue_refinement")).toHaveTextContent("未开始");
    expect(screen.getByTestId("artifact-line-story_spec")).toHaveTextContent("已定稿 v2");
    expect(screen.getByTestId("artifact-line-design_spec")).toHaveTextContent(
      "起草中（前端设计师）",
    );
    expect(screen.getByText("由前端设计师认领")).toBeInTheDocument();

    rerender(
      <ArtifactLinePanel
        sessionId="session-1"
        artifactLines={artifactLines({
          story_spec: line("story_spec", [{ key: "story_full", markdown: "# 故事" }]),
          design_spec: line("design_spec", [
            { key: "design_summary", markdown: "# 概要" },
            { key: "design_frontend", markdown: "# 前端" },
            { key: "design_backend" },
          ]),
        })}
        roles={roles}
        onDraftSlot={vi.fn()}
      />,
    );

    expect(screen.getByTestId("artifact-line-design_spec")).toHaveTextContent("待审");
    expect(screen.getByTestId("artifact-line-story_spec")).toHaveTextContent("可定稿");
  });

  it("混合模式下 Story Spec 已由其他入口定稿时仍允许尝试 Design Spec 定稿", async () => {
    const user = userEvent.setup();
    vi.mocked(finalizeGroupChat).mockResolvedValueOnce({
      event: {
        type: "finalize_event",
        artifact_line: "design_spec",
        version: "design-v1",
        included_slots: ["design_summary", "design_frontend", "design_backend"],
      },
      session: {
        id: "session-1",
        project_id: "project-1",
        issue_id: "issue-1",
        status: "active",
        roles,
        artifact_lines: artifactLines(),
        created_at: "2026-08-18T00:00:00Z",
        updated_at: "2026-08-18T00:00:00Z",
      },
    });
    renderPanel({
      artifactLines: artifactLines({
        // 本会话没有 Story finalized 标记，模拟由其他入口确认的混合模式。
        story_spec: line("story_spec", [{ key: "story_full", markdown: "# 外部已确认故事" }]),
        design_spec: line("design_spec", [
          { key: "design_summary", markdown: "# 概要" },
          { key: "design_frontend", markdown: "# 前端" },
          { key: "design_backend", markdown: "# 后端" },
        ]),
      }),
    });

    const button = screen.getByRole("button", { name: "定稿设计规格" });
    expect(button).not.toBeDisabled();
    expect(button).toHaveAttribute(
      "title",
      "可尝试定稿；若 Story Spec 前置未满足，后端会返回 story_spec_not_confirmed",
    );
    await user.click(button);
    await waitFor(() => expect(finalizeGroupChat).toHaveBeenCalledWith("session-1", {
      line_kind: "design_spec",
    }));
  });

  it("确认跳过缺失的 Design Spec 槽后，携带已有槽提交定稿", async () => {
    const user = userEvent.setup();
    const updatedSession = {
      id: "session-1",
      project_id: "project-1",
      issue_id: "issue-1",
      status: "active" as const,
      roles,
      artifact_lines: artifactLines(),
      created_at: "2026-08-18T00:00:00Z",
      updated_at: "2026-08-18T00:00:00Z",
    };
    vi.mocked(finalizeGroupChat).mockResolvedValueOnce({
      event: {
        type: "finalize_event",
        artifact_line: "design_spec",
        version: "design-v1",
        included_slots: ["design_summary", "design_frontend"],
      },
      session: updatedSession,
    });
    const onSessionUpdated = vi.fn();
    renderPanel({
      artifactLines: artifactLines({
        story_spec: line("story_spec", [{ key: "story_full", markdown: "# 故事" }], ["story-v1"]),
        design_spec: line("design_spec", [
          { key: "design_summary", markdown: "# 概要" },
          { key: "design_frontend", markdown: "# 前端" },
          { key: "design_backend" },
        ]),
      }),
      onSessionUpdated,
    });

    await user.click(screen.getByRole("button", { name: "定稿设计规格" }));

    expect(screen.getByRole("dialog", { name: "确认跳过缺失草稿槽" })).toHaveTextContent(
      "后端设计",
    );
    expect(finalizeGroupChat).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "确认跳过并定稿" }));

    await waitFor(() => {
      expect(finalizeGroupChat).toHaveBeenCalledWith("session-1", {
        line_kind: "design_spec",
        included_slots_override: ["design_summary", "design_frontend"],
      });
    });
    expect(onSessionUpdated).toHaveBeenCalledWith(updatedSession);
  });

  it("将每个槽的起草操作上抛给页面，并可展开草稿预览", async () => {
    const user = userEvent.setup();
    const onDraftSlot = vi.fn();
    renderPanel({
      artifactLines: artifactLines({
        design_spec: line("design_spec", [
          { key: "design_summary" },
          { key: "design_frontend", markdown: "# 前端方案\n\n这里是草稿内容。" },
          { key: "design_backend" },
        ]),
      }),
      onDraftSlot,
    });

    await user.click(screen.getByRole("button", { name: "起草前端设计" }));
    expect(onDraftSlot).toHaveBeenCalledWith("design_frontend");
    expect(screen.queryByText("这里是草稿内容。")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "预览前端设计草稿" }));
    expect(screen.getByText("这里是草稿内容。")).toBeInTheDocument();
  });

  it("在后端仍拒绝 Design Spec 定稿时展示 Story Spec 前置错误", async () => {
    const user = userEvent.setup();
    vi.mocked(finalizeGroupChat).mockRejectedValueOnce(
      new ApiRequestError({
        code: "story_spec_not_confirmed",
        message: "story spec must be confirmed",
        details: {},
      }),
    );
    renderPanel({
      artifactLines: artifactLines({
        story_spec: line("story_spec", [{ key: "story_full", markdown: "# 故事" }], ["story-v1"]),
        design_spec: line("design_spec", [
          { key: "design_summary", markdown: "# 概要" },
          { key: "design_frontend", markdown: "# 前端" },
          { key: "design_backend", markdown: "# 后端" },
        ]),
      }),
    });

    await user.click(screen.getByRole("button", { name: "定稿设计规格" }));

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("需先定稿 Story Spec");
    });
  });
});

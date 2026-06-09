import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ArtifactVersion, ArtifactVersionSummary } from "../../state/workspace-ws-store";
import { ArtifactPane } from "./ArtifactPane";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

vi.mock("../shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: ({
    original,
    modified,
    height,
  }: {
    original: string;
    modified: string;
    height?: string;
  }) => (
    <div data-testid="monaco-diff-viewer" data-height={height}>
      <span data-testid="artifact-diff-original">{original}</span>
      <span data-testid="artifact-diff-modified">{modified}</span>
    </div>
  ),
}));

describe("ArtifactPane", () => {
  it("loads only the latest summary version on initial render and caches it", async () => {
    const loadArtifactVersion = vi.fn(() => Promise.resolve("# Artifact v3\n\n最新内容"));
    const onCacheArtifactContent = vi.fn();

    render(
      <ArtifactPane
        artifactVersions={artifactVersionSummaries()}
        artifact={null}
        sessionId="session-1"
        artifactContentCache={{}}
        loadArtifactVersion={loadArtifactVersion}
        onCacheArtifactContent={onCacheArtifactContent}
      />,
    );

    expect(screen.getByTestId("artifact-loading")).toHaveTextContent("正在加载 v3");
    expect(loadArtifactVersion).toHaveBeenCalledTimes(1);
    expect(loadArtifactVersion).toHaveBeenCalledWith(3);

    await waitFor(() => {
      expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Artifact v3");
    });
    expect(onCacheArtifactContent).toHaveBeenCalledWith(3, "# Artifact v3\n\n最新内容");
  });

  it("loads a selected summary version without preloading other versions", async () => {
    const loadArtifactVersion = vi.fn((version: number) =>
      Promise.resolve(`# Artifact v${version}\n\n内容 ${version}`),
    );

    render(
      <ArtifactPane
        artifactVersions={artifactVersionSummaries()}
        artifact={null}
        sessionId="session-1"
        artifactContentCache={{ 3: "# Artifact v3\n\n最新内容" }}
        loadArtifactVersion={loadArtifactVersion}
        onCacheArtifactContent={vi.fn()}
      />,
    );

    expect(loadArtifactVersion).not.toHaveBeenCalled();

    fireEvent.change(screen.getByLabelText("Artifact 版本"), { target: { value: "1" } });

    expect(screen.getByTestId("artifact-loading")).toHaveTextContent("正在加载 v1");
    await waitFor(() => {
      expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Artifact v1");
    });
    expect(loadArtifactVersion).toHaveBeenCalledTimes(1);
    expect(loadArtifactVersion).toHaveBeenCalledWith(1);
  });

  it("loads selected and previous versions for diff without reloading cached previous", async () => {
    const loadArtifactVersion = vi.fn((version: number) =>
      Promise.resolve(`# Artifact v${version}\n\n内容 ${version}`),
    );

    render(
      <ArtifactPane
        artifactVersions={artifactVersionSummaries()}
        artifact={null}
        sessionId="session-1"
        artifactContentCache={{ 2: "# Artifact v2\n\n缓存内容" }}
        loadArtifactVersion={loadArtifactVersion}
        onCacheArtifactContent={vi.fn()}
      />,
    );

    await waitFor(() => expect(loadArtifactVersion).toHaveBeenCalledWith(3));
    await waitFor(() => {
      expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Artifact v3");
    });

    fireEvent.click(screen.getByRole("button", { name: "显示 Diff" }));

    await waitFor(() => {
      expect(screen.getByTestId("monaco-diff-viewer")).toBeInTheDocument();
    });
    expect(screen.getByTestId("artifact-diff-original")).toHaveTextContent("缓存内容");
    expect(screen.getByTestId("artifact-diff-modified")).toHaveTextContent("# Artifact v3");
    expect(loadArtifactVersion).toHaveBeenCalledTimes(1);
  });

  it("loads both selected and previous versions when diff needs uncached summaries", async () => {
    const loadArtifactVersion = vi.fn((version: number) =>
      Promise.resolve(`# Artifact v${version}\n\n内容 ${version}`),
    );

    render(
      <ArtifactPane
        artifactVersions={artifactVersionSummaries()}
        artifact={null}
        sessionId="session-1"
        artifactContentCache={{}}
        loadArtifactVersion={loadArtifactVersion}
        onCacheArtifactContent={vi.fn()}
      />,
    );

    await waitFor(() => {
      expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Artifact v3");
    });

    fireEvent.click(screen.getByRole("button", { name: "显示 Diff" }));

    await waitFor(() => {
      expect(screen.getByTestId("monaco-diff-viewer")).toBeInTheDocument();
    });
    expect(loadArtifactVersion).toHaveBeenCalledTimes(2);
    expect(loadArtifactVersion).toHaveBeenNthCalledWith(1, 3);
    expect(loadArtifactVersion).toHaveBeenNthCalledWith(2, 2);
  });

  it("shows a version-scoped error when loading a summary version fails", async () => {
    const loadArtifactVersion = vi.fn(() => Promise.reject(new Error("network down")));

    render(
      <ArtifactPane
        artifactVersions={artifactVersionSummaries()}
        artifact={null}
        sessionId="session-1"
        artifactContentCache={{}}
        loadArtifactVersion={loadArtifactVersion}
        onCacheArtifactContent={vi.fn()}
      />,
    );

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("加载 v3 失败：network down");
    });
  });

  it("ignores stale pending loads after session changes and loads the new session", async () => {
    const firstLoad = syncDeferredPromise<string>();
    const secondLoad = deferred<string>();
    const loadArtifactVersion = vi
      .fn<(version: number) => Promise<string>>()
      .mockReturnValueOnce(firstLoad.promise)
      .mockReturnValueOnce(secondLoad.promise);
    const onCacheArtifactContent = vi.fn();

    const { rerender } = render(
      <>
        <ArtifactPane
          artifactVersions={[artifactVersionSummary(1)]}
          artifact={null}
          sessionId="session-1"
          artifactContentCache={{}}
          loadArtifactVersion={loadArtifactVersion}
          onCacheArtifactContent={onCacheArtifactContent}
        />
      </>,
    );

    await waitFor(() => expect(loadArtifactVersion).toHaveBeenCalledTimes(1));

    rerender(
      <>
        <ArtifactPane
          artifactVersions={[artifactVersionSummary(1)]}
          artifact={null}
          sessionId="session-2"
          artifactContentCache={{}}
          loadArtifactVersion={loadArtifactVersion}
          onCacheArtifactContent={onCacheArtifactContent}
        />
        <ResolveDuringRender resolve={() => firstLoad.resolve("# Stale Session Artifact")} />
      </>,
    );

    expect(onCacheArtifactContent).not.toHaveBeenCalledWith(1, "# Stale Session Artifact");
    expect(screen.queryByText("# Stale Session Artifact")).not.toBeInTheDocument();

    await waitFor(() => expect(loadArtifactVersion).toHaveBeenCalledTimes(2));

    secondLoad.resolve("# Current Session Artifact");
    await waitFor(() => {
      expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Current Session Artifact");
    });
    expect(onCacheArtifactContent).toHaveBeenCalledWith(1, "# Current Session Artifact");
  });

  it("does not render stale same-version content immediately after session changes", async () => {
    const currentLoad = deferred<string>();
    const loadArtifactVersion = vi.fn(() => currentLoad.promise);
    const { rerender } = render(
      <ArtifactPane
        artifactVersions={[artifactVersionSummary(1)]}
        artifact={null}
        sessionId="session-1"
        artifactContentCache={{ 1: "# Session 1 Artifact" }}
        loadArtifactVersion={loadArtifactVersion}
        onCacheArtifactContent={vi.fn()}
      />,
    );

    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Session 1 Artifact");

    rerender(
      <ArtifactPane
        artifactVersions={[artifactVersionSummary(1)]}
        artifact={null}
        sessionId="session-2"
        artifactContentCache={{ 1: "# Session 1 Artifact" }}
        loadArtifactVersion={loadArtifactVersion}
        onCacheArtifactContent={vi.fn()}
      />,
    );

    expect(screen.queryByText("# Session 1 Artifact")).not.toBeInTheDocument();
    expect(screen.getByTestId("artifact-loading")).toHaveTextContent("正在加载 v1");

    currentLoad.resolve("# Session 2 Artifact");
    await waitFor(() => {
      expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Session 2 Artifact");
    });
  });

  it("retries a failed summary version load", async () => {
    const loadArtifactVersion = vi
      .fn<(version: number) => Promise<string>>()
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValueOnce("# Artifact after retry");

    render(
      <ArtifactPane
        artifactVersions={[artifactVersionSummary(1)]}
        artifact={null}
        sessionId="session-1"
        artifactContentCache={{}}
        loadArtifactVersion={loadArtifactVersion}
        onCacheArtifactContent={vi.fn()}
      />,
    );

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("加载 v1 失败：network down");
    });

    fireEvent.click(screen.getByRole("button", { name: "重试" }));

    await waitFor(() => {
      expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Artifact after retry");
    });
    expect(loadArtifactVersion).toHaveBeenCalledTimes(2);
  });

  it("shows an unavailable state in diff mode when summary content has no loader", () => {
    render(
      <ArtifactPane
        artifactVersions={[artifactVersionSummary(1), artifactVersionSummary(2)]}
        artifact={null}
        artifactContentCache={{}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "显示 Diff" }));

    expect(screen.queryByTestId("artifact-loading")).not.toBeInTheDocument();
    expect(screen.getByTestId("artifact-unavailable")).toHaveTextContent("Artifact 内容未加载");
  });

  it("renders the latest artifact version and switches versions", () => {
    render(<ArtifactPane artifactVersions={artifactVersions()} artifact="# fallback" />);

    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Artifact v2");
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("新增内容");

    fireEvent.change(screen.getByLabelText("Artifact 版本"), { target: { value: "1" } });

    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("# Artifact v1");
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("旧内容");
  });

  it("shows a Monaco diff and can collapse", () => {
    render(<ArtifactPane artifactVersions={artifactVersions()} artifact={null} />);

    fireEvent.click(screen.getByRole("button", { name: "显示 Diff" }));
    expect(screen.getByTestId("artifact-diff")).toBeInTheDocument();
    expect(screen.getByTestId("monaco-diff-viewer")).toBeInTheDocument();
    expect(screen.getByTestId("artifact-diff-original")).toHaveTextContent("# Artifact v1");
    expect(screen.getByTestId("artifact-diff-modified")).toHaveTextContent("新增内容");

    fireEvent.click(screen.getByRole("button", { name: "折叠 Artifact" }));
    expect(screen.queryByLabelText("Artifact 版本")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "展开 Artifact" }));
    expect(screen.getByLabelText("Artifact 版本")).toBeInTheDocument();
  });
});

function artifactVersions(): ArtifactVersion[] {
  return [
    {
      version: 1,
      markdown: "# Artifact v1\n\n旧内容",
      generated_by: "claude_code",
      created_at: "2026-05-21T10:00:00Z",
      source_node_id: "node-1",
    },
    {
      version: 2,
      markdown: "# Artifact v2\n\n旧内容\n新增内容",
      generated_by: "claude_code",
      created_at: "2026-05-21T10:01:00Z",
      source_node_id: "node-2",
    },
  ];
}

function artifactVersionSummaries(): ArtifactVersionSummary[] {
  return [
    artifactVersionSummary(1),
    artifactVersionSummary(2),
    artifactVersionSummary(3),
  ];
}

function artifactVersionSummary(version: number): ArtifactVersionSummary {
  return {
    version,
    generated_by: "claude_code",
    created_at: `2026-05-21T10:0${version - 1}:00Z`,
    source_node_id: `node-${version}`,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

function syncDeferredPromise<T>() {
  let fulfilled = false;
  let rejected = false;
  let value: T;
  let reason: unknown;
  const fulfillCallbacks: Array<(value: T) => void> = [];
  const rejectCallbacks: Array<(reason: unknown) => void> = [];
  const promise = {
    then<TResult1 = T, TResult2 = never>(
      onFulfilled?: ((value: T) => TResult1 | PromiseLike<TResult1>) | null,
      onRejected?: ((reason: unknown) => TResult2 | PromiseLike<TResult2>) | null,
    ): Promise<TResult1 | TResult2> {
      return new Promise<TResult1 | TResult2>((resolve, reject) => {
        const handleFulfilled = (nextValue: T) => {
          try {
            resolve(onFulfilled ? onFulfilled(nextValue) : (nextValue as unknown as TResult1));
          } catch (error) {
            reject(error);
          }
        };
        const handleRejected = (nextReason: unknown) => {
          try {
            if (onRejected) {
              resolve(onRejected(nextReason));
            } else {
              reject(nextReason);
            }
          } catch (error) {
            reject(error);
          }
        };
        if (fulfilled) {
          handleFulfilled(value);
        } else if (rejected) {
          handleRejected(reason);
        } else {
          fulfillCallbacks.push(handleFulfilled);
          rejectCallbacks.push(handleRejected);
        }
      });
    },
  } as Promise<T>;
  return {
    promise,
    resolve(nextValue: T) {
      if (fulfilled || rejected) {
        return;
      }
      fulfilled = true;
      value = nextValue;
      for (const callback of fulfillCallbacks) {
        callback(nextValue);
      }
    },
    reject(nextReason: unknown) {
      if (fulfilled || rejected) {
        return;
      }
      rejected = true;
      reason = nextReason;
      for (const callback of rejectCallbacks) {
        callback(nextReason);
      }
    },
  };
}

function ResolveDuringRender({ resolve }: { resolve: () => void }) {
  resolve();
  return null;
}

import { FolderGit2, Plus } from "lucide-react";
import { useState } from "react";
import type { CreateWorkspaceRequest, Workspace } from "../../api/types";

export function WorkspaceManager({
  workspaces,
  busy,
  onCreateWorkspace,
}: {
  workspaces: Workspace[];
  busy: boolean;
  onCreateWorkspace: (payload: CreateWorkspaceRequest) => void | Promise<void>;
}) {
  const [name, setName] = useState("");
  const [path, setPath] = useState("");

  const canSubmit = name.trim() !== "" && path.trim() !== "" && !busy;

  return (
    <section
      role="region"
      aria-label="Workspace 空间"
      className="rounded-lg border-2 border-cyan-200 bg-white p-4 shadow-[0_10px_0_rgba(6,182,212,0.10),0_18px_38px_rgba(15,118,110,0.12)]"
    >
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-black text-[#241B2F]">Workspace 空间</h2>
          <p className="mt-1 text-sm font-semibold text-[#5E516B]">本地代码库</p>
        </div>
        <span className="inline-flex items-center gap-2 rounded-lg border-2 border-cyan-200 bg-cyan-50 px-3 py-1 text-xs font-black text-cyan-950">
          <FolderGit2 className="h-4 w-4" />
          {workspaces.length}
        </span>
      </div>

      <form
        className="grid gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          if (!canSubmit) {
            return;
          }
          void Promise.resolve(onCreateWorkspace({ name: name.trim(), path: path.trim() })).then(() => {
            setName("");
            setPath("");
          });
        }}
      >
        <label className="text-xs font-black text-cyan-900">
          workspace 名称
          <input
            aria-label="workspace 名称"
            className="mt-1 w-full rounded-lg border-2 border-cyan-100 bg-white px-3 py-2 text-sm font-semibold text-cyan-950 shadow-inner shadow-cyan-200/60 outline-none transition-colors placeholder:text-cyan-300 focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
        </label>
        <label className="text-xs font-black text-cyan-900">
          workspace 路径
          <input
            aria-label="workspace 路径"
            className="mt-1 w-full rounded-lg border-2 border-cyan-100 bg-white px-3 py-2 font-mono text-sm font-semibold text-cyan-950 shadow-inner shadow-cyan-200/60 outline-none transition-colors placeholder:text-cyan-300 focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
            value={path}
            onChange={(event) => setPath(event.target.value)}
          />
        </label>
        <button
          type="submit"
          disabled={!canSubmit}
          className="inline-flex items-center justify-center rounded-lg border-2 border-cyan-600 bg-cyan-500 px-4 py-2 text-sm font-black text-white shadow-[0_5px_0_rgba(14,116,144,0.38)] transition-colors hover:bg-cyan-400 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-cyan-200 disabled:border-slate-300 disabled:bg-slate-200 disabled:text-slate-500 disabled:shadow-none"
        >
          <Plus className="mr-1 h-4 w-4" />
          新建 workspace
        </button>
      </form>

      <div className="mt-4">
        <ul aria-label="workspace 列表" className="space-y-2">
          {workspaces.map((workspace) => (
            <li
              key={workspace.workspace_id}
              className="rounded-lg border-2 border-cyan-100 bg-cyan-50/70 px-3 py-2 text-sm shadow-[0_4px_0_rgba(6,182,212,0.10)]"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <strong className="text-cyan-950">{workspace.name}</strong>
                <span className="font-mono text-xs font-bold text-cyan-800">
                  {workspace.workspace_id}
                </span>
              </div>
              <div className="mt-1 truncate font-mono text-xs font-semibold text-[#5E516B]">
                {workspace.path}
              </div>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}

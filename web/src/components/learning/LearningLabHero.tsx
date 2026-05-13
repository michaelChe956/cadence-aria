type LearningLabHeroProps = {
  activeTaskId: string | null;
  nodeCount: number;
  artifactCount: number;
  eventCount: number;
  selectedNodeId: string | null;
};

export function LearningLabHero({
  activeTaskId,
  nodeCount,
  artifactCount,
  eventCount,
  selectedNodeId,
}: LearningLabHeroProps) {
  const statusCards = [
    { label: "Task", value: activeTaskId ?? "ready", tone: "indigo" },
    { label: "Nodes", value: String(nodeCount), tone: "cyan" },
    { label: "Evidence", value: String(artifactCount), tone: "emerald" },
    { label: "Events", value: String(eventCount), tone: "orange" },
  ] as const;

  return (
    <section
      role="group"
      aria-label="AI coding workbench status"
      className="relative overflow-hidden rounded-lg border-2 border-rose-200 bg-gradient-to-br from-white via-rose-50 to-orange-50 p-4 shadow-[0_12px_0_rgba(249,115,22,0.10),0_24px_50px_rgba(190,24,93,0.14)]"
    >
      <div className="pointer-events-none absolute -right-10 -top-10 h-40 w-40 rounded-full bg-orange-200/55 blur-3xl" />
      <div className="pointer-events-none absolute bottom-4 left-1/2 h-28 w-28 rounded-full bg-cyan-200/55 blur-3xl" />
      <div className="relative grid gap-4 xl:grid-cols-[minmax(0,1fr)_20rem] xl:items-center">
        <div className="min-w-0">
          <div className="inline-flex items-center rounded-lg border-2 border-orange-200 bg-orange-100 px-3 py-1 text-xs font-bold text-orange-900 shadow-[0_4px_0_rgba(249,115,22,0.18)]">
            AI Coding Workbench
          </div>
          <h2 className="mt-3 max-w-2xl text-3xl font-black leading-tight text-[#241B2F] md:text-4xl">
            Build, inspect, and guide each node.
          </h2>
          <p className="mt-2 max-w-2xl text-sm font-semibold leading-6 text-[#5E516B] md:text-base">
            {selectedNodeId
              ? `当前聚焦 ${selectedNodeId}，交互窗口保持在主舞台。`
              : "创建任务后，交互窗口会承载主要输入、输出和确认动作。"}
          </p>
          <div className="mt-4 grid grid-cols-2 gap-2 md:grid-cols-4">
            {statusCards.map((card) => (
              <LearningMetric key={card.label} {...card} />
            ))}
          </div>
        </div>
        <div
          data-testid="workbench-visual"
          data-motion="ambient"
          className="relative mx-auto h-56 w-full max-w-sm"
        >
          <LearningLabIllustration />
          <div className="aria-float absolute left-3 top-3 rounded-lg border-2 border-cyan-300 bg-cyan-100 px-3 py-2 text-xs font-black text-cyan-950 shadow-[0_6px_0_rgba(6,182,212,0.18)]">
            Prompt
          </div>
          <div className="aria-float-slow absolute bottom-4 right-3 rounded-lg border-2 border-orange-300 bg-orange-100 px-3 py-2 text-xs font-black text-orange-950 shadow-[0_6px_0_rgba(249,115,22,0.18)]">
            Review
          </div>
          <div className="aria-pop-in absolute right-8 top-9 h-9 w-9 rounded-lg border-2 border-emerald-300 bg-emerald-100 shadow-[0_5px_0_rgba(16,185,129,0.18)]" />
        </div>
      </div>
    </section>
  );
}

function LearningMetric({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone: "indigo" | "cyan" | "emerald" | "orange";
}) {
  const toneClass = {
    indigo: "border-indigo-200 bg-white text-indigo-950",
    cyan: "border-cyan-200 bg-cyan-50 text-cyan-950",
    emerald: "border-emerald-200 bg-emerald-50 text-emerald-950",
    orange: "border-orange-200 bg-orange-50 text-orange-950",
  }[tone];
  return (
    <div className={`min-w-0 rounded-lg border-2 px-3 py-2 shadow-[0_5px_0_rgba(129,140,248,0.12)] ${toneClass}`}>
      <div className="text-[10px] font-black uppercase opacity-70">{label}</div>
      <div className="mt-1 truncate font-mono text-sm font-black">{value}</div>
    </div>
  );
}

function LearningLabIllustration() {
  return (
    <svg
      role="img"
      aria-label="AI coding workbench illustration"
      viewBox="0 0 360 240"
      className="h-full w-full drop-shadow-[0_18px_25px_rgba(190,24,93,0.16)]"
    >
      <title>AI coding workbench illustration</title>
      <defs>
        <linearGradient id="lab-screen" x1="72" x2="278" y1="44" y2="188">
          <stop stopColor="#8E2D60" />
          <stop offset="1" stopColor="#F97316" />
        </linearGradient>
        <linearGradient id="lab-card" x1="96" x2="236" y1="68" y2="160">
          <stop stopColor="#FFFFFF" />
          <stop offset="1" stopColor="#FFF4EC" />
        </linearGradient>
      </defs>
      <rect x="58" y="40" width="244" height="148" rx="18" fill="#241B2F" />
      <rect x="68" y="50" width="224" height="128" rx="14" fill="url(#lab-screen)" />
      <path d="M138 188h84l12 24h-108l12-24Z" fill="#F9A8D4" />
      <rect x="110" y="210" width="140" height="16" rx="8" fill="#8E2D60" />
      <rect x="92" y="70" width="92" height="72" rx="14" fill="url(#lab-card)" />
      <rect x="106" y="88" width="54" height="9" rx="4.5" fill="#8E2D60" />
      <rect x="106" y="106" width="62" height="8" rx="4" fill="#F9A8D4" />
      <rect x="106" y="122" width="42" height="8" rx="4" fill="#F97316" />
      <rect x="198" y="78" width="62" height="34" rx="12" fill="#CFFAFE" />
      <path d="M212 98h32" stroke="#0891B2" strokeWidth="7" strokeLinecap="round" />
      <rect x="198" y="126" width="62" height="34" rx="12" fill="#DCFCE7" />
      <path d="M214 144h28" stroke="#10B981" strokeWidth="7" strokeLinecap="round" />
      <circle cx="281" cy="66" r="20" fill="#FED7AA" />
      <path d="m273 66 6 6 12-14" stroke="#F97316" strokeWidth="6" strokeLinecap="round" strokeLinejoin="round" />
      <circle className="aria-orbit" cx="73" cy="34" r="9" fill="#22D3EE" />
      <circle className="aria-orbit-delayed" cx="310" cy="164" r="7" fill="#F97316" />
      <path d="M46 128c16 8 30 8 42 0" stroke="#F97316" strokeWidth="8" strokeLinecap="round" />
    </svg>
  );
}

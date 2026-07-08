"use client";

import { type Insight } from "@/lib/api";

const SEVERITY_COLOR: Record<string, string> = {
  critical: "var(--mk-danger)",
  high: "var(--mk-danger)",
  medium: "var(--mk-warn)",
  low: "var(--mk-border-strong)",
  info: "var(--mk-border-strong)",
};

function severityTagClass(severity: string): string {
  if (severity === "critical" || severity === "high") return "mk-tag mk-tag--danger";
  if (severity === "medium") return "mk-tag mk-tag--warn";
  return "mk-tag";
}

/** A 5-block confidence meter. Texture, not alarm: never accent-colored. */
function ConfidenceMeter({ value }: { value: number }) {
  const filled = Math.round(Math.max(0, Math.min(1, value)) * 5);
  return (
    <span className="font-mono text-[11px]" title={`confidence ${value.toFixed(2)}`}>
      {[0, 1, 2, 3, 4].map((i) => (
        <span key={i} style={{ color: i < filled ? "var(--mk-text-2)" : "var(--mk-text-3)" }}>
          {i < filled ? "▰" : "▱"}
        </span>
      ))}
    </span>
  );
}

/** Render the narrative, highlighting [E#] citation markers. */
function Narrative({ text }: { text: string }) {
  const parts = text.split(/(\[E\d+\])/g);
  return (
    <p className="text-sm leading-relaxed" style={{ color: "var(--mk-text-1)" }}>
      {parts.map((part, i) =>
        /^\[E\d+\]$/.test(part) ? (
          <span key={i} className="font-mono text-xs" style={{ color: "var(--mk-accent)" }}>
            {part}
          </span>
        ) : (
          <span key={i}>{part}</span>
        ),
      )}
    </p>
  );
}

export function InsightCard({
  insight,
  onOpenEvidence,
  onTriage,
  busy,
  selected = false,
}: {
  insight: Insight;
  onOpenEvidence: (id: string) => void;
  onTriage: (id: string, status: "useful" | "dismissed") => void;
  busy: boolean;
  selected?: boolean;
}) {
  const rail = SEVERITY_COLOR[insight.severity] ?? "var(--mk-border)";
  const dismissed = insight.status === "dismissed";

  return (
    <div
      className="mk-card overflow-hidden"
      style={{
        borderLeft: `3px solid ${rail}`,
        opacity: dismissed ? 0.55 : 1,
        outline: selected ? "2px solid var(--mk-accent)" : "none",
        outlineOffset: "2px",
      }}
    >
      <div className="p-4">
        <div className="mb-2 flex items-center gap-2">
          <span className={severityTagClass(insight.severity)}>{insight.severity}</span>
          <ConfidenceMeter value={insight.confidence} />
          {insight.status === "useful" && <span className="mk-tag mk-tag--success">saved</span>}
          {dismissed && <span className="mk-tag">dismissed</span>}
        </div>

        <h3 className="mb-2 text-lg">{insight.title}</h3>
        <Narrative text={insight.body_md} />

        {insight.suggested_actions.length > 0 && (
          <div className="mt-3">
            <span className="mk-kicker">suggested actions</span>
            <ul className="mt-1 space-y-1">
              {insight.suggested_actions.map((a, i) => (
                <li key={i} className="flex gap-2 text-sm" style={{ color: "var(--mk-text-2)" }}>
                  <span style={{ color: "var(--mk-accent)" }}>&rarr;</span>
                  {a}
                </li>
              ))}
            </ul>
          </div>
        )}

        <div className="mt-4 flex gap-2">
          <button className="mk-btn text-xs" onClick={() => onOpenEvidence(insight.id)}>
            evidence
          </button>
          <button
            className="mk-btn text-xs"
            disabled={busy}
            onClick={() => onTriage(insight.id, "useful")}
          >
            save
          </button>
          <button
            className="mk-btn text-xs"
            disabled={busy}
            onClick={() => onTriage(insight.id, "dismissed")}
          >
            dismiss
          </button>
        </div>
      </div>
    </div>
  );
}

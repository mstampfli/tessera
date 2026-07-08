"use client";

import Link from "next/link";
import { use, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { CorrelationGraph } from "@/components/CorrelationGraph";

export default function EntityDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = use(params);
  const detail = useQuery({ queryKey: ["entity", id], queryFn: () => api.entity(id) });
  const [view, setView] = useState<"table" | "graph">("table");

  const e = detail.data?.entity;

  // Stable node/edge identity so the WebGL graph (and the user's pan/zoom) is
  // not torn down and rebuilt on every unrelated re-render.
  const graphNodes = useMemo(() => {
    if (!e || !detail.data) return [];
    return [
      { id: e.id, label: e.value, kind: e.kind, weight: e.mention_count },
      ...detail.data.neighborhood.map((n) => ({
        id: n.id,
        label: n.value,
        kind: n.kind,
        weight: n.source_count,
      })),
    ];
  }, [e, detail.data]);
  const graphEdges = useMemo(
    () =>
      e && detail.data
        ? detail.data.neighborhood.map((n) => ({ source: e.id, target: n.id, weight: n.score }))
        : [],
    [e, detail.data],
  );

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">entity</span>
        <h1 className="mt-1 break-all font-mono text-2xl">{e?.value ?? "..."}</h1>
        {e && (
          <div className="mt-2 flex flex-wrap items-center gap-2 text-sm" style={{ color: "var(--mk-text-2)" }}>
            <span className="mk-tag">{e.kind.replace("hash_", "")}</span>
            <span>
              seen in {detail.data?.documents.length ?? 0} documents, {e.mention_count} mentions
            </span>
          </div>
        )}
      </div>

      {/* Correlations lead with a table, not a graph: denser and more honest.
          The graph is an opt-in view of the same neighborhood. */}
      <section>
        <div className="flex items-center justify-between">
          <span className="mk-kicker">top correlations</span>
          {detail.data && detail.data.neighborhood.length > 0 && (
            <div className="flex gap-1" role="tablist" aria-label="correlation view">
              {(["table", "graph"] as const).map((v) => (
                <button
                  key={v}
                  role="tab"
                  aria-selected={view === v}
                  onClick={() => setView(v)}
                  className="mk-btn text-xs"
                  style={
                    view === v
                      ? { borderColor: "var(--mk-accent)", color: "var(--mk-accent)" }
                      : undefined
                  }
                >
                  {v}
                </button>
              ))}
            </div>
          )}
        </div>
        {detail.data && detail.data.neighborhood.length === 0 && (
          <p className="mt-2 text-sm" style={{ color: "var(--mk-text-3)" }}>
            no correlations yet.
          </p>
        )}
        {detail.data && detail.data.neighborhood.length > 0 && view === "graph" && e && (
          <CorrelationGraph
            centerId={e.id}
            ariaLabel={`Correlation graph for ${e.value}: ${detail.data.neighborhood.length} related entities`}
            nodes={graphNodes}
            edges={graphEdges}
          />
        )}
        {detail.data && detail.data.neighborhood.length > 0 && view === "table" && (
          <table className="mt-2 w-full text-sm">
            <thead>
              <tr className="text-left font-mono text-xs" style={{ color: "var(--mk-text-3)" }}>
                <th className="pb-2">correlate</th>
                <th className="pb-2">rel</th>
                <th className="pb-2 text-right">shared</th>
                <th className="pb-2 text-right">score</th>
              </tr>
            </thead>
            <tbody>
              {detail.data.neighborhood.map((n) => (
                <tr key={n.id} className="border-t" style={{ borderColor: "var(--mk-border)" }}>
                  <td className="py-2">
                    <Link
                      href={`/entities/${n.id}`}
                      className="font-mono hover:underline"
                      style={{ color: "var(--mk-accent)" }}
                    >
                      <span className="mk-tag mr-2">{n.kind.replace("hash_", "")}</span>
                      {n.value}
                    </Link>
                  </td>
                  <td className="py-2 font-mono text-xs" style={{ color: "var(--mk-text-3)" }}>
                    {n.rel}
                  </td>
                  <td className="py-2 text-right font-mono" style={{ color: "var(--mk-text-2)" }}>
                    {n.source_count}
                  </td>
                  <td className="py-2 text-right font-mono" style={{ color: "var(--mk-text-2)" }}>
                    {n.score.toFixed(2)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section>
        <span className="mk-kicker">occurrences</span>
        <ul className="mt-2 divide-y" style={{ borderColor: "var(--mk-border)" }}>
          {detail.data?.documents.map((d) => (
            <li key={d.id} className="py-2">
              <Link href={`/documents/${d.id}`} className="hover:underline" style={{ color: "var(--mk-text-1)" }}>
                {d.title ?? d.id}
              </Link>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

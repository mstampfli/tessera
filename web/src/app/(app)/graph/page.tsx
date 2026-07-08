"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { CorrelationGraph } from "@/components/CorrelationGraph";

const NODE_CAP = 250;

export default function GraphPage() {
  const [kind, setKind] = useState<string>("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [kinds, setKinds] = useState<string[]>([]);

  const q = useQuery({
    queryKey: ["graph", kind],
    queryFn: () => api.graph(kind || undefined, NODE_CAP),
  });

  // Learn the full kind list from the unfiltered view so the filter chips stay
  // stable when a kind is selected.
  useEffect(() => {
    if (kind === "" && q.data) {
      setKinds([...new Set(q.data.nodes.map((n) => n.kind))].sort());
    }
  }, [kind, q.data]);

  // Changing the filter can drop the selected node from the set.
  useEffect(() => {
    setSelectedId(null);
  }, [kind]);

  const graphNodes = useMemo(
    () =>
      q.data?.nodes.map((n) => ({
        id: n.id,
        label: n.value,
        kind: n.kind,
        weight: n.weight,
      })) ?? [],
    [q.data],
  );
  const graphEdges = useMemo(
    () =>
      q.data?.edges.map((e) => ({
        source: e.src_id,
        target: e.dst_id,
        weight: e.strength,
        method: e.method,
      })) ?? [],
    [q.data],
  );

  const selected = q.data?.nodes.find((n) => n.id === selectedId) ?? null;
  const neighbors = useMemo(() => {
    if (!q.data || !selectedId) return [];
    const byId = new Map(q.data.nodes.map((n) => [n.id, n]));
    return q.data.edges
      .filter((e) => e.src_id === selectedId || e.dst_id === selectedId)
      .map((e) => {
        const otherId = e.src_id === selectedId ? e.dst_id : e.src_id;
        return { node: byId.get(otherId), strength: e.strength, method: e.method };
      })
      .filter(
        (x): x is { node: NonNullable<typeof x.node>; strength: number; method: string } =>
          Boolean(x.node),
      )
      .sort((a, b) => b.strength - a.strength);
  }, [q.data, selectedId]);

  const shown = q.data?.nodes.length ?? 0;
  const total = q.data?.total ?? 0;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <span className="mk-kicker">correlation graph</span>
          <h1 className="mt-1 text-2xl">overview</h1>
          <p className="mt-1 text-sm" style={{ color: "var(--mk-text-3)" }}>
            {q.isLoading
              ? "loading ..."
              : `showing ${shown} of ${total} entities${
                  shown < total ? ` (most-connected first, capped at ${NODE_CAP})` : ""
                }. click a node to inspect it.`}
          </p>
        </div>
        <div className="flex flex-wrap gap-1" role="group" aria-label="filter by kind">
          <button
            className="mk-btn text-xs"
            aria-pressed={kind === ""}
            onClick={() => setKind("")}
            style={kind === "" ? { borderColor: "var(--mk-accent)", color: "var(--mk-accent)" } : undefined}
          >
            all
          </button>
          {kinds.map((k) => (
            <button
              key={k}
              className="mk-btn text-xs"
              aria-pressed={kind === k}
              onClick={() => setKind(k)}
              style={kind === k ? { borderColor: "var(--mk-accent)", color: "var(--mk-accent)" } : undefined}
            >
              {k.replace("hash_", "")}
            </button>
          ))}
        </div>
      </div>

      <div className="flex flex-col gap-4 lg:flex-row">
        <div className="min-w-0 flex-1">
          {q.data && q.data.nodes.length === 0 ? (
            <p className="mt-2 text-sm" style={{ color: "var(--mk-text-3)" }}>
              no entities to graph yet. ingest some data first.
            </p>
          ) : (
            <CorrelationGraph
              nodes={graphNodes}
              edges={graphEdges}
              selectedId={selectedId ?? undefined}
              onSelectNode={setSelectedId}
              height={560}
              ariaLabel={`Global correlation graph: ${shown} entities`}
            />
          )}
        </div>

        <aside className="w-full lg:w-80 lg:shrink-0">
          {!selected && (
            <div className="mk-card p-4 text-sm" style={{ color: "var(--mk-text-3)" }}>
              select a node to see the entity and its correlations. hover to isolate
              its connections; drag to pan, scroll to zoom.
            </div>
          )}
          {selected && (
            <div className="mk-frame p-4">
              <div className="mb-2 flex items-center justify-between">
                <span className="mk-tag mk-tag--accent">{selected.kind.replace("hash_", "")}</span>
                <button className="mk-btn text-xs" onClick={() => setSelectedId(null)}>
                  clear
                </button>
              </div>
              <p className="break-all font-mono text-sm" style={{ color: "var(--mk-text-1)" }}>
                {selected.value}
              </p>
              <p className="mt-1 text-xs" style={{ color: "var(--mk-text-3)" }}>
                {selected.weight} mentions, {neighbors.length} correlations shown
              </p>
              <Link
                href={`/entities/${selected.id}`}
                className="mk-btn mk-btn--primary mt-3 inline-flex text-xs"
              >
                open entity page
              </Link>

              {neighbors.length > 0 && (
                <>
                  <p className="mk-kicker mt-4">correlated with</p>
                  <ul className="mt-2 space-y-1">
                    {neighbors.slice(0, 20).map(({ node, strength, method }) => (
                      <li key={node.id}>
                        <button
                          className="flex w-full items-center gap-2 rounded px-1 py-1 text-left text-xs hover:underline"
                          onClick={() => setSelectedId(node.id)}
                          style={{ color: "var(--mk-text-2)" }}
                          title={method === "similar" ? "contextual similarity" : "co-occurs directly"}
                        >
                          <span className="mk-tag">{node.kind.replace("hash_", "")}</span>
                          <span className="truncate font-mono">{node.value}</span>
                          <span
                            className="ml-auto font-mono"
                            style={{
                              color: method === "similar" ? "var(--mk-text-3)" : "var(--mk-accent)",
                            }}
                          >
                            {Math.round(strength * 100)}
                            {method === "similar" ? "~" : ""}
                          </span>
                        </button>
                      </li>
                    ))}
                  </ul>
                  <p className="mt-2 text-[11px]" style={{ color: "var(--mk-text-3)" }}>
                    number = correlation strength (0-100). ~ marks a contextual
                    (semantic) link; solid values are direct co-occurrences.
                  </p>
                </>
              )}
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}

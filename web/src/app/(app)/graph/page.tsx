"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { CorrelationGraph } from "@/components/CorrelationGraph";
import { CorrelationReason } from "@/components/CorrelationReason";

const NODE_CAP = 250;

function methodColor(method: string): string {
  if (method === "similar") return "var(--mk-clay-1)";
  if (method === "temporal") return "var(--mk-green-1)";
  return "var(--mk-accent)";
}

export default function GraphPage() {
  const [kind, setKind] = useState<string>("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedEdge, setSelectedEdge] = useState<{ a: string; b: string } | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [minStrength, setMinStrength] = useState(0);
  const [kinds, setKinds] = useState<string[]>([]);

  const q = useQuery({
    queryKey: ["graph", kind],
    queryFn: () => api.graph(kind || undefined, NODE_CAP),
  });
  const bridgesQ = useQuery({ queryKey: ["bridges"], queryFn: () => api.bridges(30) });
  const corrQ = useQuery({
    queryKey: ["correlation", selectedEdge?.a, selectedEdge?.b],
    queryFn: () => api.correlation(selectedEdge!.a, selectedEdge!.b),
    enabled: !!selectedEdge,
  });
  // The inline "why" for an expanded neighbour in the node inspector.
  const whyQ = useQuery({
    queryKey: ["correlation", selectedId, expanded],
    queryFn: () => api.correlation(selectedId!, expanded!),
    enabled: !!selectedId && !!expanded,
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

  // Collapse the inline "why" when the selected node changes.
  useEffect(() => {
    setExpanded(null);
  }, [selectedId]);

  const graphNodes = useMemo(
    () =>
      q.data?.nodes.map((n) => ({
        id: n.id,
        label: n.value,
        kind: n.kind,
        weight: n.weight,
        community: n.community_id,
      })) ?? [],
    [q.data],
  );
  const graphEdges = useMemo(
    () =>
      q.data?.edges
        .filter((e) => e.strength >= minStrength)
        .map((e) => ({
          source: e.src_id,
          target: e.dst_id,
          weight: e.strength,
          method: e.method,
        })) ?? [],
    [q.data, minStrength],
  );
  const hiddenEdges = (q.data?.edges.length ?? 0) - graphEdges.length;

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
                }.`}
          </p>
          <p className="mt-1 text-[11px]" style={{ color: "var(--mk-text-3)" }}>
            click a node to inspect its correlations · hover a line for the link, click it
            for the evidence. lines:{" "}
            <span style={{ color: "var(--mk-accent)" }}>co-occurs</span>{" "}
            <span style={{ color: "var(--mk-clay-1)" }}>contextual</span>{" "}
            <span style={{ color: "var(--mk-green-1)" }}>same-time</span>{" "}
            <span style={{ color: "var(--mk-highlight)" }}>bridge</span>; node colour =
            community.
          </p>
          <label className="mt-2 flex items-center gap-2 text-[11px]" style={{ color: "var(--mk-text-3)" }}>
            min strength
            <input
              type="range"
              min={0}
              max={0.95}
              step={0.05}
              value={minStrength}
              onChange={(e) => setMinStrength(Number(e.target.value))}
              className="w-32"
              aria-label="minimum correlation strength"
            />
            <span className="font-mono" style={{ color: "var(--mk-text-2)" }}>
              {Math.round(minStrength * 100)}
            </span>
            {hiddenEdges > 0 && <span>({hiddenEdges} weaker links hidden)</span>}
          </label>
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
              onSelectNode={(id) => {
                setSelectedId(id);
                setSelectedEdge(null);
              }}
              onSelectEdge={(a, b) => {
                setSelectedEdge({ a, b });
                setSelectedId(null);
              }}
              height={560}
              ariaLabel={`Global correlation graph: ${shown} entities`}
            />
          )}
        </div>

        <aside className="w-full lg:w-80 lg:shrink-0">
          {selectedEdge && (
            <div className="mk-frame p-4">
              <div className="mb-2 flex items-center justify-between">
                <span className="mk-kicker">why correlated</span>
                <button className="mk-btn text-xs" onClick={() => setSelectedEdge(null)}>
                  clear
                </button>
              </div>
              {corrQ.isLoading && (
                <p className="text-sm" style={{ color: "var(--mk-text-3)" }}>
                  loading ...
                </p>
              )}
              {corrQ.data && (
                <>
                  <p className="mb-2 font-mono text-xs" style={{ color: "var(--mk-text-2)" }}>
                    {corrQ.data.a.kind.replace("hash_", "")}: {corrQ.data.a.value}
                    <br />
                    <span style={{ color: "var(--mk-text-3)" }}>&harr;</span>
                    <br />
                    {corrQ.data.b.kind.replace("hash_", "")}: {corrQ.data.b.value}
                  </p>
                  <CorrelationReason detail={corrQ.data} />
                </>
              )}
            </div>
          )}
          {!selectedEdge && !selected && (
            <div className="mk-frame p-4">
              <span className="mk-kicker">bridges</span>
              <p className="mt-1 text-xs" style={{ color: "var(--mk-text-3)" }}>
                non-obvious links: entities in different communities (never mentioned
                together) that are contextually related. select a node to inspect it.
              </p>
              {bridgesQ.data && bridgesQ.data.length === 0 && (
                <p className="mt-3 text-sm" style={{ color: "var(--mk-text-3)" }}>
                  no cross-community bridges yet.
                </p>
              )}
              <ul className="mt-3 space-y-2">
                {bridgesQ.data?.slice(0, 15).map((br) => (
                  <li key={`${br.a_id}-${br.b_id}`} className="text-xs">
                    <div className="flex items-center justify-between">
                      <span className="mk-tag mk-tag--warn">bridge</span>
                      <span className="font-mono" style={{ color: "var(--mk-highlight)" }}>
                        {Math.round(br.strength * 100)}
                      </span>
                    </div>
                    <button
                      className="mt-1 block w-full truncate text-left font-mono hover:underline"
                      onClick={() => setSelectedId(br.a_id)}
                      style={{ color: "var(--mk-text-2)" }}
                    >
                      {br.a_kind.replace("hash_", "")}: {br.a_value}
                    </button>
                    <div className="text-center" style={{ color: "var(--mk-text-3)" }}>
                      &darr;
                    </div>
                    <button
                      className="block w-full truncate text-left font-mono hover:underline"
                      onClick={() => setSelectedId(br.b_id)}
                      style={{ color: "var(--mk-text-2)" }}
                    >
                      {br.b_kind.replace("hash_", "")}: {br.b_value}
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {selected && !selectedEdge && (
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
                  <p className="mk-kicker mt-4">correlated with — tap one for why</p>
                  <ul className="mt-2 space-y-1">
                    {neighbors.slice(0, 20).map(({ node, strength, method }) => {
                      const open = expanded === node.id;
                      return (
                        <li
                          key={node.id}
                          className="rounded"
                          style={{ background: open ? "var(--mk-surface-2)" : undefined }}
                        >
                          <button
                            className="flex w-full items-center gap-2 px-1 py-1 text-left text-xs"
                            onClick={() => setExpanded(open ? null : node.id)}
                            aria-expanded={open}
                          >
                            <span style={{ color: "var(--mk-text-3)", width: 9 }}>
                              {open ? "▾" : "▸"}
                            </span>
                            <span className="mk-tag">{node.kind.replace("hash_", "")}</span>
                            <span className="truncate font-mono" style={{ color: "var(--mk-text-2)" }}>
                              {node.value}
                            </span>
                            <span className="ml-auto font-mono" style={{ color: methodColor(method) }}>
                              {Math.round(strength * 100)}
                            </span>
                          </button>
                          {open && (
                            <div className="px-2 pb-2">
                              {whyQ.isLoading && (
                                <p className="text-[11px]" style={{ color: "var(--mk-text-3)" }}>
                                  loading ...
                                </p>
                              )}
                              {whyQ.data && <CorrelationReason detail={whyQ.data} />}
                              <button
                                className="mk-btn mt-2 text-[11px]"
                                onClick={() => setSelectedId(node.id)}
                              >
                                focus this entity
                              </button>
                            </div>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                  <p className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[11px]" style={{ color: "var(--mk-text-3)" }}>
                    <span style={{ color: "var(--mk-accent)" }}>&#9632; co-occurs</span>
                    <span style={{ color: "var(--mk-clay-1)" }}>&#9632; contextual</span>
                    <span style={{ color: "var(--mk-green-1)" }}>&#9632; same time</span>
                    <span>number = strength 0-100</span>
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

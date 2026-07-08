"use client";

import Link from "next/link";
import { use, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { CorrelationGraph } from "@/components/CorrelationGraph";

export default function ClusterDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = use(params);
  const detail = useQuery({ queryKey: ["cluster", id], queryFn: () => api.cluster(id) });
  const [showGraph, setShowGraph] = useState(false);
  // The network is heavy (WebGL), so fetch it only when the user opens it.
  const graph = useQuery({
    queryKey: ["cluster-graph", id],
    queryFn: () => api.clusterGraph(id),
    enabled: showGraph,
  });
  const c = detail.data?.cluster;

  // Stable identity so a background refetch does not rebuild the graph.
  const graphNodes = useMemo(
    () =>
      graph.data?.nodes.map((n) => ({
        id: n.id,
        label: n.value,
        kind: n.kind,
        weight: n.weight,
      })) ?? [],
    [graph.data],
  );
  const graphEdges = useMemo(
    () =>
      graph.data?.edges.map((e) => ({
        source: e.src_id,
        target: e.dst_id,
        weight: e.strength,
        method: e.method,
      })) ?? [],
    [graph.data],
  );

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">cluster</span>
        <h1 className="mt-1 text-2xl">{c?.label ?? "(unlabeled cluster)"}</h1>
        {c && (
          <p className="mt-1 text-sm" style={{ color: "var(--mk-text-3)" }}>
            {c.size} member chunks
          </p>
        )}
      </div>

      <section>
        <div className="flex items-center justify-between">
          <span className="mk-kicker">correlation network</span>
          <button className="mk-btn text-xs" onClick={() => setShowGraph((s) => !s)}>
            {showGraph ? "hide" : "show"}
          </button>
        </div>
        {showGraph && graph.isLoading && (
          <p className="mt-2 text-sm" style={{ color: "var(--mk-text-3)" }}>
            building network ...
          </p>
        )}
        {showGraph && graph.data && (
          <CorrelationGraph
            ariaLabel={`Entity correlation network for this cluster: ${graph.data.nodes.length} entities`}
            nodes={graphNodes}
            edges={graphEdges}
          />
        )}
        {showGraph && graph.data && graph.data.nodes.length === 0 && (
          <p className="mt-2 text-sm" style={{ color: "var(--mk-text-3)" }}>
            no entities extracted in this cluster yet.
          </p>
        )}
      </section>

      <section>
        <span className="mk-kicker">members</span>
        <ul className="mt-2 space-y-2">
          {detail.data?.members.map((m) => (
            <li key={m.chunk_id} className="mk-card p-3">
              <div className="mb-1 flex items-center gap-2">
                <Link
                  href={`/documents/${m.document_id}?chunk=${m.chunk_id}`}
                  className="font-mono text-sm hover:underline"
                  style={{ color: "var(--mk-accent)" }}
                >
                  {m.title ?? "untitled"}
                </Link>
                <span className="ml-auto font-mono text-[11px]" style={{ color: "var(--mk-text-3)" }}>
                  sim {m.similarity.toFixed(3)}
                </span>
              </div>
              <p className="text-sm" style={{ color: "var(--mk-text-2)" }}>
                {m.excerpt}
              </p>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

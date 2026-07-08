"use client";

import { useEffect, useRef } from "react";
import { useRouter } from "next/navigation";

export type GraphNode = { id: string; label: string; kind: string; weight: number };
export type GraphEdge = { source: string; target: string; weight: number };

/**
 * A correlation network: entity nodes sized by weight, edges thickened by
 * correlation strength. Used for both the entity ego view (one center + its
 * neighbors) and the cluster campaign view (an entity co-occurrence network).
 *
 * Rendering is WebGL (sigma) with every dependency bundled (no CDN); the
 * ForceAtlas2 layout runs synchronously (no web worker, so the strict
 * `default-src 'self'` CSP with no `worker-src` cannot block it); and all colors
 * are read from the live `--mk-*` design tokens, so it tracks the active theme.
 * Clicking any node opens that entity.
 */
export function CorrelationGraph({
  nodes,
  edges,
  centerId,
  height = 440,
  ariaLabel,
}: {
  nodes: GraphNode[];
  edges: GraphEdge[];
  centerId?: string;
  height?: number;
  ariaLabel: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const router = useRouter();

  useEffect(() => {
    const container = containerRef.current;
    if (!container || nodes.length === 0) return;

    let sigma: { kill: () => void } | null = null;
    let cancelled = false;

    const short = (s: string) => (s.length > 22 ? `${s.slice(0, 10)}...${s.slice(-6)}` : s);
    const token = (name: string, fallback: string) => {
      const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
      return v || fallback;
    };

    void (async () => {
      const [{ default: Graph }, sigmaMod, faMod] = await Promise.all([
        import("graphology"),
        import("sigma"),
        import("graphology-layout-forceatlas2"),
      ]);
      if (cancelled || !containerRef.current) return;
      const Sigma = sigmaMod.default;
      const forceAtlas2 = faMod.default;

      const accent = token("--mk-accent", "#e0731d");
      const nodeColor = token("--mk-text-2", "#c7b299");
      const edgeColor = token("--mk-border-strong", "#5a4023");
      const labelColor = token("--mk-text-1", "#f4ead9");
      const bg = token("--mk-surface-1", "#221810");

      const graph = new Graph({ type: "undirected" });
      const maxNodeW = Math.max(1, ...nodes.map((n) => n.weight));
      const maxEdgeW = Math.max(1, ...edges.map((e) => e.weight));

      nodes.forEach((n, i) => {
        if (graph.hasNode(n.id)) return;
        const isCenter = n.id === centerId;
        const angle = (i / nodes.length) * 2 * Math.PI;
        graph.addNode(n.id, {
          label: `${n.kind.replace("hash_", "")}: ${short(n.label)}`,
          x: isCenter ? 0 : Math.cos(angle) * 10,
          y: isCenter ? 0 : Math.sin(angle) * 10,
          size: isCenter ? 14 : 4 + (n.weight / maxNodeW) * 9,
          color: isCenter ? accent : nodeColor,
        });
      });

      edges.forEach((e) => {
        if (e.source === e.target) return;
        if (!graph.hasNode(e.source) || !graph.hasNode(e.target)) return;
        if (graph.hasEdge(e.source, e.target)) return;
        graph.addEdge(e.source, e.target, {
          size: 0.6 + (e.weight / maxEdgeW) * 3.4,
          color: edgeColor,
        });
      });

      // Synchronous layout (no worker): safe under the strict CSP.
      const settings = forceAtlas2.inferSettings(graph);
      forceAtlas2.assign(graph, {
        iterations: 300,
        settings: { ...settings, gravity: 1.2, scalingRatio: 12 },
      });

      if (cancelled || !containerRef.current) return;
      sigma = new Sigma(graph, containerRef.current, {
        renderLabels: true,
        labelColor: { color: labelColor },
        labelFont: "var(--font-ibm-mono), ui-monospace, monospace",
        labelSize: 11,
        defaultNodeColor: nodeColor,
        defaultEdgeColor: edgeColor,
        minCameraRatio: 0.3,
        maxCameraRatio: 3,
      }) as unknown as { kill: () => void };

      (sigma as unknown as { on: (e: string, cb: (p: { node: string }) => void) => void }).on(
        "clickNode",
        ({ node }) => {
          router.push(`/entities/${node}`);
        },
      );
      container.style.background = bg;
    })();

    return () => {
      cancelled = true;
      if (sigma) sigma.kill();
    };
  }, [nodes, edges, centerId, router]);

  if (nodes.length === 0) {
    return (
      <p className="mt-2 text-sm" style={{ color: "var(--mk-text-3)" }}>
        no correlations to graph yet.
      </p>
    );
  }

  return (
    <div
      ref={containerRef}
      className="mt-2 w-full overflow-hidden rounded"
      style={{ height, border: "1px solid var(--mk-border)", cursor: "grab" }}
      role="img"
      aria-label={ariaLabel}
    />
  );
}

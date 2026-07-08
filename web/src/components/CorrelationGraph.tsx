"use client";

import { useEffect, useRef } from "react";
import { useRouter } from "next/navigation";
import type Graph from "graphology";

export type GraphNode = { id: string; label: string; kind: string; weight: number };
export type GraphEdge = { source: string; target: string; weight: number };

// Minimal structural types so we do not depend on sigma's exported type surface.
type SigmaLike = {
  kill: () => void;
  refresh: () => void;
  on: (event: string, cb: (payload: { node: string }) => void) => void;
  setSetting: (key: string, value: unknown) => void;
};
type NodeData = { color?: string; size?: number; label?: string; zIndex?: number };
type EdgeData = { hidden?: boolean; color?: string };

/**
 * A correlation network: entity nodes sized by weight, edges thickened by
 * correlation strength. Used for the entity ego view (one center + neighbors),
 * the cluster campaign view, and the global overview graph.
 *
 * Rendering is WebGL (sigma) with every dependency bundled (no CDN); the
 * ForceAtlas2 layout runs synchronously (no web worker, so the strict
 * `default-src 'self'` CSP with no `worker-src` cannot block it); and all colors
 * are read from the live `--mk-*` design tokens, so it tracks the active theme.
 *
 * Interaction: hovering a node dims everything not adjacent to it. If
 * `onSelectNode` is given, clicking a node selects it (and clicking empty space
 * clears the selection) instead of navigating; otherwise a click opens that
 * entity. Selection and hover repaint via reducers, never a rebuild.
 */
export function CorrelationGraph({
  nodes,
  edges,
  centerId,
  selectedId,
  onSelectNode,
  height = 440,
  ariaLabel,
}: {
  nodes: GraphNode[];
  edges: GraphEdge[];
  centerId?: string;
  selectedId?: string;
  onSelectNode?: (id: string | null) => void;
  height?: number;
  ariaLabel: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const router = useRouter();
  const sigmaRef = useRef<SigmaLike | null>(null);
  const hoveredRef = useRef<string | null>(null);
  // Latest props for the event handlers, so changing them never rebuilds sigma.
  const onSelectRef = useRef(onSelectNode);
  const selectedRef = useRef(selectedId);
  const centerRef = useRef(centerId);
  onSelectRef.current = onSelectNode;
  selectedRef.current = selectedId;
  centerRef.current = centerId;

  useEffect(() => {
    const container = containerRef.current;
    if (!container || nodes.length === 0) return;

    let cancelled = false;
    let sigma: SigmaLike | null = null;

    const short = (s: string) => (s.length > 22 ? `${s.slice(0, 10)}...${s.slice(-6)}` : s);
    const token = (name: string, fallback: string) => {
      const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
      return v || fallback;
    };

    void (async () => {
      const [graphMod, sigmaMod, faMod] = await Promise.all([
        import("graphology"),
        import("sigma"),
        import("graphology-layout-forceatlas2"),
      ]);
      if (cancelled || !containerRef.current) return;
      const GraphCtor = graphMod.default;
      const SigmaCtor = sigmaMod.default;
      const forceAtlas2 = faMod.default;

      const accent = token("--mk-accent", "#e0731d");
      const nodeColor = token("--mk-text-2", "#c7b299");
      const edgeColor = token("--mk-border-strong", "#5a4023");
      const dimColor = token("--mk-text-3", "#8d7456");
      const labelColor = token("--mk-text-1", "#f4ead9");
      const bg = token("--mk-surface-1", "#221810");

      const graph: Graph = new GraphCtor({ type: "undirected" });
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
      sigma = new SigmaCtor(graph, containerRef.current, {
        renderLabels: true,
        labelColor: { color: labelColor },
        labelFont: "var(--font-ibm-mono), ui-monospace, monospace",
        labelSize: 11,
        defaultNodeColor: nodeColor,
        defaultEdgeColor: edgeColor,
        minCameraRatio: 0.3,
        maxCameraRatio: 3,
      }) as unknown as SigmaLike;
      sigmaRef.current = sigma;

      const adjacent = (a: string, b: string) => a === b || graph.hasEdge(a, b);

      sigma.setSetting("nodeReducer", (node: string, data: NodeData): NodeData => {
        const res: NodeData = { ...data };
        const hov = hoveredRef.current;
        if (selectedRef.current === node) {
          res.color = accent;
          res.size = (data.size ?? 6) * 1.3;
          res.zIndex = 2;
        }
        if (hov && !adjacent(hov, node)) {
          res.color = dimColor;
          res.label = "";
        }
        return res;
      });
      sigma.setSetting("edgeReducer", (edge: string, data: EdgeData): EdgeData => {
        const res: EdgeData = { ...data };
        const hov = hoveredRef.current;
        if (hov) {
          const [s, t] = graph.extremities(edge);
          if (s !== hov && t !== hov) res.hidden = true;
        }
        return res;
      });

      sigma.on("enterNode", ({ node }) => {
        hoveredRef.current = node;
        container.style.cursor = "pointer";
        sigma?.refresh();
      });
      sigma.on("leaveNode", () => {
        hoveredRef.current = null;
        container.style.cursor = "grab";
        sigma?.refresh();
      });
      sigma.on("clickNode", ({ node }) => {
        if (onSelectRef.current) onSelectRef.current(node);
        else if (node !== centerRef.current) router.push(`/entities/${node}`);
      });
      sigma.on("clickStage", () => {
        onSelectRef.current?.(null);
      });

      container.style.background = bg;
    })();

    return () => {
      cancelled = true;
      if (sigma) sigma.kill();
      sigmaRef.current = null;
    };
  }, [nodes, edges, centerId, router]);

  // Repaint (not rebuild) when the external selection changes.
  useEffect(() => {
    sigmaRef.current?.refresh();
  }, [selectedId]);

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

"use client";

import { useEffect, useRef } from "react";
import { useRouter } from "next/navigation";
import type Graph from "graphology";

export type GraphNode = {
  id: string;
  label: string;
  kind: string;
  weight: number;
  community?: number | null;
};
// weight is the correlation strength in [0, 1]; method distinguishes a direct
// co-occurrence from a semantic (contextual) similarity.
export type GraphEdge = { source: string; target: string; weight: number; method?: string };

// Minimal structural types so we do not depend on sigma's exported type surface.
type SigmaLike = {
  kill: () => void;
  refresh: () => void;
  on: (event: string, cb: (payload: { node: string; edge: string }) => void) => void;
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
  onSelectEdge,
  height = 440,
  ariaLabel,
}: {
  nodes: GraphNode[];
  edges: GraphEdge[];
  centerId?: string;
  selectedId?: string;
  onSelectNode?: (id: string | null) => void;
  onSelectEdge?: (a: string, b: string) => void;
  height?: number;
  ariaLabel: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const tooltipRef = useRef<HTMLDivElement>(null);
  const mouseRef = useRef({ x: 0, y: 0 });
  const onSelectEdgeRef = useRef(onSelectEdge);
  onSelectEdgeRef.current = onSelectEdge;
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
      const semanticColor = token("--mk-clay-1", "#3c2a19");
      const temporalColor = token("--mk-green-1", "#8fa05a");
      const bridgeColor = token("--mk-highlight", "#eda43d");
      const dimColor = token("--mk-text-3", "#8d7456");
      const labelColor = token("--mk-text-1", "#f4ead9");
      const chipBg = token("--mk-surface-2", "#2b1e13");
      const bg = token("--mk-surface-1", "#221810");

      // A small categorical palette for communities (maka's warm hues only).
      const palette = [
        token("--mk-orange-1", "#e0731d"),
        token("--mk-green-1", "#8fa05a"),
        token("--mk-amber-1", "#eda43d"),
        token("--mk-red-1", "#d9503c"),
        token("--mk-cream-2", "#c7b299"),
        token("--mk-clay-2", "#5a4023"),
      ];
      const communityColor = (c: number | null | undefined) =>
        c == null ? nodeColor : palette[((c % palette.length) + palette.length) % palette.length];

      const graph: Graph = new GraphCtor({ type: "undirected" });
      const maxNodeW = Math.max(1, ...nodes.map((n) => n.weight));
      const maxEdgeW = Math.max(1, ...edges.map((e) => e.weight));
      const communityOf = new Map(nodes.map((n) => [n.id, n.community ?? null]));

      nodes.forEach((n, i) => {
        if (graph.hasNode(n.id)) return;
        const isCenter = n.id === centerId;
        const angle = (i / nodes.length) * 2 * Math.PI;
        graph.addNode(n.id, {
          label: `${n.kind.replace("hash_", "")}: ${short(n.label)}`,
          x: isCenter ? 0 : Math.cos(angle) * 10,
          y: isCenter ? 0 : Math.sin(angle) * 10,
          size: isCenter ? 14 : 4 + (n.weight / maxNodeW) * 9,
          color: isCenter ? accent : communityColor(n.community),
        });
      });

      edges.forEach((e) => {
        if (e.source === e.target) return;
        if (!graph.hasNode(e.source) || !graph.hasNode(e.target)) return;
        if (graph.hasEdge(e.source, e.target)) return;
        // A semantic edge across two communities is a bridge (a non-obvious link
        // between things never stated together): draw it highlighted. Other
        // semantic edges are quieter than direct co-occurrence.
        const semantic = e.method === "similar";
        const temporal = e.method === "temporal";
        const ca = communityOf.get(e.source);
        const cb = communityOf.get(e.target);
        const bridge = semantic && ca != null && cb != null && ca !== cb;
        const color = bridge
          ? bridgeColor
          : temporal
            ? temporalColor
            : semantic
              ? semanticColor
              : edgeColor;
        graph.addEdge(e.source, e.target, {
          // A wider floor keeps thin edges hittable for hover/click.
          size: (bridge ? 1.6 : 1.2) + (e.weight / maxEdgeW) * 3.0,
          color,
          method: bridge ? "bridge" : (e.method ?? "co_occurs"),
          strength: e.weight,
        });
      });

      // Synchronous layout (no worker): safe under the strict CSP.
      const settings = forceAtlas2.inferSettings(graph);
      forceAtlas2.assign(graph, {
        iterations: 300,
        settings: { ...settings, gravity: 1.2, scalingRatio: 12 },
      });

      // Sigma's built-in hover paints a white label box; on the dark theme the
      // light label text vanishes on it. Draw our own themed chip instead.
      const drawHoverLabel = (
        ctx: CanvasRenderingContext2D,
        data: { x: number; y: number; size: number; label?: string | null },
        s: { labelSize?: number; labelFont?: string },
      ) => {
        if (!data.label) return;
        const size = s.labelSize ?? 11;
        ctx.font = `${size}px ${s.labelFont ?? "ui-monospace, monospace"}`;
        const textW = ctx.measureText(data.label).width;
        const px = data.x + data.size + 3;
        const py = data.y + size / 3;
        ctx.fillStyle = chipBg;
        ctx.fillRect(px - 3, py - size, textW + 8, size + 6);
        ctx.fillStyle = labelColor;
        ctx.fillText(data.label, px, py);
      };

      if (cancelled || !containerRef.current) return;
      sigma = new SigmaCtor(graph, containerRef.current, {
        renderLabels: true,
        labelColor: { color: labelColor },
        labelFont: "var(--font-ibm-mono), ui-monospace, monospace",
        labelSize: 11,
        defaultNodeColor: nodeColor,
        defaultEdgeColor: edgeColor,
        defaultDrawNodeHover: drawHoverLabel,
        // Sigma does not emit edge hover/click events unless asked.
        enableEdgeEvents: true,
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

      // Edge interaction: hover shows a small tooltip with the method + strength;
      // click asks the parent to explain the correlation in full.
      const label = (n: string) => String(graph.getNodeAttribute(n, "label") ?? n);
      const pct = (s: unknown) => Math.round((typeof s === "number" ? s : 0) * 100);
      sigma.on("enterEdge", ({ edge }: { edge: string }) => {
        const [s, t] = graph.extremities(edge);
        const method = String(graph.getEdgeAttribute(edge, "method"));
        const strength = graph.getEdgeAttribute(edge, "strength");
        const tip = tooltipRef.current;
        if (tip) {
          tip.textContent = `${method} ${pct(strength)}  ·  ${label(s)} — ${label(t)}`;
          tip.style.left = `${mouseRef.current.x + 12}px`;
          tip.style.top = `${mouseRef.current.y + 12}px`;
          tip.style.display = "block";
        }
        container.style.cursor = "pointer";
      });
      sigma.on("leaveEdge", () => {
        if (tooltipRef.current) tooltipRef.current.style.display = "none";
        container.style.cursor = "grab";
      });
      sigma.on("clickEdge", ({ edge }: { edge: string }) => {
        const [s, t] = graph.extremities(edge);
        onSelectEdgeRef.current?.(s, t);
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
      className="relative mt-2 w-full"
      onMouseMove={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        mouseRef.current = { x: e.clientX - r.left, y: e.clientY - r.top };
      }}
    >
      <div
        ref={containerRef}
        className="w-full overflow-hidden rounded"
        style={{ height, border: "1px solid var(--mk-border)", cursor: "grab" }}
        role="img"
        aria-label={ariaLabel}
      />
      <div
        ref={tooltipRef}
        className="pointer-events-none absolute z-10 rounded px-2 py-1 font-mono text-[11px]"
        style={{
          display: "none",
          background: "var(--mk-surface-2)",
          color: "var(--mk-text-1)",
          border: "1px solid var(--mk-border)",
          boxShadow: "var(--mk-shadow-pop)",
        }}
      />
    </div>
  );
}

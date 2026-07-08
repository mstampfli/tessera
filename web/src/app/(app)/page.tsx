"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type Insight } from "@/lib/api";
import { InsightCard } from "@/components/InsightCard";
import { EvidenceDrawer } from "@/components/EvidenceDrawer";

const LAST_VISIT_KEY = "tessera-last-visit";

export default function FeedPage() {
  const queryClient = useQueryClient();
  const insights = useQuery({ queryKey: ["insights"], queryFn: () => api.insights() });
  const sources = useQuery({ queryKey: ["sources"], queryFn: api.sources });
  const [drawer, setDrawer] = useState<string | null>(null);
  const [selected, setSelected] = useState(0);
  const cardRefs = useRef<(HTMLDivElement | null)[]>([]);

  // Read the previous visit time once, then stamp the current visit for next time.
  const lastVisit = useRef<number>(0);
  useEffect(() => {
    const stored = Number(localStorage.getItem(LAST_VISIT_KEY) ?? 0);
    lastVisit.current = stored;
    localStorage.setItem(LAST_VISIT_KEY, String(Date.now()));
  }, []);

  const triage = useMutation({
    mutationFn: ({ id, status }: { id: string; status: "useful" | "dismissed" }) =>
      api.insightFeedback(id, status),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["insights"] }),
  });

  const list = useMemo(() => insights.data ?? [], [insights.data]);
  const hasInsights = list.length > 0;
  const hasData = (sources.data?.length ?? 0) > 0;

  const newSinceVisit = useMemo(
    () => (lastVisit.current ? list.filter((i) => Date.parse(i.created_at) > lastVisit.current).length : 0),
    [list],
  );

  // Keyboard triage: j/k move, a save, d dismiss, e/Enter open evidence, Esc close.
  const onKey = useCallback(
    (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
      if (!hasInsights) return;
      const current: Insight | undefined = list[selected];
      switch (e.key) {
        case "j":
          setSelected((s) => Math.min(s + 1, list.length - 1));
          break;
        case "k":
          setSelected((s) => Math.max(s - 1, 0));
          break;
        case "a":
          if (current) triage.mutate({ id: current.id, status: "useful" });
          break;
        case "d":
          if (current) triage.mutate({ id: current.id, status: "dismissed" });
          break;
        case "e":
        case "Enter":
          if (current) setDrawer(current.id);
          break;
        case "Escape":
          setDrawer(null);
          break;
        default:
      }
    },
    [hasInsights, list, selected, triage],
  );

  useEffect(() => {
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onKey]);

  useEffect(() => {
    cardRefs.current[selected]?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }, [selected]);

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">feed</span>
        <h1 className="mt-1 text-2xl">what to look at</h1>
      </div>

      {/* Since-last-visit delta strip. */}
      {hasInsights && (
        <div
          className="flex items-center gap-3 rounded border px-3 py-2 font-mono text-xs"
          style={{ borderColor: "var(--mk-border)", color: "var(--mk-text-2)" }}
        >
          <span style={{ color: newSinceVisit > 0 ? "var(--mk-accent)" : "var(--mk-text-3)" }}>
            {newSinceVisit > 0 ? `${newSinceVisit} new since last visit` : "nothing new since last visit"}
          </span>
          <span style={{ color: "var(--mk-text-3)" }}>
            {list.length} insights - keys: j/k move, a save, d dismiss, e evidence
          </span>
        </div>
      )}

      {!hasData && !hasInsights && <OnboardingEmptyState />}

      {hasData && !hasInsights && (
        <div className="mk-card p-5">
          <p className="text-sm" style={{ color: "var(--mk-text-2)" }}>
            Data is ingested but no insights have been synthesized yet. Insights appear once a group
            of related material forms a cluster. In the meantime, search and ask over what you have.
          </p>
          <div className="mt-3 flex gap-2">
            <Link href="/search" className="mk-btn">search the base</Link>
            <Link href="/entities" className="mk-btn">browse entities</Link>
          </div>
        </div>
      )}

      {hasInsights && (
        <div className="space-y-4">
          {list.map((insight, i) => (
            <div key={insight.id} ref={(el) => { cardRefs.current[i] = el; }}>
              <InsightCard
                insight={insight}
                selected={i === selected}
                busy={triage.isPending}
                onOpenEvidence={(id) => setDrawer(id)}
                onTriage={(id, status) => triage.mutate({ id, status })}
              />
            </div>
          ))}
        </div>
      )}

      {drawer && <EvidenceDrawer insightId={drawer} onClose={() => setDrawer(null)} />}
    </div>
  );
}

function OnboardingEmptyState() {
  return (
    <div className="mk-frame p-8 text-center">
      <span className="mk-kicker">getting started</span>
      <h2 className="mt-2 text-xl">Feed it data to begin</h2>
      <p className="mx-auto mt-2 max-w-md text-sm" style={{ color: "var(--mk-text-2)" }}>
        Nothing has been ingested yet. Drop a file anywhere, or use the ingest button, and the
        pipeline will chunk, embed, extract entities, and cluster it. Correlated insights land here.
      </p>
      <ol
        className="mx-auto mt-5 grid max-w-md gap-2 text-left font-mono text-sm"
        style={{ color: "var(--mk-text-2)" }}
      >
        <li>1. ingest data (files, paste, logs, IOCs)</li>
        <li>2. the pipeline correlates and clusters it</li>
        <li>3. actionable, cited insights appear here</li>
      </ol>
    </div>
  );
}

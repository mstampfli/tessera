"use client";

import Link from "next/link";
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { InsightCard } from "@/components/InsightCard";
import { EvidenceDrawer } from "@/components/EvidenceDrawer";

export default function FeedPage() {
  const queryClient = useQueryClient();
  const insights = useQuery({ queryKey: ["insights"], queryFn: () => api.insights() });
  const sources = useQuery({ queryKey: ["sources"], queryFn: api.sources });
  const [drawer, setDrawer] = useState<string | null>(null);

  const triage = useMutation({
    mutationFn: ({ id, status }: { id: string; status: "useful" | "dismissed" }) =>
      api.insightFeedback(id, status),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["insights"] }),
  });

  const hasInsights = (insights.data?.length ?? 0) > 0;
  const hasData = (sources.data?.length ?? 0) > 0;

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">feed</span>
        <h1 className="mt-1 text-2xl">what to look at</h1>
      </div>

      {!hasData && !hasInsights && <OnboardingEmptyState />}

      {hasData && !hasInsights && (
        <div className="mk-card p-5">
          <p className="text-sm" style={{ color: "var(--mk-text-2)" }}>
            Data is ingested but no insights have been synthesized yet. Insights appear once a group
            of related material forms a cluster. In the meantime, search and ask over what you have.
          </p>
          <div className="mt-3 flex gap-2">
            <Link href="/search" className="mk-btn">
              search the base
            </Link>
            <Link href="/entities" className="mk-btn">
              browse entities
            </Link>
          </div>
        </div>
      )}

      {hasInsights && (
        <div className="space-y-4">
          {insights.data!.map((insight) => (
            <InsightCard
              key={insight.id}
              insight={insight}
              busy={triage.isPending}
              onOpenEvidence={(id) => setDrawer(id)}
              onTriage={(id, status) => triage.mutate({ id, status })}
            />
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

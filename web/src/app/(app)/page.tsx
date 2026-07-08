"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export default function FeedPage() {
  const sources = useQuery({ queryKey: ["sources"], queryFn: api.sources });
  const hasData = (sources.data?.length ?? 0) > 0;

  return (
    <div className="space-y-8">
      <div>
        <span className="mk-kicker">feed</span>
        <h1 className="mt-1 text-2xl">what changed</h1>
      </div>

      {!hasData ? (
        <OnboardingEmptyState />
      ) : (
        <>
          {/* The insight feed lands in M3; until then the feed surfaces recent
              ingestion so the landing page is never empty once data exists. */}
          <section>
            <p className="mb-3 text-sm" style={{ color: "var(--mk-text-3)" }}>
              Correlated insights appear here once clustering and synthesis land (M3). For now,
              search and ask over everything you have ingested.
            </p>
            <div className="flex flex-wrap gap-2">
              <Link href="/search" className="mk-btn">
                search the base
              </Link>
              <Link href="/sources" className="mk-btn">
                view sources
              </Link>
            </div>
          </section>

          <section>
            <span className="mk-kicker">recent sources</span>
            <ul className="mt-2 divide-y" style={{ borderColor: "var(--mk-border)" }}>
              {sources.data!.slice(0, 8).map((s) => (
                <li key={s.id} className="py-2">
                  <Link
                    href={`/sources/${s.id}`}
                    className="flex items-center justify-between hover:underline"
                  >
                    <span>{s.name}</span>
                    <span className="mk-tag">{s.kind}</span>
                  </Link>
                </li>
              ))}
            </ul>
          </section>
        </>
      )}
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
        pipeline will chunk, embed, and index it. Then search and ask over it.
      </p>
      <ol
        className="mx-auto mt-5 grid max-w-md gap-2 text-left font-mono text-sm"
        style={{ color: "var(--mk-text-2)" }}
      >
        <li>1. ingest data (files, paste, logs, IOCs)</li>
        <li>2. the pipeline chunks, embeds, and indexes it</li>
        <li>3. search and ask with cited answers</li>
      </ol>
    </div>
  );
}

"use client";

import Link from "next/link";
import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { api, type AskAnswer, type SearchHit } from "@/lib/api";

type Mode = "hybrid" | "semantic" | "keyword" | "ask";
const MODES: Mode[] = ["hybrid", "semantic", "keyword", "ask"];

export default function SearchPage() {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<Mode>("hybrid");

  const searchMut = useMutation({
    mutationFn: (q: string) => api.search(q, mode, 25),
  });
  const askMut = useMutation({
    mutationFn: (q: string) => api.ask(q, 8),
  });

  const run = (e: React.FormEvent) => {
    e.preventDefault();
    const q = query.trim();
    if (!q) return;
    if (mode === "ask") askMut.mutate(q);
    else searchMut.mutate(q);
  };

  const busy = searchMut.isPending || askMut.isPending;

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">retrieve</span>
        <h1 className="mt-1 text-2xl">search and ask</h1>
      </div>

      <form onSubmit={run} className="space-y-3">
        <input
          className="mk-input font-mono"
          placeholder={mode === "ask" ? "ask a question ..." : "search ..."}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          autoFocus
        />
        <div className="flex items-center gap-2">
          <div className="flex gap-1">
            {MODES.map((m) => (
              <button
                type="button"
                key={m}
                onClick={() => setMode(m)}
                className="rounded px-2 py-1 font-mono text-xs transition-colors"
                style={{
                  color: mode === m ? "var(--mk-on-accent)" : "var(--mk-text-2)",
                  background: mode === m ? "var(--mk-accent)" : "transparent",
                  border: "1px solid var(--mk-border)",
                }}
              >
                {m}
              </button>
            ))}
          </div>
          <button className="mk-btn mk-btn--primary ml-auto" disabled={busy} type="submit">
            {busy ? "..." : mode === "ask" ? "ask" : "search"}
          </button>
        </div>
      </form>

      {mode === "ask" ? (
        <AskResult data={askMut.data} error={askMut.error?.message} />
      ) : (
        <SearchResults hits={searchMut.data?.hits} error={searchMut.error?.message} />
      )}
    </div>
  );
}

function SearchResults({ hits, error }: { hits?: SearchHit[]; error?: string }) {
  if (error) return <ErrorLine text={error} />;
  if (!hits) return null;
  if (hits.length === 0)
    return <p className="text-sm" style={{ color: "var(--mk-text-3)" }}>no matches.</p>;

  return (
    <ul className="space-y-3">
      {hits.map((h) => (
        <li key={h.chunk_id} className="mk-card p-4">
          <div className="mb-1 flex items-center gap-2">
            <Link
              href={`/documents/${h.document_id}?chunk=${h.chunk_id}`}
              className="font-mono text-sm hover:underline"
              style={{ color: "var(--mk-accent)" }}
            >
              {h.title ?? "untitled"}
            </Link>
            <span className="ml-auto flex gap-1">
              {h.semantic && <span className="mk-tag">semantic</span>}
              {h.keyword && <span className="mk-tag">keyword</span>}
            </span>
          </div>
          <p className="text-sm" style={{ color: "var(--mk-text-2)" }}>
            {h.text.slice(0, 280)}
            {h.text.length > 280 ? " ..." : ""}
          </p>
          <div className="mt-2 font-mono text-[11px]" style={{ color: "var(--mk-text-3)" }}>
            score {h.score.toFixed(4)}
            {h.distance != null && ` - distance ${h.distance.toFixed(3)}`}
          </div>
        </li>
      ))}
    </ul>
  );
}

function AskResult({ data, error }: { data?: AskAnswer; error?: string }) {
  if (error) return <ErrorLine text={error} />;
  if (!data) return null;

  return (
    <div className="space-y-4">
      <div className="mk-card p-4">
        <span className="mk-kicker">answer</span>
        <p className="mt-2 whitespace-pre-wrap text-sm" style={{ color: "var(--mk-text-1)" }}>
          {data.answer}
        </p>
        <div className="mt-2 font-mono text-[11px]" style={{ color: "var(--mk-text-3)" }}>
          {data.context_used} context chunks, {data.citations.length} citations
        </div>
      </div>

      {data.citations.length > 0 && (
        <div>
          <span className="mk-kicker">citations</span>
          <ul className="mt-2 space-y-2">
            {data.citations.map((c) => (
              <li key={c.marker} className="mk-card p-3">
                <div className="mb-1 flex items-center gap-2">
                  <span className="mk-tag mk-tag--accent">{c.marker}</span>
                  <Link
                    href={`/documents/${c.document_id}?chunk=${c.chunk_id}`}
                    className="font-mono text-sm hover:underline"
                    style={{ color: "var(--mk-accent)" }}
                  >
                    {c.title ?? "untitled"}
                  </Link>
                </div>
                <p className="text-sm" style={{ color: "var(--mk-text-2)" }}>
                  {c.excerpt}
                </p>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function ErrorLine({ text }: { text: string }) {
  return (
    <p className="text-sm" style={{ color: "var(--mk-danger)" }}>
      {text}
    </p>
  );
}

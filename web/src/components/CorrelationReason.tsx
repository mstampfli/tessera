"use client";

import Link from "next/link";
import type { CorrelationDetail } from "@/lib/api";

const METHOD_LABEL: Record<string, string> = {
  co_occurs: "stated together",
  similar: "contextually similar",
  temporal: "close in time",
};

/** How two entities correlate, grounded in real text: the shared sentence for a
 *  direct link, or each side's own context for a non-obvious one. */
export function CorrelationReason({ detail }: { detail: CorrelationDetail }) {
  return (
    <div className="text-xs">
      <div className="flex flex-wrap gap-1">
        {detail.links.map((l) => (
          <span
            key={l.method}
            className="mk-tag"
            style={l.method === "co_occurs" ? { borderColor: "var(--mk-accent)", color: "var(--mk-accent)" } : undefined}
          >
            {METHOD_LABEL[l.method] ?? l.method} {Math.round(l.strength * 100)}
          </span>
        ))}
      </div>

      {detail.shared_chunks.length > 0 ? (
        <>
          <p className="mk-kicker mt-3">stated together in</p>
          {detail.shared_chunks.map((c) => (
            <Link
              key={c.chunk_id}
              href={`/documents/${c.document_id}?chunk=${c.chunk_id}`}
              className="mk-card mt-2 block p-2"
              style={{ color: "var(--mk-text-2)" }}
            >
              {c.excerpt}
            </Link>
          ))}
        </>
      ) : (
        <>
          <p className="mk-kicker mt-3">
            never stated together; correlated by these similar contexts
          </p>
          {[detail.a_sample, detail.b_sample].filter(Boolean).map(
            (c) =>
              c && (
                <Link
                  key={c.chunk_id}
                  href={`/documents/${c.document_id}?chunk=${c.chunk_id}`}
                  className="mk-card mt-2 block p-2"
                  style={{ color: "var(--mk-text-2)" }}
                >
                  {c.event_time && (
                    <span className="mk-tag mk-tag--success mr-1">{c.event_time.slice(0, 10)}</span>
                  )}
                  {c.excerpt}
                </Link>
              ),
          )}
        </>
      )}
    </div>
  );
}

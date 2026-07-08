"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export function EvidenceDrawer({ insightId, onClose }: { insightId: string; onClose: () => void }) {
  const detail = useQuery({ queryKey: ["insight", insightId], queryFn: () => api.insight(insightId) });

  return (
    <div className="fixed inset-0 z-50 flex justify-end bg-black/40" onClick={onClose}>
      <div
        className="h-full w-full max-w-md overflow-y-auto border-l p-5"
        style={{ background: "var(--mk-surface-1)", borderColor: "var(--mk-border)", boxShadow: "var(--mk-shadow-pop)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <span className="mk-kicker">evidence</span>
          <button className="mk-btn text-xs" onClick={onClose}>
            close
          </button>
        </div>

        {detail.data && (
          <>
            <h3 className="mb-1 text-base">{detail.data.insight.title}</h3>
            <p className="mb-4 text-xs" style={{ color: "var(--mk-text-3)" }}>
              synthesized by {detail.data.insight.model}
            </p>
            <div className="space-y-3">
              {detail.data.evidence.map((e) => (
                <Link
                  key={e.chunk_id}
                  href={`/documents/${e.document_id}?chunk=${e.chunk_id}`}
                  className="mk-card block p-3 transition-colors"
                >
                  <div className="mb-1 flex items-center gap-2">
                    <span className="mk-tag">chunk {e.seq}</span>
                    <span className="truncate font-mono text-xs" style={{ color: "var(--mk-accent)" }}>
                      {e.title ?? "untitled"}
                    </span>
                  </div>
                  <p className="text-sm" style={{ color: "var(--mk-text-2)" }}>
                    {e.excerpt}
                  </p>
                </Link>
              ))}
              {detail.data.evidence.length === 0 && (
                <p className="text-sm" style={{ color: "var(--mk-text-3)" }}>
                  no evidence attached.
                </p>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

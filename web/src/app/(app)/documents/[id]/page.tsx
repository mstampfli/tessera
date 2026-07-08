"use client";

import { use, useEffect, useRef } from "react";
import { useSearchParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export default function DocumentPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = use(params);
  const searchParams = useSearchParams();
  const targetChunk = searchParams.get("chunk");

  const doc = useQuery({ queryKey: ["document", id], queryFn: () => api.document(id) });
  const chunks = useQuery({ queryKey: ["chunks", id], queryFn: () => api.chunks(id) });

  const targetRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (targetChunk && targetRef.current) {
      targetRef.current.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, [targetChunk, chunks.data]);

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">document</span>
        <h1 className="mt-1 text-2xl">{doc.data?.title ?? doc.data?.id ?? "..."}</h1>
        {doc.data && (
          <div className="mt-2 flex flex-wrap gap-2">
            <span className="mk-tag">{doc.data.media_type}</span>
            <span
              className={
                doc.data.status === "ready"
                  ? "mk-tag mk-tag--success"
                  : doc.data.status === "failed"
                    ? "mk-tag mk-tag--danger"
                    : "mk-tag"
              }
            >
              {doc.data.status}
            </span>
            <span className="mk-tag">{chunks.data?.length ?? 0} chunks</span>
          </div>
        )}
        {doc.data?.error && (
          <p className="mt-2 text-sm" style={{ color: "var(--mk-danger)" }}>
            {doc.data.error}
          </p>
        )}
      </div>

      <div className="space-y-3">
        {chunks.data?.map((c) => {
          const isTarget = c.id === targetChunk;
          return (
            <div
              key={c.id}
              ref={isTarget ? targetRef : undefined}
              className={`mk-card p-4 ${isTarget ? "mk-highlight-chunk" : ""}`}
            >
              <div className="mb-1 font-mono text-[11px]" style={{ color: "var(--mk-text-3)" }}>
                chunk {c.seq} - {c.token_count} tokens
              </div>
              <p className="whitespace-pre-wrap text-sm" style={{ color: "var(--mk-text-1)" }}>
                {c.text}
              </p>
            </div>
          );
        })}
      </div>
    </div>
  );
}

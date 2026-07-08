"use client";

import Link from "next/link";
import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export default function ClusterDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = use(params);
  const detail = useQuery({ queryKey: ["cluster", id], queryFn: () => api.cluster(id) });
  const c = detail.data?.cluster;

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">cluster</span>
        <h1 className="mt-1 text-2xl">{c?.label ?? "(unlabeled cluster)"}</h1>
        {c && (
          <p className="mt-1 text-sm" style={{ color: "var(--mk-text-3)" }}>
            {c.size} member chunks
          </p>
        )}
      </div>

      <section>
        <span className="mk-kicker">members</span>
        <ul className="mt-2 space-y-2">
          {detail.data?.members.map((m) => (
            <li key={m.chunk_id} className="mk-card p-3">
              <div className="mb-1 flex items-center gap-2">
                <Link
                  href={`/documents/${m.document_id}?chunk=${m.chunk_id}`}
                  className="font-mono text-sm hover:underline"
                  style={{ color: "var(--mk-accent)" }}
                >
                  {m.title ?? "untitled"}
                </Link>
                <span className="ml-auto font-mono text-[11px]" style={{ color: "var(--mk-text-3)" }}>
                  sim {m.similarity.toFixed(3)}
                </span>
              </div>
              <p className="text-sm" style={{ color: "var(--mk-text-2)" }}>
                {m.excerpt}
              </p>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

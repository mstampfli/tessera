"use client";

import Link from "next/link";
import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

function statusTag(status: string) {
  const cls =
    status === "ready"
      ? "mk-tag mk-tag--success"
      : status === "failed"
        ? "mk-tag mk-tag--danger"
        : "mk-tag";
  return <span className={cls}>{status}</span>;
}

export default function SourceDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = use(params);
  const source = useQuery({ queryKey: ["source", id], queryFn: () => api.source(id) });
  const docs = useQuery({ queryKey: ["source-docs", id], queryFn: () => api.sourceDocuments(id) });

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">source</span>
        <h1 className="mt-1 text-2xl">{source.data?.name ?? "..."}</h1>
      </div>

      <ul className="divide-y" style={{ borderColor: "var(--mk-border)" }}>
        {docs.data?.map((d) => (
          <li key={d.id} className="flex items-center gap-3 py-3">
            <Link
              href={`/documents/${d.id}`}
              className="flex-1 truncate hover:underline"
              style={{ color: "var(--mk-text-1)" }}
            >
              {d.title ?? d.id}
            </Link>
            <span className="mk-tag">{d.media_type.replace(/^\w+\//, "")}</span>
            {statusTag(d.status)}
          </li>
        ))}
      </ul>
      {docs.data && docs.data.length === 0 && (
        <p className="text-sm" style={{ color: "var(--mk-text-3)" }}>no documents in this source.</p>
      )}
    </div>
  );
}

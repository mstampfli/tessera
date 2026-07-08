"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export default function SourcesPage() {
  const sources = useQuery({ queryKey: ["sources"], queryFn: api.sources });

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">sources</span>
        <h1 className="mt-1 text-2xl">where data came from</h1>
      </div>

      {sources.isLoading && (
        <p className="text-sm" style={{ color: "var(--mk-text-3)" }}>loading ...</p>
      )}
      {sources.data && sources.data.length === 0 && (
        <p className="text-sm" style={{ color: "var(--mk-text-3)" }}>
          no sources yet - ingest some data to create one.
        </p>
      )}

      <ul className="divide-y" style={{ borderColor: "var(--mk-border)" }}>
        {sources.data?.map((s) => (
          <li key={s.id} className="py-3">
            <Link href={`/sources/${s.id}`} className="flex items-center justify-between hover:underline">
              <span>{s.name}</span>
              <span className="mk-tag">{s.kind}</span>
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}

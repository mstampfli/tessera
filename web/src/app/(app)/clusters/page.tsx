"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export default function ClustersPage() {
  const clusters = useQuery({ queryKey: ["clusters"], queryFn: api.clusters });

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">clusters</span>
        <h1 className="mt-1 text-2xl">grouped material</h1>
      </div>

      {clusters.data && clusters.data.length === 0 && (
        <p className="text-sm" style={{ color: "var(--mk-text-3)" }}>
          no clusters yet - clusters form as related material is ingested.
        </p>
      )}

      <ul className="divide-y" style={{ borderColor: "var(--mk-border)" }}>
        {clusters.data?.map((c) => (
          <li key={c.id} className="py-3">
            <Link href={`/clusters/${c.id}`} className="flex items-center justify-between hover:underline">
              <span>{c.label ?? "(unlabeled cluster)"}</span>
              <span className="mk-tag">{c.size} chunks</span>
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}

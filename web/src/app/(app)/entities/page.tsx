"use client";

import Link from "next/link";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

const KINDS = ["ip", "domain", "url", "email", "hash_sha256", "cve", "mac", "asn"];

export default function EntitiesPage() {
  const [kind, setKind] = useState<string | undefined>(undefined);
  const [q, setQ] = useState("");

  const entities = useQuery({
    queryKey: ["entities", kind ?? "", q],
    queryFn: () => api.entities(kind, q || undefined),
  });

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">entities</span>
        <h1 className="mt-1 text-2xl">indicators and identifiers</h1>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <input
          className="mk-input max-w-xs font-mono"
          placeholder="filter by value ..."
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <div className="flex flex-wrap gap-1">
          <button
            onClick={() => setKind(undefined)}
            className="rounded px-2 py-1 font-mono text-xs"
            style={{
              color: !kind ? "var(--mk-on-accent)" : "var(--mk-text-2)",
              background: !kind ? "var(--mk-accent)" : "transparent",
              border: "1px solid var(--mk-border)",
            }}
          >
            all
          </button>
          {KINDS.map((k) => (
            <button
              key={k}
              onClick={() => setKind(k)}
              className="rounded px-2 py-1 font-mono text-xs"
              style={{
                color: kind === k ? "var(--mk-on-accent)" : "var(--mk-text-2)",
                background: kind === k ? "var(--mk-accent)" : "transparent",
                border: "1px solid var(--mk-border)",
              }}
            >
              {k.replace("hash_", "")}
            </button>
          ))}
        </div>
      </div>

      {entities.data && entities.data.length === 0 && (
        <p className="text-sm" style={{ color: "var(--mk-text-3)" }}>
          no entities yet - entities appear when ingestion runs.
        </p>
      )}

      <table className="w-full text-sm">
        <thead>
          <tr className="text-left font-mono text-xs" style={{ color: "var(--mk-text-3)" }}>
            <th className="pb-2">kind</th>
            <th className="pb-2">value</th>
            <th className="pb-2 text-right">mentions</th>
          </tr>
        </thead>
        <tbody>
          {entities.data?.map((e) => (
            <tr key={e.id} className="border-t" style={{ borderColor: "var(--mk-border)" }}>
              <td className="py-2">
                <span className="mk-tag">{e.kind.replace("hash_", "")}</span>
              </td>
              <td className="py-2">
                <Link
                  href={`/entities/${e.id}`}
                  className="font-mono hover:underline"
                  style={{ color: "var(--mk-accent)" }}
                >
                  {e.value}
                </Link>
              </td>
              <td className="py-2 text-right font-mono" style={{ color: "var(--mk-text-2)" }}>
                {e.mention_count}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

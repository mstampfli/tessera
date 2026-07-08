"use client";

import { useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";

export default function SettingsPage() {
  const router = useRouter();
  const me = useQuery({ queryKey: ["me"], queryFn: api.me });

  const logout = async () => {
    await api.logout();
    router.push("/login");
  };

  return (
    <div className="space-y-6">
      <div>
        <span className="mk-kicker">settings</span>
        <h1 className="mt-1 text-2xl">account</h1>
      </div>

      <div className="mk-card p-4">
        <dl className="grid grid-cols-[8rem_1fr] gap-y-2 text-sm">
          <dt style={{ color: "var(--mk-text-3)" }}>user</dt>
          <dd>{me.data?.username ?? "..."}</dd>
          <dt style={{ color: "var(--mk-text-3)" }}>principal</dt>
          <dd>{me.data?.principal ?? "..."}</dd>
          <dt style={{ color: "var(--mk-text-3)" }}>scopes</dt>
          <dd className="flex flex-wrap gap-1">
            {me.data?.scopes.map((s) => (
              <span key={s} className="mk-tag">
                {s}
              </span>
            ))}
          </dd>
        </dl>
      </div>

      <div className="mk-card p-4">
        <span className="mk-kicker">api tokens</span>
        <p className="mt-2 text-sm" style={{ color: "var(--mk-text-2)" }}>
          Programs and agents authenticate with API tokens. Create one with the CLI:
        </p>
        <pre
          className="mt-2 overflow-x-auto rounded p-2 font-mono text-xs"
          style={{ background: "var(--mk-surface-inset)", color: "var(--mk-text-2)" }}
        >
          tesserad token new --user {me.data?.username ?? "you"} --name my-agent --scopes read,ingest
        </pre>
      </div>

      <button className="mk-btn" onClick={logout}>
        sign out
      </button>
    </div>
  );
}

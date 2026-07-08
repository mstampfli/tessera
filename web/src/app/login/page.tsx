"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { api, ApiError } from "@/lib/api";

export default function LoginPage() {
  const router = useRouter();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.login(username, password);
      router.push("/");
    } catch (err) {
      setError(err instanceof ApiError ? err.detail : "login failed");
      setBusy(false);
    }
  };

  return (
    <div className="flex min-h-screen items-center justify-center p-4">
      <form onSubmit={submit} className="mk-frame w-full max-w-sm p-6">
        <div className="mb-1 font-mono text-lg font-semibold" style={{ color: "var(--mk-accent)" }}>
          tessera
        </div>
        <p className="mb-5 text-sm" style={{ color: "var(--mk-text-3)" }}>
          knowledge base and correlation engine
        </p>

        <label className="mk-kicker">username</label>
        <input
          className="mk-input mb-3 mt-1"
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          autoComplete="username"
          autoFocus
        />

        <label className="mk-kicker">password</label>
        <input
          className="mk-input mb-4 mt-1"
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          autoComplete="current-password"
        />

        {error && (
          <p className="mb-3 text-sm" style={{ color: "var(--mk-danger)" }}>
            {error}
          </p>
        )}

        <button className="mk-btn mk-btn--primary w-full justify-center" disabled={busy} type="submit">
          {busy ? "signing in ..." : "sign in"}
        </button>
      </form>
    </div>
  );
}

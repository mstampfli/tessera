"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { api } from "@/lib/api";
import { IngestProvider, useIngest } from "./IngestProvider";
import { IngestModal } from "./IngestModal";
import { JobsTray } from "./JobsTray";

const NAV = [
  { href: "/", label: "feed" },
  { href: "/search", label: "search" },
  { href: "/entities", label: "entities" },
  { href: "/sources", label: "sources" },
  { href: "/settings", label: "settings" },
];

function ThemeToggle() {
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  useEffect(() => {
    const stored = (localStorage.getItem("tessera-theme") as "dark" | "light") ?? "dark";
    setTheme(stored);
    document.documentElement.dataset.theme = stored;
  }, []);
  const toggle = () => {
    const next = theme === "dark" ? "light" : "dark";
    setTheme(next);
    document.documentElement.dataset.theme = next;
    localStorage.setItem("tessera-theme", next);
  };
  return (
    <button className="mk-btn text-xs" onClick={toggle} aria-label="Toggle theme">
      {theme === "dark" ? "light" : "dark"}
    </button>
  );
}

function Navbar({ onIngest }: { onIngest: () => void }) {
  const pathname = usePathname();
  const { activeCount } = useIngest();
  return (
    <header
      className="sticky top-0 z-30 border-b"
      style={{ borderColor: "var(--mk-border)", background: "var(--mk-surface-page)" }}
    >
      <div className="mx-auto flex max-w-6xl items-center gap-4 px-4 py-3">
        <Link href="/" className="font-mono text-base font-semibold" style={{ color: "var(--mk-accent)" }}>
          tessera
        </Link>
        <nav className="flex items-center gap-1">
          {NAV.map((item) => {
            const active = item.href === "/" ? pathname === "/" : pathname.startsWith(item.href);
            return (
              <Link
                key={item.href}
                href={item.href}
                className="rounded px-2 py-1 font-mono text-sm transition-colors"
                style={{ color: active ? "var(--mk-accent)" : "var(--mk-text-2)" }}
              >
                {item.label}
              </Link>
            );
          })}
        </nav>
        <div className="ml-auto flex items-center gap-2">
          {activeCount > 0 && <span className="mk-tag mk-tag--accent">{activeCount} running</span>}
          <button className="mk-btn mk-btn--primary text-xs" onClick={onIngest}>
            + ingest
          </button>
          <ThemeToggle />
        </div>
      </div>
    </header>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  const [modalOpen, setModalOpen] = useState(false);
  const session = useQuery({ queryKey: ["me"], queryFn: api.me, retry: false });

  // Global drop: opening the modal on a file drag anywhere makes ingestion feel
  // reachable from any screen.
  useEffect(() => {
    const onDragOver = (e: DragEvent) => {
      if (e.dataTransfer?.types.includes("Files")) {
        e.preventDefault();
        setModalOpen(true);
      }
    };
    window.addEventListener("dragover", onDragOver);
    return () => window.removeEventListener("dragover", onDragOver);
  }, []);

  if (session.isLoading) {
    return <div className="p-8 font-mono text-sm" style={{ color: "var(--mk-text-3)" }}>loading ...</div>;
  }
  if (session.isError) {
    // The fetch wrapper redirects to /login on 401; render nothing meanwhile.
    return null;
  }

  return (
    <div className="min-h-screen">
      <Navbar onIngest={() => setModalOpen(true)} />
      <main className="mx-auto max-w-6xl px-4 py-6">{children}</main>
      <IngestModal open={modalOpen} onClose={() => setModalOpen(false)} />
      <JobsTray />
    </div>
  );
}

export function AppShell({ children }: { children: React.ReactNode }) {
  return (
    <IngestProvider>
      <Shell>{children}</Shell>
    </IngestProvider>
  );
}

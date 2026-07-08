"use client";

import { createContext, useContext, useCallback, useEffect, useRef, useState } from "react";
import { subscribeEvents, type PipelineEvent } from "@/lib/api";

// One tracked ingestion job (per document), updated live from SSE.
export type JobState = {
  documentId: string;
  title: string;
  stage: "queued" | "processing" | "embedding" | "ready" | "failed";
  chunks?: number;
  embedded?: number;
  total?: number;
  error?: string;
};

type IngestContextValue = {
  jobs: JobState[];
  track: (docs: { documentId: string; title: string }[]) => void;
  clear: () => void;
  activeCount: number;
};

const IngestContext = createContext<IngestContextValue | null>(null);

export function useIngest(): IngestContextValue {
  const ctx = useContext(IngestContext);
  if (!ctx) throw new Error("useIngest must be used within IngestProvider");
  return ctx;
}

export function IngestProvider({ children }: { children: React.ReactNode }) {
  const [jobs, setJobs] = useState<JobState[]>([]);
  // Keep a set of tracked document ids so SSE only updates jobs the user started.
  const tracked = useRef<Set<string>>(new Set());

  const applyEvent = useCallback((e: PipelineEvent) => {
    if (!e.document_id || !tracked.current.has(e.document_id)) return;
    setJobs((prev) =>
      prev.map((j) => {
        if (j.documentId !== e.document_id) return j;
        switch (e.type) {
          case "document.processing":
            return { ...j, stage: "processing", chunks: e.chunks };
          case "embed.progress":
            return { ...j, stage: "embedding", embedded: e.embedded, total: e.total };
          case "document.ready":
            return {
              ...j,
              stage: "ready",
              chunks: e.chunks ?? j.chunks,
              embedded: e.embedded ?? j.embedded,
              total: e.total ?? j.total,
            };
          case "document.failed":
            return { ...j, stage: "failed", error: e.error };
          default:
            return j;
        }
      }),
    );
  }, []);

  useEffect(() => {
    const cleanup = subscribeEvents(applyEvent);
    return cleanup;
  }, [applyEvent]);

  const track = useCallback((docs: { documentId: string; title: string }[]) => {
    for (const d of docs) tracked.current.add(d.documentId);
    setJobs((prev) => [
      ...docs.map<JobState>((d) => ({ documentId: d.documentId, title: d.title, stage: "queued" })),
      ...prev,
    ]);
  }, []);

  const clear = useCallback(() => {
    setJobs((prev) => prev.filter((j) => j.stage !== "ready" && j.stage !== "failed"));
  }, []);

  const activeCount = jobs.filter((j) => j.stage !== "ready" && j.stage !== "failed").length;

  return (
    <IngestContext.Provider value={{ jobs, track, clear, activeCount }}>
      {children}
    </IngestContext.Provider>
  );
}

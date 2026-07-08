"use client";

import Link from "next/link";
import { useIngest, type JobState } from "./IngestProvider";

const STAGES = ["queued", "processing", "embedding", "ready"] as const;

function StageLadder({ job }: { job: JobState }) {
  if (job.stage === "failed") {
    return (
      <span className="mk-tag mk-tag--danger" title={job.error ?? ""}>
        failed
      </span>
    );
  }
  const activeIdx = STAGES.indexOf(job.stage as (typeof STAGES)[number]);
  return (
    <div className="flex items-center gap-1 font-mono text-[11px]">
      {STAGES.map((s, i) => {
        const done = i < activeIdx || job.stage === "ready";
        const active = i === activeIdx && job.stage !== "ready";
        const color = done
          ? "var(--mk-success)"
          : active
            ? "var(--mk-accent)"
            : "var(--mk-text-3)";
        const label =
          active && s === "embedding" && job.total
            ? `embedding ${job.embedded ?? 0}/${job.total}`
            : s;
        return (
          <span key={s} style={{ color }}>
            {i > 0 && <span style={{ color: "var(--mk-text-3)" }}> </span>}
            {label}
          </span>
        );
      })}
    </div>
  );
}

function FoundSummary({ job }: { job: JobState }) {
  return (
    <div className="text-xs" style={{ color: "var(--mk-text-2)" }}>
      +{job.chunks ?? 0} chunks, +{job.embedded ?? 0} embedded{" "}
      <Link
        href={`/documents/${job.documentId}`}
        className="underline"
        style={{ color: "var(--mk-accent)" }}
      >
        view
      </Link>
    </div>
  );
}

export function JobsTray() {
  const { jobs, clear } = useIngest();
  if (jobs.length === 0) return null;

  return (
    <div className="fixed bottom-4 right-4 z-40 w-80 max-w-[calc(100vw-2rem)] space-y-2">
      <div className="flex items-center justify-between">
        <span className="mk-kicker">ingestion</span>
        <button className="mk-btn text-xs" onClick={clear}>
          clear done
        </button>
      </div>
      {jobs.slice(0, 6).map((job) => (
        <div key={job.documentId} className="mk-card p-3" style={{ boxShadow: "var(--mk-shadow-pop)" }}>
          <div className="mb-1 truncate text-sm" style={{ color: "var(--mk-text-1)" }}>
            {job.title}
          </div>
          <StageLadder job={job} />
          {job.stage === "ready" && (
            <div className="mt-1">
              <FoundSummary job={job} />
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

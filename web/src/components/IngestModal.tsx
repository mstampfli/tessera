"use client";

import { useCallback, useMemo, useRef, useState } from "react";
import { api, ApiError, type BulkResponse } from "@/lib/api";
import { detect } from "@/lib/detect";
import { useIngest } from "./IngestProvider";

function docsFromBulk(b: BulkResponse): { documentId: string; title: string }[] {
  return b.results
    .filter((r) => typeof r.document_id === "string")
    .map((r) => ({
      documentId: r.document_id as string,
      title: (r.filename as string) ?? "record",
    }));
}

export function IngestModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { track } = useIngest();
  const [paste, setPaste] = useState("");
  const [files, setFiles] = useState<File[]>([]);
  const [dragging, setDragging] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const detections = useMemo(() => detect(paste), [paste]);

  const onDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
    setFiles((prev) => [...prev, ...Array.from(e.dataTransfer.files)]);
  }, []);

  const submit = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const tracked: { documentId: string; title: string }[] = [];

      if (files.length > 0) {
        const res = await api.upload(files);
        tracked.push(...docsFromBulk(res));
      }

      const text = paste.trim();
      if (text) {
        const isNdjson = detections.some((d) => d.label === "ndjson");
        if (isNdjson) {
          const res = await api.ingestBulk(text);
          tracked.push(...docsFromBulk(res));
        } else {
          const firstLine = text.split("\n")[0].slice(0, 60);
          const res = await api.ingest({ content: text, title: firstLine || "pasted", source_name: "pasted" });
          tracked.push({ documentId: res.document_id, title: firstLine || "pasted" });
        }
      }

      if (tracked.length === 0) {
        setError("Add a file or paste some data first.");
        setBusy(false);
        return;
      }
      track(tracked);
      setPaste("");
      setFiles([]);
      onClose();
    } catch (e) {
      setError(e instanceof ApiError ? e.detail : "ingestion failed");
    } finally {
      setBusy(false);
    }
  }, [files, paste, detections, track, onClose]);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/50 p-4 pt-[8vh]"
      onClick={onClose}
    >
      <div
        className="mk-frame w-full max-w-2xl p-5"
        style={{ boxShadow: "var(--mk-shadow-pop)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <span className="mk-kicker">ingest</span>
          <button className="mk-btn" onClick={onClose} aria-label="Close">
            close
          </button>
        </div>

        {/* Drop zone */}
        <div
          onDragOver={(e) => {
            e.preventDefault();
            setDragging(true);
          }}
          onDragLeave={() => setDragging(false)}
          onDrop={onDrop}
          onClick={() => inputRef.current?.click()}
          className="cursor-pointer rounded border-2 border-dashed p-6 text-center transition-colors"
          style={{ borderColor: dragging ? "var(--mk-accent)" : "var(--mk-border)" }}
        >
          <p className="text-sm" style={{ color: "var(--mk-text-2)" }}>
            drop files here, or click to choose
          </p>
          <input
            ref={inputRef}
            type="file"
            multiple
            hidden
            onChange={(e) => setFiles((prev) => [...prev, ...Array.from(e.target.files ?? [])])}
          />
        </div>
        {files.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-2">
            {files.map((f, i) => (
              <span key={i} className="mk-tag">
                {f.name} ({Math.ceil(f.size / 1024)}kb)
              </span>
            ))}
          </div>
        )}

        {/* Paste box */}
        <div className="mt-4">
          <label className="mk-kicker">paste anything</label>
          <textarea
            className="mk-input mt-1 font-mono text-sm"
            rows={6}
            placeholder="logs, IOCs, JSON, CSV, prose ..."
            value={paste}
            onChange={(e) => setPaste(e.target.value)}
          />
          {detections.length > 0 && (
            <p className="mt-1 text-xs" style={{ color: "var(--mk-text-3)" }}>
              detected: {detections.map((d) => `${d.count} ${d.label}`).join(" - ")}
            </p>
          )}
        </div>

        {error && (
          <p className="mt-3 text-sm" style={{ color: "var(--mk-danger)" }}>
            {error}
          </p>
        )}

        <div className="mt-4 flex justify-end gap-2">
          <button className="mk-btn mk-btn--primary" disabled={busy} onClick={submit}>
            {busy ? "ingesting ..." : "ingest"}
          </button>
        </div>
      </div>
    </div>
  );
}

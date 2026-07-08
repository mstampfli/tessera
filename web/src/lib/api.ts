import { z } from "zod";

// All requests go to same-origin /api, which the dev proxy (and Caddy in prod)
// forwards to the Rust core. The session cookie flows automatically; cookie-authed
// mutations carry the CSRF header the core requires.
const BASE = "/api";
const CSRF_HEADER = "X-Tessera-Csrf";
const MUTATING = new Set(["POST", "PUT", "PATCH", "DELETE"]);

export class ApiError extends Error {
  constructor(
    public status: number,
    public detail: string,
  ) {
    super(detail);
  }
}

async function request<T>(path: string, schema: z.ZodType<T>, init?: RequestInit): Promise<T> {
  const method = (init?.method ?? "GET").toUpperCase();
  const headers = new Headers(init?.headers);
  if (MUTATING.has(method)) headers.set(CSRF_HEADER, "1");
  if (init?.body && !headers.has("content-type")) headers.set("content-type", "application/json");

  const res = await fetch(BASE + path, { ...init, headers, credentials: "same-origin" });

  if (res.status === 401) {
    // Centralized redirect on auth failure, except during the login flow itself.
    if (typeof window !== "undefined" && !path.startsWith("/v1/auth")) {
      window.location.href = "/login";
    }
    throw new ApiError(401, "authentication required");
  }

  const text = await res.text();
  const json = text ? JSON.parse(text) : null;
  if (!res.ok) {
    throw new ApiError(res.status, json?.detail ?? res.statusText);
  }
  return schema.parse(json);
}

// ---------------- schemas (runtime validation at the trust boundary) ----------

export const Me = z.object({
  user_id: z.string(),
  username: z.string(),
  principal: z.string(),
  scopes: z.array(z.string()),
});
export type Me = z.infer<typeof Me>;

export const SearchHit = z.object({
  chunk_id: z.string(),
  document_id: z.string(),
  seq: z.number(),
  text: z.string(),
  title: z.string().nullable(),
  score: z.number(),
  semantic: z.boolean(),
  keyword: z.boolean(),
  distance: z.number().nullable(),
});
export type SearchHit = z.infer<typeof SearchHit>;

export const SearchResponse = z.object({
  query: z.string(),
  mode: z.string(),
  hits: z.array(SearchHit),
});

export const Citation = z.object({
  marker: z.string(),
  chunk_id: z.string(),
  document_id: z.string(),
  seq: z.number(),
  title: z.string().nullable(),
  excerpt: z.string(),
});
export type Citation = z.infer<typeof Citation>;

export const AskAnswer = z.object({
  answer: z.string(),
  citations: z.array(Citation),
  context_used: z.number(),
});
export type AskAnswer = z.infer<typeof AskAnswer>;

export const IngestResult = z.object({
  document_id: z.string(),
  deduped: z.boolean(),
  status: z.string(),
});
export type IngestResult = z.infer<typeof IngestResult>;

export const BulkResponse = z.object({
  source_id: z.string(),
  accepted: z.number(),
  deduped: z.number(),
  failed: z.number(),
  results: z.array(z.record(z.string(), z.unknown())),
});
export type BulkResponse = z.infer<typeof BulkResponse>;

export const DocumentView = z.object({
  id: z.string(),
  source_id: z.string(),
  media_type: z.string(),
  size_bytes: z.number(),
  title: z.string().nullable(),
  uri: z.string().nullable(),
  status: z.string(),
  error: z.string().nullable(),
  content_hash: z.string(),
});
export type DocumentView = z.infer<typeof DocumentView>;

export const ChunkView = z.object({
  id: z.string(),
  seq: z.number(),
  text: z.string(),
  token_count: z.number(),
});
export type ChunkView = z.infer<typeof ChunkView>;

export const Source = z.object({
  id: z.string(),
  kind: z.string(),
  name: z.string(),
  created_at: z.string(),
});
export type Source = z.infer<typeof Source>;

export const DocumentSummary = z.object({
  id: z.string(),
  title: z.string().nullable(),
  media_type: z.string(),
  status: z.string(),
  size_bytes: z.number(),
});
export type DocumentSummary = z.infer<typeof DocumentSummary>;

// ---------------- endpoints ---------------------------------------------------

export const api = {
  me: () => request("/v1/auth/me", Me),
  login: (username: string, password: string) =>
    request("/v1/auth/login", z.object({ user_id: z.string(), username: z.string() }), {
      method: "POST",
      body: JSON.stringify({ username, password }),
    }),
  logout: () => request("/v1/auth/logout", z.object({ ok: z.boolean() }), { method: "POST" }),

  search: (q: string, mode: string, limit = 20) =>
    request(
      `/v1/search?q=${encodeURIComponent(q)}&mode=${mode}&limit=${limit}`,
      SearchResponse,
    ),
  ask: (question: string, k = 8) =>
    request("/v1/ask", AskAnswer, { method: "POST", body: JSON.stringify({ question, k }) }),

  ingest: (item: {
    content?: string;
    content_base64?: string;
    media_type?: string;
    title?: string;
    source_name?: string;
  }) => request("/v1/ingest", IngestResult, { method: "POST", body: JSON.stringify(item) }),

  ingestBulk: (ndjson: string) =>
    request("/v1/ingest/bulk", BulkResponse, {
      method: "POST",
      headers: { "content-type": "application/x-ndjson" },
      body: ndjson,
    }),

  upload: (files: File[]) => {
    const form = new FormData();
    files.forEach((f, i) => form.append(`file${i}`, f, f.name));
    return request("/v1/ingest/upload", BulkResponse, { method: "POST", body: form });
  },

  sources: () => request("/v1/sources", z.array(Source)),
  source: (id: string) => request(`/v1/sources/${id}`, Source),
  sourceDocuments: (id: string) =>
    request(`/v1/sources/${id}/documents`, z.array(DocumentSummary)),
  document: (id: string) => request(`/v1/documents/${id}`, DocumentView),
  chunks: (id: string) => request(`/v1/documents/${id}/chunks`, z.array(ChunkView)),
};

// ---------------- live events (SSE) ------------------------------------------

export type PipelineEvent = {
  type: string;
  document_id?: string;
  chunks?: number;
  embedded?: number;
  total?: number;
  error?: string;
};

/** Subscribe to pipeline progress. Returns a cleanup function. */
export function subscribeEvents(onEvent: (e: PipelineEvent) => void): () => void {
  const source = new EventSource(`${BASE}/v1/events`, { withCredentials: true });
  source.onmessage = (msg) => {
    try {
      onEvent(JSON.parse(msg.data) as PipelineEvent);
    } catch {
      // Ignore malformed payloads; events are best-effort progress.
    }
  };
  return () => source.close();
}

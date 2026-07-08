// Client-side "what did I just paste" detection. Advisory only: the server
// re-sniffs authoritatively. This drives the instant echo in the paste box so
// the input visibly understands the user before they submit.

const RE = {
  url: /\bhttps?:\/\/[^\s]+/gi,
  ipv4: /\b(?:\d{1,3}\.){3}\d{1,3}\b/g,
  sha256: /\b[a-f0-9]{64}\b/gi,
  sha1: /\b[a-f0-9]{40}\b/gi,
  md5: /\b[a-f0-9]{32}\b/gi,
  cve: /\bCVE-\d{4}-\d{4,7}\b/gi,
  email: /\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b/gi,
  domain: /\b(?:[a-z0-9-]+\.)+[a-z]{2,}\b/gi,
};

export type Detection = { label: string; count: number };

export function detect(text: string): Detection[] {
  const t = text.trim();
  if (!t) return [];

  // Structured formats first (whole-body shape).
  const first = t[0];
  if (first === "{" || first === "[") {
    const lines = t.split("\n").filter((l) => l.trim());
    const jsonLines = lines.filter((l) => {
      try {
        JSON.parse(l.trim());
        return true;
      } catch {
        return false;
      }
    }).length;
    if (jsonLines >= 2) return [{ label: "ndjson", count: jsonLines }];
    return [{ label: "json", count: 1 }];
  }

  const counts: Detection[] = [];
  const add = (label: string, matches: RegExpMatchArray | null) => {
    if (matches && matches.length) counts.push({ label, count: matches.length });
  };
  add("urls", t.match(RE.url));
  add("cves", t.match(RE.cve));
  add("sha256", t.match(RE.sha256));
  add("emails", t.match(RE.email));
  // IPv4 and domains overlap with other matches; report them but they are hints.
  add("ipv4", t.match(RE.ipv4));

  if (counts.length === 0) {
    const words = t.split(/\s+/).length;
    return [{ label: "text", count: words }];
  }
  return counts;
}

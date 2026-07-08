import type { NextConfig } from "next";

// In development, proxy /api/* to the Rust core so the browser sees one origin
// (the session cookie and EventSource just work, no CORS). In production, Caddy
// does this routing instead and Next never proxies.
const API_ORIGIN = process.env.TESSERA_API_ORIGIN ?? "http://127.0.0.1:8080";

const config: NextConfig = {
  output: "standalone",
  async rewrites() {
    return [{ source: "/api/:path*", destination: `${API_ORIGIN}/:path*` }];
  },
};

export default config;

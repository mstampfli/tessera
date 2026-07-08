import { NextResponse } from "next/server";

// Security headers and a strict Content-Security-Policy. Content and model
// output are rendered as text by React (no raw HTML injection), and everything
// is same-origin, so the policy can be tight.
export function middleware() {
  const isDev = process.env.NODE_ENV !== "production";

  const csp = [
    `default-src 'self'`,
    // Next injects a small inline bootstrap; dev also needs eval for HMR. We keep
    // 'unsafe-inline' for scripts only where Next requires it; no third-party.
    `script-src 'self' 'unsafe-inline'${isDev ? " 'unsafe-eval'" : ""}`,
    `style-src 'self' 'unsafe-inline'`,
    `img-src 'self' blob: data:`,
    `font-src 'self'`,
    `connect-src 'self'`,
    `object-src 'none'`,
    `base-uri 'self'`,
    `form-action 'self'`,
    `frame-ancestors 'none'`,
    isDev ? "" : `upgrade-insecure-requests`,
  ]
    .filter(Boolean)
    .join("; ");

  const response = NextResponse.next();
  response.headers.set("content-security-policy", csp);
  response.headers.set("x-content-type-options", "nosniff");
  response.headers.set("referrer-policy", "no-referrer");
  response.headers.set("x-frame-options", "DENY");
  if (!isDev) {
    response.headers.set(
      "strict-transport-security",
      "max-age=63072000; includeSubDomains; preload",
    );
  }
  return response;
}

export const config = {
  matcher: [
    "/((?!_next/static|_next/image|favicon.ico|.*\\.(?:svg|png|jpg|jpeg|gif|webp|ico)$).*)",
  ],
};

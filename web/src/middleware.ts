import { NextResponse, type NextRequest } from "next/server";

// Name of the server-issued session cookie (see tessera-api auth).
const SESSION_COOKIE = "tessera_session";

// Security headers, a strict Content-Security-Policy, and a server-side auth
// gate. Content and model output are rendered as text by React (no raw HTML
// injection), and everything is same-origin, so the policy can be tight.
export function middleware(request: NextRequest) {
  const isDev = process.env.NODE_ENV !== "production";

  // Auth gate: send a visitor with no session straight to /login before any app
  // shell renders, so an unauthenticated hit on "/" is a redirect, not a blank
  // page. The API still enforces real auth; this is only a UX gate on cookie
  // presence. It cannot loop: /login and /api are always allowed, so a present-
  // but-invalid session falls through to the API's own 401 -> /login redirect.
  const { pathname } = request.nextUrl;
  const isLogin = pathname === "/login";
  const isApi = pathname.startsWith("/api");
  if (!isLogin && !isApi && !request.cookies.has(SESSION_COOKIE)) {
    const url = request.nextUrl.clone();
    url.pathname = "/login";
    url.search = "";
    return NextResponse.redirect(url);
  }

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

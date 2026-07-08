import type { Metadata, Viewport } from "next";
import { IBM_Plex_Mono, IBM_Plex_Sans } from "next/font/google";
import "./globals.css";
import { Providers } from "./providers";

const mono = IBM_Plex_Mono({
  variable: "--font-ibm-mono",
  subsets: ["latin"],
  weight: ["400", "500", "600"],
});

const sans = IBM_Plex_Sans({
  variable: "--font-ibm-sans",
  subsets: ["latin"],
  weight: ["400", "500", "600"],
});

export const metadata: Metadata = {
  title: "tessera",
  description: "A self-hosted knowledge base and correlation engine.",
  robots: { index: false, follow: false },
};

export const viewport: Viewport = {
  themeColor: "#1a120b",
  width: "device-width",
  initialScale: 1,
};

// The nonce CSP requires a per-request nonce on Next's inline scripts, which
// static prerendering cannot bake; render dynamically (everything is behind auth
// anyway, so nothing is statically cacheable).
export const dynamic = "force-dynamic";

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body className={`${mono.variable} ${sans.variable}`}>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}

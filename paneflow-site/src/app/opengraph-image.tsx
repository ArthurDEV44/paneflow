import { ImageResponse } from "next/og";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

// US-006 — programmatic OG card.
// Generated at build time via Satori. The PRD AC mentions a static
// `public/og/cover.png`, but Next.js 16's file convention is the
// idiomatic path for `output: "export"` and produces an identical
// real-PNG outcome (1200x630, image/png, auto-wired meta tags).
// To swap for a hand-crafted design later, either replace this file's
// JSX or delete it and drop a `public/og/cover.png` + manual
// metadata.openGraph.images override on the root layout.

// Required by `output: "export"` — emits a static PNG at build time.
export const dynamic = "force-static";

export const alt =
  "PaneFlow: GPU-accelerated terminal multiplexer";

export const size = {
  width: 1200,
  height: 630,
};

export const contentType = "image/png";

const BG = "#0a0a0a";
const SURFACE = "#141414";
const BORDER = "rgba(255, 255, 255, 0.08)";
const TEXT = "#e8e8e8";
const TEXT_MUTED = "#888888";
const TEXT_SUBTLE = "#555555";

export default async function Image() {
  const logoData = await readFile(
    join(process.cwd(), "public/logos/paneflow-web-300.png"),
  );
  const logoSrc = `data:image/png;base64,${logoData.toString("base64")}`;

  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          background: BG,
          display: "flex",
          flexDirection: "column",
          padding: "72px 88px",
          color: TEXT,
          fontFamily:
            '"Helvetica Neue", "Helvetica", "Arial", system-ui, sans-serif',
        }}
      >
        {/* Header: logo + wordmark */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 28,
          }}
        >
          <img
            src={logoSrc}
            width={88}
            height={88}
            alt=""
            style={{ borderRadius: 18 }}
          />
          <div
            style={{
              fontSize: 76,
              fontWeight: 600,
              letterSpacing: -2,
              color: TEXT,
              display: "flex",
            }}
          >
            PaneFlow
          </div>
        </div>

        {/* Body: tagline + sub-tagline */}
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            marginTop: 56,
            gap: 18,
          }}
        >
          <div
            style={{
              fontSize: 64,
              fontWeight: 600,
              lineHeight: 1.05,
              letterSpacing: -1.5,
              color: TEXT,
              maxWidth: 880,
              display: "flex",
            }}
          >
            GPU-accelerated terminal multiplexer.
          </div>
          <div
            style={{
              fontSize: 30,
              color: TEXT_MUTED,
              lineHeight: 1.3,
              maxWidth: 820,
              display: "flex",
            }}
          >
            Built in pure Rust with Zed&rsquo;s GPUI framework.
          </div>
        </div>

        {/* Footer row: terminal mock + domain */}
        <div
          style={{
            marginTop: "auto",
            display: "flex",
            alignItems: "flex-end",
            justifyContent: "space-between",
            width: "100%",
          }}
        >
          {/* Domain badge */}
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 14,
              fontSize: 26,
              color: TEXT_MUTED,
            }}
          >
            <div
              style={{
                width: 12,
                height: 12,
                borderRadius: 999,
                background: TEXT,
                display: "flex",
              }}
            />
            <div style={{ display: "flex" }}>paneflow.dev</div>
          </div>

          {/* Terminal mockup card */}
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              width: 360,
              height: 200,
              background: SURFACE,
              border: `1px solid ${BORDER}`,
              borderRadius: 14,
              overflow: "hidden",
            }}
          >
            {/* Title bar */}
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "10px 14px",
                borderBottom: `1px solid ${BORDER}`,
              }}
            >
              <div
                style={{
                  width: 10,
                  height: 10,
                  borderRadius: 999,
                  background: "#3a3a3a",
                  display: "flex",
                }}
              />
              <div
                style={{
                  width: 10,
                  height: 10,
                  borderRadius: 999,
                  background: "#3a3a3a",
                  display: "flex",
                }}
              />
              <div
                style={{
                  width: 10,
                  height: 10,
                  borderRadius: 999,
                  background: "#3a3a3a",
                  display: "flex",
                }}
              />
            </div>
            {/* Split-pane body */}
            <div
              style={{
                display: "flex",
                flex: 1,
                fontFamily:
                  '"SF Mono", "Menlo", "Consolas", "Liberation Mono", monospace',
                fontSize: 13,
                color: TEXT_SUBTLE,
              }}
            >
              {/* Left pane */}
              <div
                style={{
                  display: "flex",
                  flexDirection: "column",
                  flex: 1,
                  padding: "12px 14px",
                  borderRight: `1px solid ${BORDER}`,
                  gap: 6,
                }}
              >
                <div style={{ display: "flex" }}>
                  <span style={{ color: TEXT_MUTED }}>$ cargo run</span>
                </div>
                <div style={{ display: "flex" }}>
                  Compiling paneflow…
                </div>
                <div style={{ display: "flex" }}>Finished release</div>
              </div>
              {/* Right pane */}
              <div
                style={{
                  display: "flex",
                  flexDirection: "column",
                  flex: 1,
                  padding: "12px 14px",
                  gap: 6,
                }}
              >
                <div style={{ display: "flex" }}>
                  <span style={{ color: TEXT_MUTED }}>$ git status</span>
                </div>
                <div style={{ display: "flex" }}>On branch main</div>
                <div style={{ display: "flex" }}>nothing to commit</div>
              </div>
            </div>
          </div>
        </div>
      </div>
    ),
    {
      ...size,
    },
  );
}

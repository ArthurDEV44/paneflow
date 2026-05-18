"use client";

import { useEffect, useRef, useState } from "react";
import { Check, Mail, Send } from "lucide-react";
import posthog from "posthog-js";
import { Button } from "@/components/ui/button";

// Cloudflare Turnstile global (loaded via <script> tag in layout).
// Type matches the documented "explicit render" surface only - the
// auto-render API is unused here.
type TurnstileRenderOptions = {
  sitekey: string;
  theme?: "auto" | "light" | "dark";
  size?: "normal" | "flexible" | "compact" | "invisible";
  callback?: (token: string) => void;
  "error-callback"?: () => void;
  "expired-callback"?: () => void;
  "timeout-callback"?: () => void;
};
declare global {
  interface Window {
    turnstile?: {
      render: (
        el: HTMLElement | string,
        options: TurnstileRenderOptions,
      ) => string;
      remove: (widgetId: string) => void;
      reset: (widgetId?: string) => void;
    };
  }
}

type Status = "idle" | "submitting" | "success" | "error";
type Source = "hero" | "download_matrix" | "download_primary";

// Polls until window.turnstile is ready (the Cloudflare loader sets it
// after the async script finishes). Returns a cleanup that aborts the
// poll if the component unmounts before the script loads.
function waitForTurnstile(callback: () => void): () => void {
  if (typeof window === "undefined") return () => {};
  if (window.turnstile) {
    callback();
    return () => {};
  }
  let cancelled = false;
  const id = window.setInterval(() => {
    if (cancelled) return;
    if (window.turnstile) {
      window.clearInterval(id);
      callback();
    }
  }, 100);
  return () => {
    cancelled = true;
    window.clearInterval(id);
  };
}

export function WaitlistForm({
  source,
  platform = "windows",
  onSuccess,
}: {
  source: Source;
  platform?: "windows" | "macos" | "linux";
  onSuccess?: () => void;
}) {
  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<Status>("idle");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [token, setToken] = useState<string | null>(null);
  const turnstileRef = useRef<HTMLDivElement>(null);
  const widgetIdRef = useRef<string | null>(null);

  const siteKey = process.env.NEXT_PUBLIC_TURNSTILE_SITE_KEY ?? "";

  useEffect(() => {
    if (!siteKey || !turnstileRef.current) return;
    const cleanup = waitForTurnstile(() => {
      if (!turnstileRef.current || !window.turnstile) return;
      // size: "invisible" runs Turnstile in pure-API mode - no visible
      // widget, no Cloudflare branding, no testing-keys banner in dev.
      // Token still arrives via the callback. In managed/non-invisible
      // mode, real users on prod see no widget either (Turnstile decides
      // dynamically), but testing keys force a visible widget that
      // pollutes the UI - invisible avoids both problems uniformly.
      widgetIdRef.current = window.turnstile.render(turnstileRef.current, {
        sitekey: siteKey,
        size: "invisible",
        callback: (t) => setToken(t),
        "expired-callback": () => setToken(null),
        "error-callback": () => setToken(null),
      });
    });
    return () => {
      cleanup();
      if (widgetIdRef.current && window.turnstile) {
        window.turnstile.remove(widgetIdRef.current);
        widgetIdRef.current = null;
      }
    };
  }, [siteKey]);

  const submit = async (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (status === "submitting" || status === "success") return;
    if (!token) {
      setErrorMsg("Verifying. Try again in 2s.");
      setStatus("error");
      return;
    }
    setStatus("submitting");
    setErrorMsg(null);
    posthog.capture("windows_waitlist_submitted", { source, platform });

    try {
      const fd = new FormData(e.currentTarget);
      const honeypot = String(fd.get("company") ?? "");
      const res = await fetch("/api/waitlist", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          email,
          turnstileToken: token,
          honeypot,
          source,
        }),
      });
      if (!res.ok) {
        const body = (await res.json().catch(() => ({}))) as {
          error?: string;
        };
        const code = body.error ?? `http_${res.status}`;
        posthog.capture("windows_waitlist_failed", { source, code });
        setStatus("error");
        setErrorMsg(
          code === "rate_limit_exceeded"
            ? "Too many attempts. Wait a minute."
            : code === "validation_error"
              ? "Invalid email."
              : code === "turnstile_failed"
                ? "Verification failed. Reload the page."
                : "Something went wrong. Try again in 30s.",
        );
        if (widgetIdRef.current && window.turnstile) {
          window.turnstile.reset(widgetIdRef.current);
          setToken(null);
        }
        return;
      }
      posthog.capture("windows_waitlist_succeeded", { source, platform });
      setStatus("success");
      onSuccess?.();
    } catch {
      posthog.capture("windows_waitlist_failed", {
        source,
        code: "network_error",
      });
      setStatus("error");
      setErrorMsg("Connection lost. Retry.");
    }
  };

  if (status === "success") {
    return (
      <div className="flex items-center gap-2 text-sm text-text-muted">
        <Check className="w-4 h-4 text-accent-green" />
        <span>
          You&apos;re in. We&apos;ll email you at{" "}
          <strong className="text-text font-semibold">{email}</strong>.
        </span>
      </div>
    );
  }

  return (
    <form onSubmit={submit} className="space-y-2.5">
      <label className="flex items-center gap-2 pl-3 pr-2 py-1 rounded-md border border-surface-border/50 bg-bg focus-within:border-surface-border transition-colors">
        <Mail className="w-3.5 h-3.5 text-text-subtle shrink-0" />
        <input
          type="email"
          name="email"
          required
          autoComplete="email"
          placeholder="ton@email.com"
          value={email}
          onChange={(ev) => setEmail(ev.target.value)}
          disabled={status === "submitting"}
          className="min-w-0 flex-1 bg-transparent text-sm text-text placeholder:text-text-subtle outline-none disabled:opacity-60"
        />
        <Button
          type="submit"
          size="icon-sm"
          loading={status === "submitting"}
          disabled={!email}
          aria-label={status === "submitting" ? "Sending" : "Subscribe"}
          className="ml-1 size-7 rounded-sm before:rounded-[calc(var(--radius-sm)-1px)] [&_svg:not([class*='size-'])]:size-3.5"
        >
          <Send strokeWidth={2} />
        </Button>
      </label>

      {/* Honeypot: invisible to humans, irresistible to dumb bots. */}
      <input
        type="text"
        name="company"
        autoComplete="off"
        tabIndex={-1}
        aria-hidden="true"
        className="hidden"
      />

      {/* Invisible Turnstile mount point. The widget renders nothing
          visible; the token arrives via the JS callback. Hidden so
          accidental CSS leaks (display, padding) can't bring it back. */}
      <div ref={turnstileRef} className="hidden" aria-hidden="true" />

      {errorMsg && (
        <p className="text-xs text-accent-red/90">{errorMsg}</p>
      )}
      <p className="text-[11px] text-text-subtle">
        One email, the day Windows ships. No marketing.
      </p>
    </form>
  );
}

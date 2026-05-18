import { Resend } from "resend";
import { z } from "zod";

// Defaults to Node runtime, which is what the Resend SDK + fetch to
// Cloudflare Turnstile both target. Explicit `runtime = 'nodejs'` is
// not required in Next 16, but kept for surface clarity should we ever
// want to opt into edge later (Resend SDK is fetch-only and edge-safe,
// but Turnstile siteverify accepts both, so the choice is forward-open).

const BodySchema = z.object({
  email: z.string().email().max(254),
  turnstileToken: z.string().min(1).max(2048),
  // Hidden honeypot field. Bots tend to fill every input they parse;
  // humans never see this one because it's display:none. A non-empty
  // value is the strongest single signal of a non-human submission.
  honeypot: z.string().max(0).optional().default(""),
  source: z.enum(["hero", "download_matrix", "download_primary"]),
});

type TurnstileResult = {
  success: boolean;
  "error-codes"?: string[];
  challenge_ts?: string;
  hostname?: string;
};

export async function POST(request: Request) {
  // Cloudflare's `cf-connecting-ip` is the authoritative source when
  // the request transits CF. Vercel sets `x-forwarded-for` (comma-
  // separated list, first = client). Fall back to "anon" so the
  // Turnstile call still works without remoteip in dev.
  const ip =
    request.headers.get("cf-connecting-ip") ||
    request.headers.get("x-forwarded-for")?.split(",")[0]?.trim() ||
    "anon";

  const body = await request.json().catch(() => null);
  const parsed = BodySchema.safeParse(body);
  if (!parsed.success) {
    return Response.json({ error: "invalid_payload" }, { status: 400 });
  }
  const { email, turnstileToken, honeypot, source } = parsed.data;

  // Honeypot hit: succeed silently. Returning 200 makes it harder for
  // a determined attacker to enumerate the gate by diffing responses.
  // We do NOT call Resend on this path - the contact is dropped on
  // the floor, the bot thinks it won.
  if (honeypot.length > 0) {
    return Response.json({ status: "subscribed" }, { status: 200 });
  }

  // Turnstile server-side verification. Required even if the client
  // widget reported success - without this, the entire Turnstile
  // protection is bypassable by anyone replaying a stolen widget DOM.
  const turnstileSecret = process.env.TURNSTILE_SECRET_KEY;
  if (!turnstileSecret) {
    console.error("waitlist_turnstile_secret_missing");
    return Response.json({ error: "server_misconfigured" }, { status: 500 });
  }
  const tsResp = await fetch(
    "https://challenges.cloudflare.com/turnstile/v0/siteverify",
    {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        secret: turnstileSecret,
        response: turnstileToken,
        remoteip: ip,
      }),
    },
  );
  const tsResult = (await tsResp.json()) as TurnstileResult;
  if (!tsResult.success) {
    return Response.json(
      {
        error: "turnstile_failed",
        codes: tsResult["error-codes"] ?? [],
      },
      { status: 403 },
    );
  }

  const apiKey = process.env.RESEND_API_KEY;
  const audienceId = process.env.RESEND_WINDOWS_AUDIENCE_ID;
  if (!apiKey || !audienceId) {
    console.error("waitlist_resend_env_missing");
    return Response.json({ error: "server_misconfigured" }, { status: 500 });
  }

  const resend = new Resend(apiKey);
  // Resend upserts by email since 2025-01-22, so re-submits are a
  // no-op on the contact row. The legacy `audienceId` field still
  // works but is marked @deprecated in the SDK types - the new
  // `segments` field is the forward path (same UUID, aliased server-
  // side during the audiences -> segments migration).
  const { data, error } = await resend.contacts.create({
    email,
    unsubscribed: false,
    segments: [{ id: audienceId }],
  });

  if (error) {
    // Typed error.name covers rate_limit_exceeded, validation_error,
    // quota_exceeded, etc. Surface a generic 502 for upstream issues
    // and let the client retry; log the typed name for debugging.
    console.error("waitlist_resend_failed", {
      code: error.name,
      message: error.message,
      source,
    });
    const status =
      error.name === "rate_limit_exceeded"
        ? 429
        : error.name === "validation_error"
          ? 400
          : 502;
    return Response.json({ error: error.name }, { status });
  }

  return Response.json(
    { status: "subscribed", id: data?.id ?? null },
    { status: 200 },
  );
}

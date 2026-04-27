// US-006 — Twitter card image. Re-uses the opengraph-image generator
// so the X/Twitter Card Validator and the OG unfurlers serve identical
// art. Next.js's file convention auto-emits twitter:image / :width /
// :height / :type meta tags from this module.

// Required by `output: "export"` — must be declared here, not re-exported,
// so Next.js's static config parser can pick it up.
export const dynamic = "force-static";

export {
  default,
  alt,
  size,
  contentType,
} from "./opengraph-image";

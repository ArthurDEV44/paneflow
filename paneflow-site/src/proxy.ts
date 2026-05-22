import createMiddleware from "next-intl/middleware";
import { routing } from "../i18n/routing";

export default createMiddleware(routing);

// Docs are now under `[locale]/docs/...` and must go through the
// next-intl middleware to resolve the active locale before reaching
// the Fumadocs source loader (which keys page trees by locale). The
// previous matcher excluded `docs` from middleware, leaving `/fr/docs`
// uninstrumented and falling through to the `[locale]/[...rest]`
// catch-all 404.
export const config = {
  matcher: "/((?!api|_next|_vercel|.*\\..*).*)",
};

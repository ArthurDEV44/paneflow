import { loader } from "fumadocs-core/source";
import { docs } from "collections/server";

/*
 * Fumadocs source loader. Server-only.
 *
 * EN-only at v1 per the PRD (FR i18n is P3 backlog). Content lives at
 * `content/docs/...` flat; when FR ships, switch to a `[lang]/[[...slug]]`
 * route and re-nest under `content/docs/<locale>/`.
 */
export const source = loader({
  baseUrl: "/docs",
  source: docs.toFumadocsSource(),
});

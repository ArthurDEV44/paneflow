<!-- BEGIN:nextjs-agent-rules -->
# This is NOT the Next.js you know

This version has breaking changes - APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` before writing any code. Heed deprecation notices.
<!-- END:nextjs-agent-rules -->

# Adding a translated docs page

The docs are i18n-aware (`src/lib/i18n-fumadocs.ts` + `src/app/[locale]/docs/`). Every locale renders, but only EN content exists today. To translate one page into one locale:

0. **Read the translation glossary + per-locale system prompt FIRST.** `tasks/translation-glossary.md` is the source of truth for brand names that stay Latin, `paneflow.json` keys that never translate, per-locale register (FR tu, DE du, ES tú, JA ます/です, ZH-Hans declarative), known false-cognate traps (ES `biblioteca` not `librería`, FR `bibliothèque` not `librairie`), and JSX-prop translation rules (`<Card title=`, `description=`). `tasks/translation-prompt.md` has 5 paste-ready Claude system prompts (one per target locale) that already encode those rules - copy the relevant block as the system prompt, paste the EN MDX as the user message, save the output as `<page>.<locale>.mdx`. Sonnet 4.6 for FR/DE/ES, Opus 4.7 for ZH-Hans/JA.

1. **Copy the EN source under the dot-suffix.** Fumadocs uses `parser: "dot"` by default, so `content/docs/<page>.<locale>.mdx` is auto-discovered alongside `content/docs/<page>.mdx`. Example:
   ```bash
   cp content/docs/installation/linux.mdx content/docs/installation/linux.fr.mdx
   ```
2. **Set `lastSyncedFrom` to the EN HEAD.** Add this to the new file's frontmatter (the Zod schema in `source.config.ts` enforces a 7-40 char lowercase hex SHA):
   ```bash
   git log -1 --format=%H -- content/docs/installation/linux.mdx
   # paste the SHA into the new file's frontmatter:
   #   lastSyncedFrom: <sha>
   ```
   The EN source MUST NOT have this field. `scripts/check-docs-stale.ts` reads it to detect drift.
3. **Translate the body.** Also localize the frontmatter `title` and `description`. Keep `dateModified` if present (update to today's ISO date). Native autoglossonym in JSON-LD comes for free from the URL locale.
4. **Verify locally.** Both gates must pass:
   ```bash
   bun run check:docs   # fresh-or-warning for missing SHA; non-zero exit means stale or invalid SHA
   bun run build        # MDX schema validation runs here; malformed lastSyncedFrom fails the build
   ```

After landing the PR, flip the corresponding cell from `·` to `✓` in `tasks/TRANSLATIONS.md`.

## What NOT to translate

Keep these tokens in English even when the rest of the body is translated:

- Code blocks (` ``` `) - they contain commands, paths, and identifiers that must match the user's terminal verbatim.
- File paths and binary names (`paneflow.json`, `~/.config/paneflow/`, `cargo`, `bun`, `git`).
- `paneflow.json` config keys (`default_shell`, `theme`, `window_decorations`, `shortcuts`, `commands`). The schema reads English keys; translated keys silently break the config loader.
- Keybinding strings (`Ctrl+Shift+D`, `Alt+Arrow`) - keep the `Ctrl`/`Alt`/`Shift` modifier names in English to match what the app dispatches.
- Brand names: `Paneflow`, `Claude Code`, `Codex`, `OpenCode`, `Cursor`, `Zed`, `GPUI`. Always Latin script.
- URLs and Markdown link targets.

## Related

- Translation glossary (authoritative): `tasks/translation-glossary.md`.
- Per-locale system prompts (paste-ready for Claude): `tasks/translation-prompt.md`.
- Schema: `source.config.ts` `pageSchema.lastSyncedFrom` (added by `prd-fumadocs-docs-i18n.md` US-007).
- Freshness CI: `scripts/check-docs-stale.ts` (US-008; run via `bun run check:docs`).
- Coverage tracker: `tasks/TRANSLATIONS.md` (US-009).
- Infra PRD: `tasks/prd-fumadocs-docs-i18n.md`.
- Content PRD: `tasks/prd-docs-i18n-content.md`.

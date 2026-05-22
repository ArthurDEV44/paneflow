# Paneflow — Simplified Chinese Translation Style Guide

**Version:** 1.0
**Author:** Claude (AI agent), to be validated via fresh-context self-critique pass (US-010)
**Source PRD:** `tasks/prd-i18n-fr-zh-Hans.md` section 6.2
**Memory refs:** `feedback_brand_paneflow.md`
**Scope:** all translations into Simplified Chinese for `messages/zh-Hans.json` (US-009), and any future zh-Hans marketing copy on `paneflow.dev`.

This document is normative. US-009 must follow it strictly. US-010 (QA) uses it as the grading rubric.

---

## 1. Register (语体)

**Formal professional tech (正式技术风格).** Concise. Matches the register of zh-Hans marketing copy from Linear, Vercel, Cursor, and other tier-1 dev tools serving Chinese developers.

Key characteristics:

- **Direct, no hype.** Avoid "革命性的" (revolutionary), "完美的" (perfect), "终极" (ultimate) superlatives unless the EN source uses an equivalent strong claim.
- **Sentence-final particles minimal.** Use 了/吗/呢 only when grammatically required, not as conversational softeners.
- **No first-person plural marketing voice.** Avoid "我们" (we) constructions — prefer impersonal or product-as-subject sentences. "Paneflow 让您..." is acceptable; "我们让您..." sounds like sales copy.
- **No second-person address inflation.** "您" is the formal you-form; use it sparingly when the EN source uses "you" in a direct user-instruction context. For descriptive sentences ("Paneflow runs agents in parallel"), no "您" is needed.

---

## 2. Brand

**"Paneflow" in Latin script, always.** Never transliterate (no 派恩弗洛, 派恩弗罗, 板流, or any phonetic adaptation).

The brand stays as the literal Latin token inside JSON values. The catalogue from US-005 already preserves "Paneflow" inline; US-009 keeps it intact.

Correct: `使用 Paneflow 并行运行您的智能体。`
Incorrect: `使用派恩弗洛...`, `使用 paneflow...` (lowercase), `使用 PaneFlow...` (camelCase).

Note the half-width space on both sides of `Paneflow` — see section 4 (Pangu spacing).

---

## 3. Glossary (术语表)

All terms below are **canonical**. Variant translations are forbidden (US-010 verifies automatically).

| English (source) | zh-Hans (canonical) |
|---|---|
| Terminal multiplexer | 终端复用器 |
| Terminal | 终端 |
| AI agents | AI 智能体 |
| Agent | 智能体 |
| Panes | 面板 |
| Pane (singular) | 面板 |
| Splits | 分屏 |
| Split (singular) | 分屏 |
| Workspaces | 工作区 |
| Workspace (singular) | 工作区 |
| Tabs | 标签页 |
| Tab (singular) | 标签页 |
| Drop-in replacement | 无缝替代 |
| Open source | 开源 |
| Self-hosted | 自托管 |
| Built with Rust | 使用 Rust 构建 |
| Free and open source | 免费开源 |
| Get started | 开始使用 |
| Download | 下载 |
| Compare | 对比 |
| Documentation | 文档 |
| Docs (short) | 文档 |
| Coming soon | 即将推出 |
| Roadmap | 路线图 |
| Release notes | 版本说明 |
| Waitlist | 候补名单 |
| Privacy policy | 隐私政策 |
| Terms of service | 服务条款 |
| GPU-accelerated | GPU 加速 |
| Cross-platform | 跨平台 |
| Native | 原生 |
| Lightweight | 轻量 |
| Branch-aware | 分支感知 |
| Session restore | 会话恢复 |
| Dev server | 开发服务器 |
| Coding agent | 编码智能体 |
| CLI agent | CLI 智能体 |
| Parallel | 并行 |
| Keyboard navigation | 键盘导航 |
| Founder | 创始人 |
| Contact | 联系方式 |
| Issue (GitHub) | issue (keep Latin) |
| Pull request | PR (keep Latin) |
| Branch | 分支 |
| Commit | 提交 |

**Term-choice rationale (for the cross-check in US-010):**

- `终端复用器` for `terminal multiplexer`: standard Chinese tech term used by tmux/screen documentation translations and Linux community blogs. Alternatives like `终端多路复用器` are too literal/verbose.
- `面板` for `panes`: used by Warp's zh-Hans copy and standard window-manager terminology. Avoid `窗格` which is a Windows-specific UI term.
- `智能体` for `agents`: contemporary tech standard for AI/LLM agents. Avoid `代理` which conflates with network-proxy meaning.
- `工作区` for `workspaces`: used by Cursor, VS Code zh-Hans, and most modern IDE translations.
- `开源` for `open source`: established Chinese tech term, never expand to `开放源代码` in marketing copy (too formal/legal).
- `候补名单` for `waitlist`: clearer than literal `等待列表`; matches the user-facing intent.

---

## 4. Pangu spacing rule (盘古之白)

**Insert a half-width space between Chinese characters and adjacent Latin letters, digits, or Latin punctuation.**

Apply when:
- Chinese character adjacent to a Latin letter: `使用 Rust 构建` (not `使用Rust构建`).
- Chinese character adjacent to a digit: `3 个工作区` (not `3个工作区`).
- Chinese character adjacent to Latin punctuation that bridges Latin content: `运行 Claude Code, Codex 和 OpenCode` (the comma after `Code` is ASCII because it lives between two Latin tokens; but `运行 Claude Code，然后` uses a fullwidth comma `，` because the next clause is Chinese).

**Do NOT apply** when:
- Adjacent to CJK fullwidth punctuation: `面板，分屏，标签页。` (fullwidth `，` and `。`).
- Inside a pure-Latin token: `Paneflow GPU` is one Latin sequence, no space inside (just `Paneflow GPU`, never `Paneflow G P U`).

Verification: US-010's automated check flags any `[一-鿿][a-zA-Z0-9]` or `[a-zA-Z0-9][一-鿿]` adjacency in JSON values that lacks a half-width space.

---

## 5. Punctuation (标点符号)

### 5.1 Fullwidth in pure-Chinese prose

Use CJK fullwidth punctuation when both sides of the punctuation are Chinese characters:

- `，` (fullwidth comma) — not ASCII `,`
- `。` (fullwidth period) — not ASCII `.`
- `；` (fullwidth semicolon) — not ASCII `;`
- `：` (fullwidth colon) — not ASCII `:`
- `？` (fullwidth question mark) — not ASCII `?`
- `！` (fullwidth exclamation) — not ASCII `!`
- `「」` (Chinese single quotes) or `"" ''` (curly quotes) — context-dependent; prefer `""` for casual marketing copy
- `（）` (fullwidth parentheses) — not ASCII `()` when wrapping Chinese content
- `——` (Chinese em-dash, two characters wide) — but project-wide ASCII-hyphen rule applies: use `-` instead per `feedback_simple_hyphens.md`

### 5.2 ASCII around Latin tokens

When punctuation sits between Latin tokens, use ASCII:

- `Paneflow, Cursor, and VS Code` → `Paneflow、Cursor 和 VS Code` (here `、` is the Chinese enumeration mark, used for listing items in Chinese)
- BUT inside an English fragment kept verbatim: `the "best terminal" award` stays ASCII `"…"`.

### 5.3 Hyphen rule (project-wide)

**ASCII `-` only, never `—` or `–` or `——`.** Per memory `feedback_simple_hyphens.md`. This overrides standard Chinese typography conventions for this project.

Example:
- ✅ `Paneflow - 并行运行您的智能体`
- ❌ `Paneflow —— 并行运行您的智能体`

### 5.4 Numerals

**Use Arabic numerals** for version numbers, dates, counts, prices: `16.2.4`, `2026 年`, `3 个工作区`, `$20/月`. Do NOT convert to Chinese numerals (`一二三` / `叁` / etc.) — too formal/archaic for tech marketing.

Note Pangu spacing: `2026 年` (digit + Chinese), `3 个工作区` (digit + Chinese measure word).

---

## 6. Banned constructions

### 6.1 Literal idiom translations

Avoid word-for-word translation of English idioms; use the established Chinese equivalent.

- ❌ "Out of the box" → `在盒子外面`
- ✅ "Out of the box" → `开箱即用`

- ❌ "Plug and play" → `插入并播放`
- ✅ "Plug and play" → `即插即用`

- ❌ "First-class citizen" → `头等公民`
- ✅ "First-class citizen" → `一等公民` or rephrase: `原生支持`

### 6.2 Overuse of `的` particles

Aim for cleaner noun-phrase constructions. Multiple consecutive `的` is a marker of literal English translation.

- ❌ `用于运行您的多个智能体的工作区的功能` (4 × `的`)
- ✅ `用于运行多个智能体的工作区` or `多智能体工作区` (1 × `的`)

Rule of thumb: at most 1-2 `的` per sentence; if a sentence has 3+, restructure.

### 6.3 Transliteration when a standard term exists

- ❌ `终端` → `特米诺尔` (phonetic transliteration of "terminal")
- ✅ `终端`

- ❌ `文档` → `多克` (phonetic transliteration of "doc/docs")
- ✅ `文档`

### 6.4 Western branding overlays

- ❌ `欢迎来到 Paneflow` (literal translation of "Welcome to Paneflow")
- ✅ Direct value statement: `Paneflow，并行编码智能体的终端工作区。`

### 6.5 `请` overuse

`请` (please) is appropriate in formal instructions but should not appear in every CTA or sentence.

- ❌ `请点击此处下载 Paneflow`
- ✅ `下载 Paneflow` (CTA button) or `点击此处下载 Paneflow` (link description)

### 6.6 Marketing-cliché expansions

- ❌ `打造卓越的开发体验` (literal corporate boilerplate)
- ✅ Be specific: `让编码智能体并行运行` or whatever the EN source actually says

### 6.7 Mixed-script unnecessary

When a clean Chinese term exists, do not leave the English word:

- ❌ `运行 parallel 的智能体` (mid-sentence English word)
- ✅ `并行运行智能体`

Exceptions: established Latin-script tech tokens — `Rust`, `GPU`, `CLI`, `JSON`, `MIT`, `AGPL`, product names (`Claude Code`, `Codex`, `OpenCode`), `Linux`, `macOS`, `Windows`, `GitHub`, `Vercel`.

---

## 7. Examples EN → zh-Hans

### Example 1 — Hero headline

**Source EN:**
> A terminal workspace for orchestrating Claude Code, Codex, OpenCode, and custom CLI agents.

**zh-Hans canonical:**
> 用于编排 Claude Code、Codex、OpenCode 和自定义 CLI 智能体的终端工作区。

Notes:
- `terminal workspace` → `终端工作区` (glossary).
- `orchestrating` → `编排` (standard tech term, not `指挥` which sounds musical).
- `custom CLI agents` → `自定义 CLI 智能体` (Pangu space around `CLI`).
- Enumeration mark `、` between Latin product names; fullwidth `和` (the word "and") before the last item.
- Pangu spacing around `Claude Code`, `Codex`, `OpenCode`, `CLI`.
- Fullwidth period `。` (Chinese sentence terminator).

### Example 2 — CTA button

**Source EN:**
> Download Paneflow

**zh-Hans canonical:**
> 下载 Paneflow

Notes:
- `Download` → `下载` (glossary).
- Brand unchanged with Pangu space.
- No fullwidth period needed for short CTA labels.

### Example 3 — Sentence with interpolation

**Source EN (JSON value):**
> You're in. We'll email you at `<strong>{email}</strong>`.

**zh-Hans canonical (JSON value):**
> 已加入。我们将发送邮件至 `<strong>{email}</strong>`。

Notes:
- `You're in` → `已加入` (concise, idiomatic). Alternative: `加入成功`.
- `We'll email you at X` → `我们将发送邮件至 X` (factual, no marketing inflation).
- Placeholder `<strong>{email}</strong>` preserved verbatim; Pangu space before `<strong>`.
- Fullwidth period `。` after each sentence.

### Example 4 — Rich text with multiple placeholders

**Source EN (JSON value):**
> Built with `<strong>Rust</strong>`, runs on Linux, macOS, and Windows.

**zh-Hans canonical (JSON value):**
> 使用 `<strong>Rust</strong>` 构建，可在 Linux、macOS 和 Windows 上运行。

Notes:
- `Built with Rust` → `使用 Rust 构建` (glossary).
- Placeholder `<strong>Rust</strong>` preserved; Pangu spaces around it.
- Operating system names kept in Latin: `Linux`、`macOS`、`Windows` (established Latin tokens, enumeration mark `、`).
- `runs on X` → `可在 X 上运行` (idiomatic, not literal `运行在...`).
- Fullwidth comma `，` between the two clauses; fullwidth period at end.

### Example 5 — Section heading

**Source EN:**
> Compare Paneflow vs other terminal workspaces

**zh-Hans canonical:**
> 对比 Paneflow 与其他终端工作区

Notes:
- `Compare X vs Y` → `对比 X 与 Y` (`与` is the formal "with/and" for comparative constructions, more natural than `vs`).
- No fullwidth period (it's a heading, not a sentence).
- Pangu space around `Paneflow`.

### Example 6 — Benefits list

**Source EN:**
> - One pane per agent. Resize, navigate, focus from the keyboard.
> - One workspace per task. Restore everything after a restart.
> - Branch-aware sessions. Switch context without losing state.

**zh-Hans canonical:**
> - 每个智能体一个面板。通过键盘调整大小、导航和聚焦。
> - 每个任务一个工作区。重启后恢复所有内容。
> - 分支感知会话。切换上下文，不丢失状态。

Notes:
- Glossary terms applied: `面板`, `工作区`, `分支感知`, `会话`.
- `Resize, navigate, focus` → `调整大小、导航和聚焦` (enumeration mark `、` between verbs, `和` before final).
- `from the keyboard` → `通过键盘` (idiomatic, placed at sentence start for emphasis).
- No `您` here — sentences are descriptive of product behavior, not direct user address.
- Pangu spacing not needed inside pure-Chinese phrases.

### Example 7 — Pricing/legal text

**Source EN:**
> Free and open source: install in 30 seconds.

**zh-Hans canonical:**
> 免费开源：30 秒完成安装。

Notes:
- `Free and open source` → `免费开源` (glossary).
- Fullwidth colon `：` (both sides Chinese-context).
- Digit + measure word: `30 秒` with Pangu space.
- `install in 30 seconds` → `30 秒完成安装` (Chinese prefers result-first ordering for marketing claims).

### Example 8 — Privacy/legal phrase

**Source EN:**
> By signing up, you accept our terms of service and privacy policy.

**zh-Hans canonical:**
> 注册即表示您接受我们的服务条款和隐私政策。

Notes:
- `you` → `您` (formal you-form, appropriate for legal context).
- `terms of service` → `服务条款`, `privacy policy` → `隐私政策` (glossary).
- `By X-ing, you Y` → `X 即表示您 Y` (standard Chinese legal phrasing).
- `our` → `我们的` (only place where `我们` is acceptable, since the contract is between the user and the company).
- Fullwidth comma `，` and period `。`.

### Example 9 — Footnote / fine print

**Source EN:**
> Paneflow stores no telemetry by default. Opt-in only.

**zh-Hans canonical:**
> Paneflow 默认不收集遥测数据。仅在选择加入后启用。

Notes:
- Pangu space around `Paneflow`.
- `Opt-in only` → `仅在选择加入后启用` (idiomatic; `opt-in` has no single Chinese equivalent, expand as `选择加入`).
- Two short factual sentences, no marketing softening.

---

## 8. Self-critique pass (process for US-009 / US-010)

Because the team has no in-house native zh-Hans reviewer, US-009 ships AI-authored copy and US-010 enforces a **fresh-context self-critique pass**:

1. **First pass (US-009):** translate `messages/en.json` → `messages/zh-Hans.json` following this guide and the glossary.
2. **Self-critique (US-010, fresh session):** start a NEW conversation (no prior synthesis bias). Read this style guide first. Then read `messages/zh-Hans.json` and grade every entry against:
   - Glossary fidelity (canonical term used? no variants?)
   - Pangu spacing applied where required
   - Fullwidth punctuation correct (no stray ASCII `,` `.` `:` inside pure-Chinese prose)
   - Brand "Paneflow" Latin-only (no transliteration)
   - No banned constructions (literal idioms, `的` overuse, transliteration, marketing clichés, mid-sentence English)
   - Tone matches the source (no inflation, no softening)
3. **Fixes applied** to `messages/zh-Hans.json` and committed.
4. **Automated check** via `scripts/check-translations.ts`:
   - All keys present in zh-Hans.json
   - No empty strings, no English-only values (banned-token grep: detects English sentences in JSON values that should have been translated)
   - Pangu spacing check: regex `[一-鿿][a-zA-Z0-9]|[a-zA-Z0-9][一-鿿]` should find ZERO matches in JSON values without an intervening U+0020 space
   - CJK character count > 0 in the file (sanity check: no all-English placeholder leaked)
   - JSON validates as well-formed UTF-8

If any check fails, the merge is blocked.

---

## 9. References

- Source PRD: `tasks/prd-i18n-fr-zh-Hans.md`
- Source catalogue: `messages/en.json`
- Memory: `feedback_brand_paneflow.md`, `feedback_simple_hyphens.md`
- QA script: `scripts/check-translations.ts` (shipped in US-010)
- Comparable zh-Hans dev marketing copy for register calibration: cursor.com (zh-Hans), linear.app (zh-Hans), vercel.com (zh-Hans, internal Chinese landing).

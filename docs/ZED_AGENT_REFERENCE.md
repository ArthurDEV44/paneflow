# Zed Agent — Référence technique complète

> Document de référence pour la refonte de l'interface Agent de Paneflow.
> Synthèse de l'exploration de `~/dev/zed` (crates `agent`, `acp_thread`, `agent_servers`,
> `agent_settings`, `agent_skills`, `agent_ui`, `prompt_store`, `context_server`,
> `language_model`, `language_models`, `markdown`).
> Source : Zed @ `main` (2026-05-23). Toutes les références sont au format `crate/fichier.rs:ligne`.

---

## Table des matières

1. [Vue d'ensemble & vision produit](#1-vue-densemble--vision-produit)
2. [Carte des crates](#2-carte-des-crates)
3. [Agent Client Protocol (ACP)](#3-agent-client-protocol-acp)
4. [Modèle de domaine](#4-modèle-de-domaine)
5. [Cycle de vie d'un turn (data flow)](#5-cycle-de-vie-dun-turn-data-flow)
6. [Persistance](#6-persistance)
7. [Réglages utilisateur](#7-réglages-utilisateur)
8. [Anatomie du panneau Agent](#8-anatomie-du-panneau-agent)
9. [Toolbar & menus](#9-toolbar--menus)
10. [Composer (input)](#10-composer-input)
11. [Sélecteurs : modèle / mode / profil](#11-sélecteurs--modèle--mode--profil)
12. [Vue conversation : rendering détaillé](#12-vue-conversation--rendering-détaillé)
13. [Tool calls & permissions](#13-tool-calls--permissions)
14. [Mentions, slash commands, attachments](#14-mentions-slash-commands-attachments)
15. [Système d'outils & MCP](#15-système-doutils--mcp)
16. [Profils, modes, providers LLM](#16-profils-modes-providers-llm)
17. [Skills](#17-skills)
18. [Inline Assistant (buffer & terminal)](#18-inline-assistant-buffer--terminal)
19. [Historique & archive](#19-historique--archive)
20. [Drafts & connection store](#20-drafts--connection-store)
21. [Keybindings & actions](#21-keybindings--actions)
22. [Streaming, scroll, follow](#22-streaming-scroll-follow)
23. [Erreurs, annulation, notifications](#23-erreurs-annulation-notifications)
24. [Système visuel : tokens, espacements, icônes](#24-système-visuel--tokens-espacements-icônes)
25. [Patterns UI à voler pour Paneflow](#25-patterns-ui-à-voler-pour-paneflow)
26. [Recommandations concrètes pour Paneflow](#26-recommandations-concrètes-pour-paneflow)

---

## 1. Vue d'ensemble & vision produit

Zed traite l'agent comme un **premier citoyen de l'IDE** : c'est un panel latéral persistant, dockable gauche/droite uniquement (pas en bas), avec un système de threads sauvegardés en SQLite et reprises de session. Le moteur supporte **deux types d'agents** sous une même interface :

- **Native agent** (`NativeAgent`) — moteur intégré qui parle directement aux providers LLM (Anthropic, OpenAI, Ollama, Gemini, etc.) et exécute les outils en process.
- **External ACP agents** (Claude Code, Codex, Gemini CLI, n'importe quel binaire qui parle ACP en JSON-RPC sur stdio) — sous-processus encapsulés derrière la même abstraction.

Cette abstraction passe par le **trait `AgentConnection`** dans `acp_thread`, et l'UI ne sait pas — ne doit pas savoir — quel type d'agent est derrière. C'est le levier d'extensibilité fondamental.

**Couches** (de bas en haut) :

```
provider LLM (anthropic, openai, …)         ← réseau
        ↑
language_model (trait LanguageModel)        ← abstraction provider
        ↑
agent::Thread (turn loop, tools)            ← moteur natif
        ↑
acp_thread::AcpThread (display model)       ← état de rendu partagé
        ↑       ↑
   NativeAgentConnection   AcpConnection (subprocess ACP)
        ↑
agent_ui::AgentPanel + ConversationView     ← UI GPUI
```

---

## 2. Carte des crates

| Crate | Rôle | Types-clés exportés |
|---|---|---|
| **`agent`** | Moteur natif. Boucle `Thread` (state machine de conversation), `NativeAgent`/`NativeAgentConnection`, `ThreadStore` (gateway SQLite), 23 outils built-in, rendu du system prompt, schéma DB. | `Thread`, `ThreadStore`, `DbThread`, `Message`, `UserMessage`, `AgentMessage`, `NativeAgentServer`, `ThreadEvent` |
| **`acp_thread`** | Couche d'affichage partagée. Représentation streamée d'un thread vue par l'UI. Définit l'abstraction `AgentConnection`. | `AcpThread`, `AgentConnection`, `AgentServer`, `AgentThreadEntry`, `UserMessageId`, `ToolCall`, `AssistantMessage` |
| **`agent_servers`** | Intégrations avec agents externes via ACP/JSON-RPC sur stdio. Gère cycle de vie subprocess, framing, auth, session create/load/resume. | `AcpConnection`, `AgentServer`, `AgentServerDelegate` |
| **`agent_settings`** | Schéma de tous les réglages utilisateur agent (modèle, profils, permissions outils, UI). | `AgentSettings`, `AgentProfileSettings`, `LanguageModelSelection`, `ToolPermissions` |
| **`agent_skills`** | Découverte/chargement des skills (`SKILL.md` avec frontmatter YAML). Crate feuille, pas de dépendance sur `agent`. | `Skill`, `SkillSource`, `SkillIndex`, `load_skills_from_directory` |
| **`agent_ui`** | UI GPUI. `AgentPanel`, `ConversationView`, `ThreadView`, `MessageEditor`, tous les selectors, archive view, inline assistant. ~58 k lignes. | `AgentPanel`, `ConversationView`, `ThreadView`, `MessageEditor`, `ProfileSelector`, `ModelSelector`, `ModeSelector` |
| **`prompt_store`** | Stockage des prompts utilisateur (LMDB sous `~/.local/share/zed/prompts`), `ProjectContext` (worktrees, rules, skills, shell info) qui alimente le template Handlebars du system prompt. | `PromptStore`, `ProjectContext`, `WorktreeContext`, `UserRulesContext` |
| **`context_server`** | Implémentation MCP (Model Context Protocol). Transports stdio / HTTP / custom. | `ContextServer`, `ContextServerTransport` |
| **`language_model`** | Trait `LanguageModel` et `LanguageModelProvider`, structures de requête/réponse, gestion des tool_use events. | `LanguageModel`, `LanguageModelProvider`, `LanguageModelRequest`, `LanguageModelCompletionEvent` |
| **`language_models`** | Implémentations concrètes des providers (Anthropic, OpenAI, Ollama, Bedrock, etc.). | `register_language_model_providers()` |
| **`markdown`** | Rendu markdown via `pulldown_cmark` avec syntax highlighting tree-sitter, polices configurables. | `Markdown`, `MarkdownElement`, `MarkdownFont::Agent` |

**Direction des dépendances** :

```
agent_ui → agent → acp_thread → agent_client_protocol (schémas JSON)
   ↓        ↓
   ↓        prompt_store
   ↓        agent_settings
   ↓        agent_skills
   ↓        language_model → language_models
   ↓
agent_servers → acp_thread
context_server (autonome, consommé par agent)
```

`agent_skills` et `prompt_store` sont des crates feuilles. `acp_thread` ne dépend pas de `agent` — c'est `agent` qui dépend de `acp_thread` pour `UserMessageId`, `AcpThread`, et le trait `AgentConnection`.

---

## 3. Agent Client Protocol (ACP)

**ACP** est un protocole JSON-RPC sur stdio qui permet à Zed de parler à des agents externes (Claude Code, Codex, etc.) avec la même interface que son agent natif. Les types Rust typés vivent dans le crate `agent_client_protocol` (`acp::PromptRequest`, `acp::ToolCall`, `acp::SessionId`, etc.).

**Trait central — `AgentConnection`** (`acp_thread/src/connection.rs:47`) :

```rust
pub trait AgentConnection {
    fn agent_id(&self) -> AgentId;
    fn new_session(self: Rc<Self>, project, work_dirs, cx) -> Task<Result<Entity<AcpThread>>>;
    fn load_session(self: Rc<Self>, session_id, project, work_dirs, title, cx) -> Task<Result<Entity<AcpThread>>>;
    fn resume_session(self: Rc<Self>, session_id, project, work_dirs, title, cx) -> Task<Result<Entity<AcpThread>>>;
    fn prompt(&self, user_message_id, params: acp::PromptRequest, cx) -> Task<Result<acp::PromptResponse>>;
    fn authenticate(&self, method, cx) -> Task<Result<()>>;
    fn cancel(&self, session_id, cx);
    // + auth_methods, supports_load_session, supports_resume_session…
}
```

**Deux implémentations** :

1. **`NativeAgentConnection`** (`agent/src/agent.rs:1748`) — enveloppe une `Entity<NativeAgent>`. Son `prompt()` (ligne 2206) appelle `thread.send(id, content, cx)` qui drive la stack LM Zed directement. Aucun subprocess.

2. **`AcpConnection`** (`agent_servers/src/acp.rs:1453`) — enveloppe un canal JSON-RPC vers un subprocess. Son `prompt()` (ligne 1864) envoie `acp::PromptRequest` sur le fil via `connection.send_request(params)`. C'est par là que passent Claude Code, Gemini CLI, Codex, et tout serveur ACP custom. Tool calls, requests d'autorisation, session modes, options de config : tout passe par le même canal RPC.

Au-dessus, le trait **`AgentServer`** (`agent_servers/src/agent_servers.rs:47`) est une factory : sa méthode `connect()` retourne `Task<Result<Rc<dyn AgentConnection>>>`. `NativeAgentServer` produit une `NativeAgentConnection` ; les serveurs externes produisent une `AcpConnection` après avoir lancé leur subprocess.

**Pourquoi c'est important pour Paneflow** : si tu veux permettre plusieurs backends d'agents (le tien, Claude Code, OpenAI Assistants, etc.) sous la même UI, factorise comme ça dès le début. Sinon tu vas avoir des branches partout.

---

## 4. Modèle de domaine

### Côté moteur (`agent::Thread`)

`Thread` est une state machine async qui détient la conversation, le modèle, le profil et le turn en cours. Champs-clés (`agent/src/thread.rs:957`) :

| Champ | Type | Rôle |
|---|---|---|
| `id` | `acp::SessionId` | UUID de session |
| `prompt_id` | `PromptId` | UUID rafraîchi à chaque soumission user (télémétrie) |
| `messages` | `Vec<Message>` | Historique ordonné |
| `running_turn` | `Option<RunningTurn>` | Tâche async du loop modèle en cours |
| `pending_message` | `Option<AgentMessage>` | Message assistant en cours d'assemblage (streaming) |
| `tools` | `BTreeMap<SharedString, Arc<dyn AnyAgentTool>>` | Outils activés pour ce turn |
| `model` | `Option<Arc<dyn LanguageModel>>` | LLM sélectionné |
| `profile_id` | `AgentProfileId` | Profil actif (contrôle quels outils sont actifs) |
| `project_context` | `Entity<ProjectContext>` | Contexte du system prompt (worktrees, rules, skills) |
| `thinking_enabled` / `thinking_effort` / `speed` | divers | Paramètres LM |
| `subagent_context` | `Option<SubagentContext>` | Set si c'est un sous-agent spawn |

### Hiérarchie des messages

```rust
enum Message {              // thread.rs:127
    User(UserMessage),
    Agent(AgentMessage),
    Resume,                 // synthetic "continue où tu en étais"
}

struct UserMessage {        // thread.rs:177
    id: UserMessageId,      // UUID stable à travers la sérialisation
    content: Vec<UserMessageContent>,
}

enum UserMessageContent {
    Text(String),
    Image(LanguageModelImage),
    Mention { uri: MentionUri, content: String },
}

struct AgentMessage {       // thread.rs:628
    content: Vec<AgentMessageContent>,
    tool_results: IndexMap<LanguageModelToolUseId, LanguageModelToolResult>,
    reasoning_details: Option<serde_json::Value>,
}

enum AgentMessageContent {
    Text(String),
    Thinking { text: String, signature: Option<String> },
    RedactedThinking(String),
    ToolUse(LanguageModelToolUse),
}
```

À la sérialisation vers `LanguageModelRequestMessage`, les `Mention` sont **expansées en blocs XML** (`<files>`, `<symbols>`, `<rules>`, `<skills>`, etc.) ajoutés après le texte (`thread.rs:215-471`).

### Côté affichage (`acp_thread::AcpThread`)

`AcpThread` est le modèle de **rendu**. Il reçoit les événements push de la connexion active et expose une liste d'`AgentThreadEntry` :

```rust
enum AgentThreadEntry {       // acp_thread/src/acp_thread.rs:178
    UserMessage(UserMessage),
    AssistantMessage(AssistantMessage),
    ToolCall(ToolCall),
    CompletedPlan(Vec<PlanEntry>),
}
```

C'est cette structure que `ThreadView` rend dans la `list()` virtuelle. Cette séparation est délibérée : `Thread` est la source de vérité pour le moteur, `AcpThread` est ce que l'UI traverse. Les agents ACP externes peuvent populer `AcpThread` sans jamais toucher à `Thread`.

---

## 5. Cycle de vie d'un turn (data flow)

Soumission user → réponse finale. Trace concrète :

1. **User submit** → `ConversationView` appelle `AgentConnection::prompt()` avec un `acp::PromptRequest`.

2. **`NativeAgentConnection::prompt()`** (`agent.rs:2206`) parse les slash commands (skills, MCP prompts). Pour un message normal, appelle `self.run_turn(session_id, cx, |thread, cx| thread.update(cx, |thread, cx| thread.send(id, content, cx)))` (lignes 2337-2348).

3. **`Thread::send()`** (`thread.rs:1908`) push un `Message::User`, puis appelle `send_existing()` (ligne 1924).

4. **`Thread::send_existing()`** (`thread.rs:1927`) avance `prompt_id`, vérifie le modèle, appelle `self.run_turn(cx)`.

5. **`Thread::run_turn()`** (`thread.rs:1979`) cancel le turn précédent, crée un canal `mpsc::unbounded`, set `self.running_turn`, et spawn `Self::run_turn_internal()` comme tâche GPUI async.

6. **`Thread::run_turn_internal()`** (`thread.rs:2044`) entre dans une boucle. Chaque itération :
   - Appelle `build_completion_request(intent, cx)` (`thread.rs:2945`) qui assemble le `LanguageModelRequest` (system prompt via `build_request_messages` ligne 3154, tous les `messages`, les outils courants, température, thinking).
   - Appelle `model.stream_completion(request, cx).await` (ligne 2080) — **l'appel LM réel**.

7. **Stream processing** (boucle ligne 2088) — race entre `events.next()`, futures d'outils, et cancellation. Chaque `LanguageModelCompletionEvent` est dispatché par `handle_completion_event()` :
   - **Text chunk** → push dans `pending_message.content` comme `AgentMessageContent::Text`.
   - **Thinking chunk** → push comme `AgentMessageContent::Thinking`.
   - **ToolUse event** → dispatch via `spawn_tool()`, retourne `Task<LanguageModelToolResult>` ajoutée à `tool_results: FuturesUnordered`.

8. **Tool execution** — outils en concurrence. Si un outil nécessite autorisation, un `ThreadEvent::ToolCallAuthorization` est envoyé au canal upstream. L'UI présente la dialog et renvoie la décision via un `oneshot::Sender`.

9. **`handle_thread_events()`** (`agent.rs:1832`) tourne en tâche externe — celle qui retourne le `Task<Result<acp::PromptResponse>>` final. Elle reçoit les `ThreadEvent` et les forward vers `AcpThread` :
   - `AgentText(text)` → `acp_thread.push_assistant_content_block(text, false, cx)` (1858)
   - `ToolCall(call)` → `acp_thread.upsert_tool_call(call, cx)` (1893)
   - `Stop(reason)` → retourne `Ok(acp::PromptResponse::new(stop_reason))` (1917)

10. **Boucle tool results** — après fin du stream et settlement des outils (`tool_results.next().await`), `process_tool_result()` (`thread.rs:2254`) écrit le résultat dans `pending_message.tool_results`, puis `flush_pending_message()` déplace l'`AgentMessage` assemblé dans `self.messages`.

11. **Next iteration** — s'il y a des tool results et aucun message en file d'attente, `intent = CompletionIntent::ToolResults` et la boucle continue avec un nouvel appel modèle portant les tool results.

12. **Title generation** — après le premier turn, `generate_title()` (ligne 2206) lance un appel LM secondaire pour produire le titre du thread.

**Insight pour Paneflow** : la boucle est un **loop tool-result-aware**, pas un simple appel-réponse. Tant que le modèle émet des `tool_use`, on continue. C'est ce qui transforme un chat en agent.

---

## 6. Persistance

**Stockage threads** : SQLite à `~/.local/share/zed/threads/threads.db` (in-memory pour tests ou `ZED_STATELESS`). `ThreadsDatabase` enveloppe une `sqlez::Connection` dans `Arc<Mutex<>>` (`agent/src/db.rs:346`).

**Schéma principal** (`db.rs:398`) :

```sql
CREATE TABLE threads (
    id TEXT PRIMARY KEY,
    summary TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    data_type TEXT NOT NULL,   -- "json" ou "zstd"
    data BLOB NOT NULL,
    parent_id TEXT,
    folder_paths TEXT,         -- groupement projet
    folder_paths_order TEXT,
    created_at TEXT
)
```

La colonne `data` stocke un **blob JSON zstd-compressé** de `DbThread` (version `"0.3.0"`). Toute la conversation (messages, tool uses, results, thinking blocks) est dans ce blob. Compression niveau 3.

**`ThreadStore`** (`thread_store.rs:12`) — global GPUI qui expose `load_thread()`, `save_thread()`, `delete_thread()`, `reload()`. `ThreadsDatabase::connect()` est idempotent.

**Resume** :
- **Native** : `NativeAgentConnection::load_thread()` (`agent.rs:1800`) charge le `DbThread`, reconstruit un `Thread` et crée un nouveau `AcpThread` initialisé avec l'historique (replay via `push_user_content_block` / `push_assistant_content_block` / `upsert_tool_call` pour chaque message stocké).
- **ACP externe** : si `supports_load_session()`, on envoie le session_id au protocole (le subprocess restaure son propre état). Si `supports_resume_session()`, on reconnecte sans replay. Par défaut les deux sont `false`.

**Métadonnées thread** (séparé du blob) : `ThreadMetadataStore` (`agent_ui/src/thread_metadata_store.rs:550`) — autre SQLite, schéma `sidebar_threads` (thread_id, session_id, agent_id, title, title_override, updated_at, created_at, interacted_at, folder_paths, archived, remote_connection). C'est ce que la sidebar d'historique lit pour la liste rapide sans charger les blobs complets.

**Drafts** (saisie en cours, non envoyée) : stockés à part dans le `KeyValueStore` namespace `"agent_draft_prompts"`, valeur = `Vec<acp::ContentBlock>` JSON-sérialisé (préserve les mentions). Voir §20.

**Insight pour Paneflow** : sépare bien (1) la liste rapide pour la sidebar, (2) le blob complet du thread, (3) les drafts. Ne charge le blob complet que sur ouverture du thread. Le zstd sur le blob est gratuit en CPU et divise par 5-10 le poids des longues conversations.

---

## 7. Réglages utilisateur

Tous dans `AgentSettings` (`agent_settings/src/agent_settings.rs:137`). Sous-ensemble important :

| Champ | Type | Usage |
|---|---|---|
| `default_model` | `Option<LanguageModelSelection>` | LM par défaut (provider + modèle + thinking + effort + speed) |
| `subagent_model` | `Option<LanguageModelSelection>` | Override pour subagents spawn |
| `inline_assistant_model` | `Option<LanguageModelSelection>` | Modèle des completions inline |
| `thread_summary_model` | `Option<LanguageModelSelection>` | Modèle pour résumés/titres |
| `default_profile` | `AgentProfileId` | Profil actif ("write", "ask", "minimal", ou custom) |
| `profiles` | `IndexMap<AgentProfileId, AgentProfileSettings>` | Tous les profils |
| `tool_permissions` | `ToolPermissions` | Allow/deny patterns par outil |
| `thinking_display` | `ThinkingBlockDisplay` | Comment afficher les blocs thinking |
| `dock` | `DockPosition` | Position panel (left/right) |
| `flexible` | `bool` | Taille flexible du panel |
| `model_parameters` | `Vec<LanguageModelParameters>` | Overrides température par provider/modèle |
| `notify_when_agent_waiting` | `NotifyWhenAgentWaiting` | Comportement notif OS |
| `show_turn_stats` | `bool` | Afficher tokens/durée |
| `use_modifier_to_send` | `bool` | Shift+Enter vs Enter |
| `favorite_models` | `Vec<LanguageModelSelection>` | Modèles favoris pour cycle rapide |
| `max_content_width` | (px) | Largeur max colonne de texte conversation |

`AgentProfileSettings` (`agent_profile.rs:104`) :

```rust
struct AgentProfileSettings {
    name: SharedString,
    tools: IndexMap<Arc<str>, bool>,            // overrides on/off par outil
    enable_all_context_servers: bool,
    context_servers: IndexMap<Arc<str>, ContextServerPreset>,
    default_model: Option<LanguageModelSelection>,  // override modèle par profil
}
```

Trois profils built-in : **`write`** (défaut, tous outils mutants OK), **`ask`** (lecture seule), **`minimal`** (juste l'essentiel).

---

## 8. Anatomie du panneau Agent

Le panel est un `v_flex().size_full().justify_between()` enveloppé dans un `WithRemSize` qui scale l'ensemble à `agent_ui_font_size` (rythme typographique indépendant de l'éditeur, `agent_panel.rs:5839-5846`).

```
┌──────────────────────────────────┐
│  TOOLBAR (tab_bar_background)    │  ← border_b, hauteur = Tab::container_height
│  [icon/title] [+ ⛶ ⋯]            │
├──────────────────────────────────┤
│  ONBOARDING UPSELL (conditional) │  ← editor_background, dismissable
├──────────────────────────────────┤
│                                  │
│  MESSAGE STREAM (flex-1)         │  ← virtualized list(), centered
│  ┌────────────────────────────┐  │     max_content_width configurable
│  │  user message bubble       │  │
│  │  assistant response        │  │
│  │  tool call cards           │  │
│  └────────────────────────────┘  │
│                                  │
├──────────────────────────────────┤
│  ACTIVITY BAR (conditional)      │  ← rounded_t_md, border, shadow
│  [edits] [plan] [queue]          │     visible seulement si contenu
├──────────────────────────────────┤
│  COMPOSER (p_2, editor_bg)       │  ← border_t
│  ┌────────────────────────────┐  │
│  │  message editor (auto-h)   │  │
│  └────────────────────────────┘  │
│  [+] [↪] [fast] [16px●] [P][M][▶]│ ← footer bar
└──────────────────────────────────┘
```

La zone visible est gouvernée par `VisibleSurface` qui switche entre `AgentThread`, `Terminal`, `Configuration`, `Uninitialized`.

### Docking & sizing

`impl Panel for AgentPanel` (`agent_panel.rs:4245`) :

- `position_is_valid` rejette explicitement `DockPosition::Bottom` (4258-4260) — **gauche/droite uniquement**.
- `default_size` lit `AgentSettings::default_width` / `default_height` (4276-4281).
- `MIN_PANEL_WIDTH = px(300.)` (4284-4288).
- `flexible` opt-in via settings, permet de partager l'espace avec l'éditeur (4291-4303).
- `panel_key = "agent_panel"` pour la persistance (`AGENT_PANEL_KEY`, ligne 93).
- L'icône du panel dans la sidebar est `IconName::ZedAssistant` (4319).

---

## 9. Toolbar & menus

La toolbar a quatre modes selon l'état du panneau :

**Mode EmptyThread** (aucun message, projet ouvert) — la partie gauche entière devient un **gros bouton agent-selector** : `Button::new("agent-selector-trigger", label)` avec leading icon (SVG custom ou `IconName::ZedAgent`) et trailing chevron qui flip selon ouverture du menu (`agent_panel.rs:5363-5408`). Couleurs : `Color::Accent` quand déployé, sinon `Color::Default` / `Color::Muted`.

**Modes ActiveThread / Terminal / Overlay** :
- Gauche : petite icône fixe (`px_0p5`) de l'agent à `IconSize` par défaut en `Color::Muted`, avec animation `pulsating_between(0.2, 0.6)` à 1s en boucle pendant le chargement (5299-5311).
- À côté : zone titre via `render_title_view`.

**Cluster droit** (tous modes) — `h_flex` avec `gap_1`, `pl_1`, `pr_1` :
1. **New Thread** — `IconButton(IconName::Plus)`, `IconSize::Small`, ancré `TopRight` (5431-5447).
2. **Full-screen toggle** — `IconName::Maximize` / `IconName::Minimize` (5344-5349).
3. **Options menu** — `IconButton(IconName::Ellipsis)`, ancré `TopRight` (5837-5849).

**Menu options** (sections) : Current Thread (regenerate title) · MCP Servers · Skills · Rules · Profiles · Settings · Toggle Threads Sidebar · (optionnel) Reauthenticate / Log Out (4854-4982).

**Title area** (`render_title_view`, 4599) — prend l'espace horizontal restant. Quand un thread native est actif, le titre est un `Editor` inline click-to-edit. Pendant la génération AI du titre, un `Label` avec animation `alpha` 0.4–0.8 sur 2s en `Color::Muted`. **Détail à voler** : un `GradientFade` de 64 px overlay le bord droit pour fade dans le background toolbar, avec un bouton `IconName::Pencil` `visible_on_hover` (4727-4756).

---

## 10. Composer (input)

Le composer wrap directement l'`Editor` de Zed (`message_editor.rs:468`). C'est un `Buffer::local("")` → `MultiBuffer::singleton` → `Editor::new(mode, ...)`.

**Configuration de l'éditeur** :
- Mode `EditorMode::AutoHeight { min_lines: 1, max_lines: None }` (2482-2484) — grandit sans limite ; le scroll interne n'apparaît que si le flex parent contraint.
- `set_placeholder_text(placeholder, ...)` (479).
- `set_soft_wrap()` activé, indent guides off, `set_show_completions_on_input(Some(true))`, modal editing on, context menu cap 12 (480-489).

**Submit key** — contrôlé par `use_modifier_to_send` :
- `false` (défaut) : `enter` → `agent::Chat` ; `ctrl-enter` → `agent::ChatWithFollow`.
- `true` : `ctrl-enter` → submit ; `enter` → newline. Implémenté en injectant la chaîne `"use_modifier_to_send"` dans le key context (2052-2053), ce qui flip la binding active.
- `agent::SendImmediately` (toujours `ctrl-shift-enter`) bypass toute file d'attente.

**Events émis par le `MessageEditor`** : `Send`, `SendImmediately`, `Cancel`, `Focus`, `LostFocus`, `SlashAutocompleteOpened`, `InputAttempted` (203-217).

**Container visuel** — `h_flex().p_2().bg(editor_background)` avec `border_t_1().border_color(border)` quand une conversation est déjà en cours (`thread_view.rs:3573-3586`). Quand le panel est vide, le composer remplit l'espace (`flex_1().size_full()`).

**Expand toggle** : peut être expandé à `vh(0.8)` via un `IconButton` Maximize/Minimize qui apparaît à `opacity(0.5)` top-right, monte à `opacity(1.0)` au hover (3605-3636).

**Footer bar** — `h_flex().w_full().flex_wrap().justify_between()` :

- **Cluster gauche** (`gap_0p5`) : `+` add-context (ouvre ContextMenu slash/refs ancré `BottomLeft`) · follow-toggle · fast-mode (staff-only, `IconName::FastForward` / `IconName::FastForwardOff`) · thinking-mode toggle.
- **Cluster droit** (`gap_1`, `flex_wrap`) : token-usage ring → profile selector → mode selector → model selector → send button (3639-3665).

**Send button state machine** (3 états + 1 variant) :

| État | Icon | Style | Color |
|---|---|---|---|
| Idle, vide | (ghost) | disabled | `Muted` |
| Idle, contenu | `Send` | `Filled` | `Accent` |
| Generating, vide | `Stop` | `Tinted(TintColor::Error)` | `Error` |
| Generating, contenu | `QueueMessage` | `Filled` | `Accent` |

Pas un seul bouton à `disabled=true` — **trois éléments différents** selon l'état. Tooltip à deux lignes quand le mode "queue" : explique séparément "Queue and Send" et "Send Immediately" comme actions distinctes keybindables (4324-4402). Pattern propre, à voler.

---

## 11. Sélecteurs : modèle / mode / profil

Tous trois utilisent un pattern **popover/picker**, pas de dropdown inline. Tous trois sont des `Button` avec :
- `LabelSize::Small`
- leading icon (SVG provider ou `IconName`) à `IconSize::XSmall`
- trailing `ChevronDown` / `ChevronUp` à `IconSize::XSmall`
- couleur `Color::Accent` quand déployé, `Color::Muted` sinon

### Model selector — `Picker<ModelPickerDelegate>` (`model_selector.rs:29`)

- Width `20rem`, max height `20rem`, scrollbar (42-43).
- Liste typée `AgentModelList` (`connection.rs:468`) : `Flat(Vec)` ou `Grouped(IndexMap<AgentModelGroupName, Vec>)`.
- `AgentModelInfo` (442) : `id`, `name`, `description`, `icon`, `is_latest`, `cost`.
- **Favoris** : `HashSet<acp::ModelId>` peuplé depuis `AgentSettings::favorite_models`, sérialisé `"<provider>/<model>"`. `cycle_favorite_models()` cycle entre favoris. Holding modifier + confirm toggle le favori (280).
- Star toggle button hover-reveal sur chaque ligne (`ui/model_selector_components.rs:128-180`).
- Footer "Configure" outlined button.

### Mode selector — `ContextMenu` (`mode_selector.rs:166`)

- **Modes dynamiques**, pas un enum hardcodé. Vient de `AgentSessionModes::all_modes()` / `current_mode()` exposé par le trait sur la connection.
- Chaque mode peut avoir un `documentation_aside` rendu à gauche ou droite.
- **Holding ⌘ + click** = set comme défaut. Communiqué via composant `HoldForDefault` : "Hold ⌘ to set as default" en `text_sm`, `Color::Muted`, séparé par un border-top.
- `cycle_mode()` action keybindable.

### Profile selector — `PickerPopoverMenu` (`profile_selector.rs:161`)

- Width `18rem`, max height `20rem`, ancré `BottomRight`.
- Lignes groupées "Built-in" / "Custom" avec fuzzy highlight (219-228).
- `ProfileProvider` trait (25) abstrait native vs ACP.

**Tooltip riche** sur les trois — `v_flex().gap_1()` à deux lignes : action principale + keybinding ; deuxième ligne pour le cycle shortcut.

---

## 12. Vue conversation : rendering détaillé

### Structure

Deux niveaux d'entités :
- **`ConversationView`** (`conversation_view.rs:500`) — outer shell. `Render` impl à 3079 dispatch sur `ServerState` (loading, error, auth, connected).
- **`ThreadView`** (`thread_view.rs:537`) — owns la liste de messages.

La liste est une **GPUI `list()` virtualized** backed by `ListState` (`conversation_view.rs:1088`) :

```rust
ListState::new(0, gpui::ListAlignment::Top, px(2048.))
list_state.set_follow_mode(gpui::FollowMode::Tail)
```

Items ancrés en haut, follow-tail pour l'auto-scroll, 2048 px d'overscan. Chaque item est centré avec largeur max optionnelle = `AgentSettings::max_content_width` → sur grand écran la colonne de texte reste lisible (`thread_view.rs:4831`).

### Types de messages

`render_entry` (`thread_view.rs:4863`) dispatch sur `AgentThreadEntry` :

**User message** (4881) :
- Texte = `Editor` live (instance `MessageEditor` embedée) dans container stylé.
- Visuel : `rounded_md`, `bg(editor_background)`, `border_1`, `border_color(border)`, `shadow_md`. Hover éditable → `border_color(focus_border).opacity(0.8)`. En édition → `focus_border`. Subagent non-éditable → `border_dashed`.
- Toolbar flottante au-dessus quand l'éditeur a focus : `IconName::Close` (error color) + `IconName::Return` (muted), positionnée absolument `top_neg_3p5`, `right_3` (4977-5053).
- Bouton "Restore Checkpoint" (`IconName::Undo`) sur ligne séparateur si checkpoint (4923).
- Padding : `py_3`, `px_2` (outer), `pt_2`, `pb_3`, `px_2` (column).

**Assistant message** (5057) :
- `chunks: Vec<AssistantMessageChunk>` — `Message { block }` ou `Thought { block }`, chaque block tient lazily une `Markdown` entity.
- Visuel : **aucun background** — flush sur le bg du panel. `px_5`, `py_1p5`, dernier message ajoute `pb_4`. Chunks multiples empilés `v_flex().gap_3()`.
- Messages assistant blank (whitespace-only) supprimés entièrement (5108).

**Tool calls** (5128) — dispatch sur `render_any_tool_call` (6631) qui route vers terminal / subagent / standard. Voir §13.

**Completed plans** — `render_completed_plan` (3228).

**Indented entries (subagent content)** : `is_indented() == true` ⇒ left vertical border (`w_px()`, `bg(border.opacity(0.6))`) + `pl_5`, visuellement nesting des turns subagent sous le parent (5193).

### Streaming

Pendant la génération, la `ListState` a un item virtuel supplémentaire au-delà des entries réelles : l'**indicateur de génération** (`render_generating`, 5743) :
- `SpinnerVariant::Dots` (`GeneratingSpinnerElement`) — stateful animated view.
- Awaiting confirmation : `SpinnerVariant::Sand` + `LoadingLabel::new("Awaiting Confirmation")`.
- Optionnel : elapsed time label (après 30s, `STOPWATCH_THRESHOLD`) + token count (après 250 tokens, `TOKEN_THRESHOLD`), muted/small.
- Layout : `h_flex`, `py_2`, `px(22px)`, `gap_2`.

**Auto-scroll** : `FollowMode::Tail` gère naturellement. Sur message envoyé : `list_state.scroll_to_end()` explicite (1432). Le flag `should_be_following` (576) track si user en mode follow. Pendant la génération, lié à `workspace.follow(CollaboratorId::Agent, ...)` (1363).

**Pas de blinking cursor** : les chunks streamés update directement la `Markdown` entity et `cx.notify()` trigger re-render.

### Markdown

Parser `pulldown_cmark` dans `crates/markdown` (`markdown.rs:43`). Le `Markdown` struct (320) owns source parsé, `ParsedMarkdown` tree, et une `LanguageRegistry` pour le syntax highlighting.

**Font variant Agent** (`MarkdownFont::Agent`, 141) :
- Body = `agent_ui_font_size` (police UI)
- Code = `agent_buffer_font_size` (police buffer/mono)
- Line height = `buffer_font_size * 1.75`

**Headings agent** (272) : H1=`rems(1.15)`, H2=`1.10`, H3=`1.05`, H4=`1.0`, H5=`0.95`, H6=`0.875`.

**Links** : `text_accent`, fond `editor_foreground.opacity(0.025)`, underline 1px en `text_accent.opacity(0.5)` (262).

**Inline code** : police buffer, fond `editor_foreground.opacity(0.08)` (253).

**Block quotes** : left border `block_quote_border_color`. GFM alerts (Note/Tip/Warning/etc.) avec couleurs status distinctes (210).

Le `render_agent_markdown` (`conversation_view.rs:3145`) wrap `MarkdownElement::new(...)` avec :
- `code_block_renderer` avec `CopyButtonVisibility::VisibleOnHover` + `WrapButtonVisibility::VisibleOnHover`.
- Image resolver qui mappe URLs relatives → paths absolus worktree-relative.
- `on_url_click` callback `open_link`.
- `on_code_span_link` callback `AgentCodeSpanResolver` (LRU 2048 entries, 3176) — convertit les inline code spans qui ressemblent à un path (`src/main.rs:42`) en liens cliquables workspace.

### Code blocks

Highlight via `language::Language::highlight_text` + tree-sitter. Theme = `SyntaxTheme` actif (markdown.rs:105).

**Standard markdown code blocks** : `bg(editor_background)`, `border(border_variant)` 1px, padding 8px, marge top 8px / bottom 12px. Buffer font à `buffer_font_size`. Overflow horizontal → `overflow_x_scroll` (220). Bouton copy + wrap-toggle visible au hover.

**`read_file` tool output — numbered code blocks** : path spécial. `render_numbered_read_file_output` (`thread_view.rs:8113`) parse les sorties `cat -n` et appelle `render_cat_numbered_code_block` (308) qui :
- Render une **gutter** avec numéros de ligne droite-alignés en `text_muted`.
- Code avec syntax highlighting via `highlight_code_runs` (409) → `language.highlight_text()`.
- Bouton copy custom positionné `absolute().top_0().right_0()`, visible sur hover du groupe `"read-file-code-block"`.

### Diffs

**`AgentDiffPane`** (`agent_diff.rs:41`) — pane item séparé (pas inline dans le thread), centre du workspace. `SplittableEditor` wrap un `MultiBuffer` qui aggregate les hunks de tous les fichiers changés.

- Style (split vs unified) contrôlé par `EditorSettings::diff_view_style` (95).
- `update_excerpts` (132) monitore le `action_log` du thread, extrait les ranges de hunks + context lines. Fichiers triés alphabétiquement (142). Fichiers supprimés foldés par défaut (212).
- Hunk controls inline (accept/reject par hunk) via `set_render_diff_hunk_controls` (104).

**Diff inline dans le thread** : `render_diff_editor` (`thread_view.rs:8025`) render un editor inline pour un seul diff **seulement** si `diff.has_revealed_range(cx)` — i.e. après que l'agent a streamé l'edit réel. Pendant le streaming (InProgress/Pending), `render_diff_loading` à la place.

### Tool call cards

`render_tool_call` (6686). Trois variants.

**Standard** :
- Header (`render_tool_call_label`, 7699) — 1 line-height moins 2px, `h_flex`. Contient :
  - Icon left : file icon (extension) pour Edit ; sinon `ToolSearch`, `ToolPencil`, `ToolDeleteFile`, `ArrowRightLeft`, `ToolTerminal`, `ToolThink`, `ToolWeb`, `ToolHammer` selon `acp::ToolKind`.
  - Edit failed + diff revealed → `DecoratedIcon` avec badge warning triangle.
  - Tool label text truncated avec **gradient overlay 48px** linear-gradient → transparent au bord droit.
- Expand/collapse trackés dans `ThreadView::expanded_tool_calls` (566). Forcé open quand `WaitingForConfirmation`.
- States : `WaitingForConfirmation` → card layout avec `tool_card_header_bg`. Failed/rejected → `border_dashed`.
- Raw input → section collapsible "View Raw Input" rendue comme markdown (non-edit, non-terminal, non-image tools seulement).
- Output : `ToolCallContent` rend markdown / image / diff editor.
- Permission buttons : voir §13.

**Terminal tool call** (`render_terminal_tool_call`, 6328) :
- Header : working dir, elapsed time, exit status, line count.
- Command rendue via `render_collapsible_command` (6269) — code block buffer font 12px. Copy button supprimé sur le markdown interne ; un `CopyButton` extérieur capture le raw text (sans fences).

**Subagent tool call** (`render_subagent_tool_call`, 8210) — card avec titre, status icon, diff stats, sous-thread expandable. Status icons : `SpinnerLabel` (running), `IconName::Circle` dimmed (cancelled), `IconName::Close` error, `IconName::Check` success. Sans titre ou failed/cancelled → `border_dashed`.

### Thinking blocks

`render_thinking_block` (5933). Modes (`AgentSettings::thinking_display`) :
- `Auto` : collapsed par défaut, auto-expand du dernier thought streaming.
- `Preview` : collapsed mais montre 256px de preview avec gradient fade (`max_h_64` + `linear_gradient` `panel_bg.opacity(0.8)` → transparent au top, 6044-6057).
- `AlwaysExpanded` / `AlwaysCollapsed` : override toggle user.

**Visuel** :
- Header `h_flex`, full width, `ToolThink` icon + "Thinking" label muted, `Disclosure` chevron right-aligned. Toute la row clickable.
- Content open : indenté `ml_1p5`, `pl_3p5`, `border_l_1`, `border_color(tool_card_border_color)`.
- Manual override tracké dans `user_toggled_thinking_blocks` (570).

### Per-message actions

**Right-click context menu sur assistant** (`render_message_context_menu`, 6063) :
- Triggé par `right_click_menu` wrap du message body.
- Items : Copy This Agent Response · Copy Selected Text (si sélection markdown) · Open Link (si right-click sur lien) · Scroll to Top / Bottom.

**Thread-end controls** (`render_thread_controls`, 5306) — sous le dernier entry :
- Icon buttons opacity 0.6 → 1.0 au hover : Open Thread as Markdown (`FileMarkdown`) · Scroll To Most Recent User Prompt (`ForwardArrow`) · Scroll To Top (`ArrowUp`).
- Optionnel : turn stats (durée + tokens en small muted).
- Optionnel : thumbs-up / thumbs-down si `enable_feedback`.

**User message edit** : click sur user message → `editing_message` set → border highlight, toolbar flottante avec close + send.

---

## 13. Tool calls & permissions

### Tool system

Deux traits dans `agent/src/thread.rs` :

`AgentTool` (3501) — trait typé :
```rust
trait AgentTool {
    type Input: Deserialize + Serialize + JsonSchema;
    type Output: Into<LanguageModelToolResultContent>;
    const NAME: &'static str;
    fn kind() -> acp::ToolKind;
    fn run(self: Arc<Self>, input: ToolInput<Self::Input>, event_stream: ToolCallEventStream, cx: &mut App)
        -> Task<Result<Self::Output, Self::Output>>;
    //                       ↑ Notez : l'erreur est Self::Output, pas anyhow::Error
}
```

L'erreur typée comme `Output` permet de retourner les erreurs au modèle comme **contenu structuré** plutôt que comme exception.

`AnyAgentTool` (3596) — version object-safe. Stocké comme `Arc<dyn AnyAgentTool>` dans `BTreeMap<SharedString, ...>` (976).

### Outils built-in (`add_default_tools()`, thread.rs:1654)

23 outils :

| Catégorie | Outils |
|---|---|
| FS read | `ReadFileTool`, `ListDirectoryTool`, `FindPathTool`, `GrepTool` |
| FS write | `EditFileTool`, `WriteFileTool`, `CreateDirectoryTool`, `MovePathTool`, `CopyPathTool`, `DeletePathTool` |
| Shell | `TerminalTool` |
| Network | `FetchTool`, `WebSearchTool` |
| LSP (feature-flag) | `FindReferencesTool`, `GetCodeActionsTool`, `ApplyCodeActionTool`, `GoToDefinitionTool`, `RenameTool`, `DiagnosticsTool` |
| Meta | `SpawnAgentTool` (depth-limited à `MAX_SUBAGENT_DEPTH = 1`), `UpdatePlanTool`, `UpdateTitleTool`, `SkillTool` |

### Flow d'invocation (`thread.rs:2482`)

1. **Streaming input** : si `tool.supports_input_streaming()` et input pas encore complet, crée un `ToolInputSender` et call `run_tool` immédiatement avec stream live (2486). `EditFileTool` l'utilise pour streamer les diffs.
2. **Non-streaming** : input complet → `ToolInput::ready(tool_use.input)` → `run_tool()` (2523).
3. **`run_tool()`** (2535) wrap exécution dans foreground task. Filtre les images de l'output si le modèle ne supporte pas (2560).

**Gating par profil** : `enabled_tools()` (3008) consulte `AgentProfileSettings` — seuls les outils où `profile.is_tool_enabled(tool_name) == true` sont dans `running_turn.tools`. Le modèle ne **voit même pas** les outils désactivés.

### Permissions

`ToolPermissionMode` (`settings_content/src/agent.rs:639`) :
- `Allow` — auto-approuve.
- `Deny` — auto-rejette avec erreur.
- `Confirm` (défaut) — toujours prompt.

`ToolPermissions` (`agent_settings.rs:338`) : `default: ToolPermissionMode` global + map `tool_name → ToolRules` (373) avec listes regex `always_allow`, `always_deny`, `always_confirm`.

**Règles hardcodées non-overridables** (`agent/src/tool_permissions.rs:20`) : bloque `rm -rf` sur `/`, `~`, `$HOME`, `.`, `..` avant toute check settings.

**Flow d'approbation** : quand un tool nécessite confirmation, `run_authorization_loop()` (`thread.rs:4092`) :
1. Check `ToolPermissionDecision` from settings. Si `Allow` → return. Si `Deny` → error. Si `Confirm` → continue.
2. Émet `ThreadEvent::ToolCallAuthorization` avec `ToolCallAuthorization { title, PermissionOptions, oneshot_sender }`. `PermissionOptions` contient `acp::PermissionOption` avec IDs `"allow"`, `"deny"`, `"always_allow_pattern"`, `"always_deny_pattern"`.
3. Loop race **user input vs settings live changes** — si l'user modifie settings.json pendant prompt pending, la loop re-évalue et peut auto-résoudre sans action (4155).
4. UI rend les options comme button group sur la tool call card via `render_permission_buttons` (`thread_view.rs:7138`) — variantes flat ou dropdown avec "Allow Once / Allow Always / Reject".
5. Sur sélection pattern-based, `persist_permission_outcome()` (4204) écrit dans settings.json — devient permanent.

Pour `TerminalTool`, les options "Always allow" pattern sont supprimées sur shells qui ne supportent pas le chaining POSIX (`&&`, `||`, `;`, `|`) — Nushell, Elvish, Rc — parce que sans parsing fiable des sub-commands, le pattern matching n'est pas safe (785).

---

## 14. Mentions, slash commands, attachments

### Mentions (@)

`MentionSet` (`mention_set.rs:61`). L'enum `MentionUri` couvre 13 variantes :

| Variante | Sens |
|---|---|
| `File { abs_path }` | Fichier |
| `Directory { abs_path }` | Dossier (produit listing) |
| `Symbol { abs_path, line_range }` | Symbole code à range donné |
| `Selection { abs_path, line_range }` | Sélection éditeur |
| `Thread { id }` | Thread antérieur |
| `Fetch { url }` | URL à fetcher |
| `Rule { id }` | Fichier `.rules` |
| `Skill { skill_file_path }` | Définition de skill |
| `Diagnostics { include_errors, include_warnings }` | Diagnostics workspace |
| `GitDiff { base_ref }` | Diff branche vs ref |
| `PastedImage` | Image collée |
| `TerminalSelection` | Sélection dans terminal |
| `MergeConflict` | Contenu de merge conflict |

**Completion** (`completion_provider.rs:156`) — `PromptContextType` : `File`, `Symbol`, `Fetch`, `Thread`, `Skill`, `Diagnostics`, `BranchDiff`. Au-delà de File et Symbol, nécessite `prompt_capabilities.embedded_context` (`message_editor.rs:96`).

**Affichage chips** : `MentionCrease` (`ui/mention_crease.rs:21`) — `ButtonLike` :
- `ButtonStyle::Outlined` base, `ButtonStyle::Tinted(TintColor::Accent)` quand toggled/selected.
- `ButtonSize::Compact`, hauteur = `line_height - 1px`.
- Icon `XSmall`, `Color::Muted`, label en buffer font à `agent_buffer_font_size`.
- Loading state : animation pulsating opacity sur 2s.
- Click ouvre le fichier/symbole/sélection/dossier dans le workspace.
- Tooltip hover : text simple OU image preview (pour pasted images) via `hoverable_tooltip` (142).

Ces chips vivent dans le display map de l'`Editor` comme **creases** (inline decorations), pas comme éléments UI séparés. Présents uniquement dans l'input éditable user.

**Sérialisation au send** (`thread.rs:215`) : chaque mention est sérialisée dans une enveloppe XML-tag (`<files>`, `<directories>`, `<symbols>`, `<threads>`, `<fetched_urls>`, `<rules>`, `<skills>`, `<diagnostics>`, `<diffs>`, `<merge_conflicts>`) dans un bloc `<context>` prepended au texte user.

### Slash commands

Parsing dans `PromptCompletionProvider` : `SlashCommandCompletion::try_parse()` (`completion_provider.rs:1807`) walk en arrière depuis le curseur pour le dernier `/` à word boundary.

Sources :
- `AvailableCommand` entries from `session_capabilities.available_commands` — déclarés par le serveur ACP via `acp::AvailableCommand`. Champs : `name`, `description`, `requires_argument`, optionnel `source`.
- `AvailableSkill` entries discovered file-system.

Skills surfacent comme `/<scope>:<name>` — scope vide = global, sinon worktree root name. Skills avec `disable_model_invocation = true` cachés au modèle mais accessibles via slash command.

### Attachments / drag-drop

Trois `on_drop` handlers sur `AgentPanel` (`agent_panel.rs:5632`) :
1. **`DraggedTab`** — extrait `project_path` du pane item.
2. **`DraggedSelection`** — extrait paths du project panel.
3. **`ExternalPaths`** — `handle_external_paths_drop()` (5654) qui résout les paths OS en project paths (ajoute worktrees si besoin), puis `conversation_view.insert_dragged_files()`.

**Paste** : `resolve_pasted_context_items()` (`message_editor.rs:314`) gère `ClipboardEntry::Image` (wrap en `MentionUri::PastedImage` si modèle supporte) et `ClipboardEntry::ExternalPaths`. `supports_images()` vient de `session_capabilities.prompt_capabilities.image`.

---

## 15. Système d'outils & MCP

### MCP (Model Context Protocol)

Dans Zed, **"context server" = "MCP server"** — c'est le nom interne. `ContextServer` (`context_server/src/context_server.rs:40`) supporte trois transports :
- **stdio** (`ContextServerTransport::Stdio`) — subprocess local, communication stdin/stdout.
- **HTTP** — URL distante, streamable HTTP transport (64).
- **Custom** — `Arc<dyn Transport>` (84).

Protocol versions trackées : `2024-11-05`, `2025-03-26`, `2025-06-18`, `2025-11-25` (`types.rs:8-11`).

Requests définies : `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`, `prompts/get`, `completion/complete`, `ping`.

### Configuration utilisateur

Dans `settings.json` sous `context_servers` (section projet). `ConfigureContextServerModal` (`agent_ui/src/agent_configuration/configure_context_server_modal.rs`) — wizard UI pour stdio ou HTTP. Quand une extension Zed qui déclare des context servers est installée, `context_server_configuration.rs:27` ouvre auto le modal via `ExtensionEvents::ExtensionInstalled`. Désinstallation → `remove_context_server_settings()` (66) clean les settings.

**Pas d'auto-discovery**, pas de trust tiers. L'utilisateur doit explicitement ajouter.

### Lifecycle & registry

`ContextServerStore` (`project/src/context_server_store.rs`) gère les states : `Starting`, `Running`, `Stopped`, `Error`, `AuthRequired`, `ClientSecretRequired`, `Authenticating` (51).

`ContextServerRegistry` (`agent/src/tools/context_server_registry.rs`) collecte les tools des servers actifs et les merge dans la map des tools enabled (`thread.rs:3047`). **Disambiguation** : doublons de noms à travers servers sont préfixés par le snake_case server ID (3062).

---

## 16. Profils, modes, providers LLM

### Profils

Voir §7 pour la struct. Trois built-in : `write` (défaut), `ask`, `minimal`.

Sélection : `Thread::set_profile()` (`thread.rs:1738`) appelle `Self::resolve_profile_model()` qui swap au modèle préféré du profil si configuré.

UI : `ProfileSelector` (`profile_selector.rs:39`) — Picker popover. Observe `SettingsStore`. Expose `cycle_profile()` action.

### Modes

**Dynamiques** — pas d'enum hardcodé. `ModeSelector` (`mode_selector.rs:19`) query `self.connection.all_modes()` et `current_mode()`. Noms et descriptions sont des `acp::SessionModeId` / `SharedString` opaques définis par le serveur agent connecté. C'est cohérent avec la philosophie ACP : le serveur sait quels modes il offre.

### Model selector

Voir §11.

### Providers LLM

Trait `LanguageModelProvider` (`language_model/src/language_model.rs:271`) : `id()`, `name()`, `provided_models()`, `is_authenticated()`, `authenticate()`, `configuration_view()`, `reset_credentials()`. Plus `LanguageModelProviderState` (301) pour observabilité GPUI.

Registration : `register_language_model_providers()` (`language_models/src/language_models.rs:222`). Liste complète :

- `CloudLanguageModelProvider` — Zed Cloud hébergé (OAuth)
- `AnthropicLanguageModelProvider`
- `OpenAiLanguageModelProvider`
- `OllamaLanguageModelProvider` — local Ollama
- `LmStudioLanguageModelProvider` — local LM Studio
- `DeepSeekLanguageModelProvider`
- `GoogleLanguageModelProvider` — Gemini
- `MistralLanguageModelProvider`
- `BedrockLanguageModelProvider` — AWS
- `OpenRouterLanguageModelProvider`
- `VercelAiGatewayLanguageModelProvider`
- `XAiLanguageModelProvider` — Grok
- `OpenCodeLanguageModelProvider`
- `CopilotChatLanguageModelProvider` — GitHub Copilot
- `OpenAiSubscribedProvider` — ChatGPT Plus
- `OpenAiCompatibleLanguageModelProvider` — n'importe quel endpoint OpenAI-compatible, **dynamiquement registré** depuis `AllLanguageModelSettings.openai_compatible` map (105)

Extensions tierces : `LanguageModelRegistry::sync_installed_llm_extensions()` appelé sur events `ExtensionStore` (58).

UI auth : `AgentConfiguration` (`agent_configuration.rs:54`) itère `LanguageModelRegistry::visible_providers()` et appelle `provider.configuration_view()` — UI auth par provider. La plupart : API keys via `CredentialsProvider`. Cloud : OAuth Zed.

---

## 17. Skills

Skills = fichiers `SKILL.md` avec frontmatter YAML, dans `<scope>/.agents/skills/<name>/`. Crate `agent_skills` autonome.

**Trois tiers de scopes** (`agent_skills.rs:77`), priorité décroissante :
1. `BuiltIn` — compilé dans le binaire (seul : `create-skill`).
2. `Global` — `~/.agents/skills/`.
3. `ProjectLocal { worktree_id, worktree_root_name }` — `{worktree}/.agents/skills/`.

Same-named higher-priority shadow lower.

**Loading** (`agent_skills.rs:473`) — `load_skills_from_directory(fs, directory, source)` async :
1. `find_skill_files()` — read one level deep, `<entry_dir>/SKILL.md`.
2. Jusqu'à 16 I/O en parallèle, `load_skill_frontmatter()` lit en chunks 4096 bytes jusqu'au closing `---`, parse via `parse_skill_frontmatter()`.
3. **Seul le frontmatter est lu à discovery time**. Body full lu on-demand quand le modèle invoke.

**Struct `Skill`** (57) : `name`, `description`, `source`, `directory_path`, `skill_file_path`, `disable_model_invocation: bool`, `embedded_body: Option<&'static str>`.

**Global index** : `SkillIndex` (165) — global GPUI avec `global_skills` + `project_skills: Vec<ProjectSkillGroup>`. Peuplé par `NativeAgent` qui appelle `load_skills_from_directory()` pour chaque worktree + path global au startup et sur worktree changes.

**Dans le system prompt** : `ProjectContext` (`prompt_store/src/prompts.rs:37`) carry `skills: Vec<SkillSummary>` (name + description + path). Le template Handlebars les rend dans un catalog `<available_skills>`, cappé à `MAX_SKILL_DESCRIPTIONS_SIZE = 50 KB`.

**Invocation** :
- Slash `/<scope>:<name>` → `NativeAgentConnection::prompt()` check `project_state.skills` → si match, `send_skill_invocation()` au lieu du flow normal (`agent.rs:2227-2244`).
- `SkillTool` (`tools/skill_tool.rs`) handle invocations model-initiated. `render_skill_envelope()` (47) wrap le SKILL.md dans `<skill_content>` XML-escaped (anti-injection).

---

## 18. Inline Assistant (buffer & terminal)

### Buffer inline assistant

**Entry** : `InlineAssistant::inline_assist` (`inline_assistant.rs:206`) — workspace action handler. Résout la target (editor ou terminal via `resolve_inline_assist_target`), check model config errors (unauth → re-auth ; autre erreur → prompt natif `["Configure", "Cancel"]`, 266-296), puis dispatch.

**Trigger** : `ctrl-enter` dans context `editor` → `assistant::InlineAssist`.

**UX flow** :
1. `assist` (571) calcule `codegen_ranges` (selection ou ligne), appelle `batch_assist`.
2. Insert **trois blocs editor** autour du range (`insert_assist_blocks`, 611) : sticky `PromptEditor` au-dessus, flex block en dessous pour tool descriptions, sticky bottom border.
3. `PromptEditor` (`inline_prompt_editor.rs`) holds son propre `Editor` pour le prompt text. Submit (Enter/`menu::Confirm`) → `handle_confirm` (545) → `PromptEditorEvent::StartRequested` si status `Idle` ou `Error`.
4. `start_assist` (1229) commence le stream `BufferCodegen`.
5. Pendant `CodegenStatus::Pending` : toolbar montre `IconButton(IconName::Stop)` rouge, tooltip "Changes won't be discarded" (820-833). Stop → `StopRequested`.
6. Sur `Done` : boutons accept apparaissent — `alt-y` / `ctrl-alt-y` → `agent::Keep` ; `ctrl-alt-z` → `agent::Reject`. Si user edit le prompt après Done, restart button (`IconName::RotateCw`, 839).
7. Erreur `BufferCodegen` → `CodegenStatus::Error`. Sans décorations visibles → `workspace.show_toast(Toast::new(id, error))` (1738).

**History** : jusqu'à 20 prompt strings dans `InlineAssistant::prompt_history` (80, 94), cyclables up/down arrow.

### Terminal inline assistant

Même entry (`inline_assist`), dispatch vers `TerminalInlineAssistant::assist` (`terminal_inline_assistant.rs:61`) si active item = `TerminalView`.

**Différences** :
- Pas de buffer ranges, codegen direct sur le PTY.
- `TerminalCodegen` (`terminal_codegen.rs:12`) tient un `TerminalTransaction` qui stream chunks via `terminal.input(bytes)` (118-119). Sanitization anti-execution accidentelle.
- **Undo** : `TerminalTransaction::undo` envoie `\x15` (Ctrl-U, clear ligne) sur Unix, `\x03` (Ctrl-C) sur Windows (176-179).
- **Accept** : `complete` envoie `\x0d` (CR) — execute la commande générée (208-210).
- `PromptEditor` en mode terminal : `secondary_confirm` (Shift-Enter) execute immédiatement au lieu de juste insérer (535-543).
- Contexte limité aux `DEFAULT_CONTEXT_LINES = 50` dernières lignes (37).
- Pas de blocs management — chaque `assist` → nouveau `TerminalInlineAssistId`.

---

## 19. Historique & archive

`ThreadsArchiveView` (`threads_archive_view.rs:140`) — sidebar séparée, pas dans le panel. Toggle via "Toggle Threads Sidebar" du menu options.

### Layout

**Search header** (`render_header`, 842) — `platform_title_bar_height` tall, `border_b_1`. `IconName::MagnifyingGlass` `IconSize::Small` `Color::Muted` + `Editor` inline pour search (placeholder "Search all threads…") + `IconName::Close` clear button visible seulement si query.

**Toolbar** (`render_toolbar`, 930) — `Tab::content_height` row. Gauche : thread count label `LabelSize::Small` `Color::Muted`. Droite : `IconName::Download` (Import Threads) + `IconName::Archive` (toggle archived-only, toggleable state).

**Thread list** — virtualized GPUI `list`. Deux types d'items :

- **`BucketSeparator`** : `div().px_2p5().pt_3().pb_1()` header avec `LabelSize::Small` `Color::Muted`. Buckets : Today, Yesterday, This Week, Past Week, Older (94-100).
- **`Entry`** : `ThreadItem` avec agent icon (`ZedAgent` natif, `Sparkle` externe), titre + fuzzy-match highlight, timestamp formaté, folder paths projet. Hover → action slot droite : `Archive` actif, `Trash` archivé, `Close` cancel restore. Archivés en `Color::Muted` + icon `opacity(0.6)`.

### Recherche & filtres

`update_items` (270) read tous les entries `ThreadMetadataStore`, sort par `created_at` (fallback `updated_at`) desc, assigne à `TimeBucket` (66-102). Separator injecté quand bucket change.

Search : case-insensitive substring fuzzy match (`fuzzy_match_positions`, 105-129) sur title. Non-matchants skip. Highlight byte-positions stockés sur entry.

`ThreadFilter::ArchivedOnly` filtre `t.archived == true`. Si plus aucun archivé après delete, auto-reset à `All` (276-280).

### Restore

Click thread → `ThreadsArchiveViewEvent::Activate { thread }` (133). Parent panel ouvre/load. Restoring trackés dans `restoring: HashSet<ThreadId>` (154).

### Thread worktree archive

`thread_worktree_archive.rs` — companion. À archive : snapshot état branche (staged/unstaged hashes, branch name) dans `archived_git_worktrees`. À restore : worktree recréé.

### Import de threads

Deux paths (`thread_import.rs`) :

**Cross-channel** (606-683) — `import_threads_from_other_channels` lit `database_dir`, pour chaque `ReleaseChannel` non-current et non-Dev, ouvre le SQLite directement via `sqlez::connection::Connection::open_file` (682) sans migrations, appelle `list_thread_metadata_from_connection`. Nouveaux threads (by `thread_id`) save via `save_all`. `StatusToast` on completion. Flag `"dismissed-cross-channel-thread-import"` gate le first-run prompt.

**ACP thread import** (32-47, `ThreadImportModal`) — modal liste agents ACP disponibles, user select lesquels importer. Track `unchecked_agents`. Import via `acp_thread::AgentSessionListRequest` (214).

**Pas de formats externes** (JSON export, markdown). Que des row data SQLite ou réponses ACP live.

---

## 20. Drafts & connection store

### Drafts

`draft_prompt_store.rs`. Storage = `KeyValueStore` (SQLite KVP) namespace `"agent_draft_prompts"` (23).

- Key : `thread_id.to_key_string()` — UUID hyphenated.
- Value : JSON-sérialisé `Vec<acp::ContentBlock>` (preserve mentions resource links, pas que plain text) (45).
- Write : `cx.background_spawn` (49). Read : synchrone (28-36). Delete : à first message sent ou thread deleted (52-55).

**Display label** : `display_label_for_draft` check live in-memory editor text d'abord, fallback KVP. Mention links `[@Foo](file://...)` cleaned en bare `@Foo` (`clean_mention_links`, 95). Truncated 250 chars from first line (26, 120-131).

**Persiste à travers thread switches et process restarts**.

### Agent connection store

`AgentConnectionStore` (`agent_connection_store.rs:68`) — entité non-globale (une par panel/project) qui mappe `Agent` → `Entity<AgentConnectionEntry>`.

**States** (16-24) : `Connecting { connect_task }`, `Connected(AgentConnectedState)`, `Error { error: LoadError }`.

`request_connection` (142) **idempotent** : retourne entry existant si présent. Sinon `start_connection` retourne shared `Task<Result<AgentConnectedState, LoadError>>`, wrap dans `Connecting`, insert, spawn foreground task qui await et transition `Connected`/`Error`. Sur erreur l'entry est removed → prochain call retry.

`restart_connection` (126) : remove entry (sauf si déjà Connecting), call `request_connection`.

**Version tracking** : `watch::Receiver<Option<String>>` par connection. Quand le serveur signale nouvelle version, entry émet `NewVersionAvailable` et est removed (207-234) → UI surface un upgrade prompt.

**Pas de persistance disque** : tout en mémoire. Entries pruned quand `AgentServersUpdated` fire et un agent n'est plus dans `AgentServerStore` (240-253).

---

## 21. Keybindings & actions

Actions principales (`zed_actions/src/lib.rs:490-516`, module `agent`) :

| Action | Description |
|---|---|
| `agent::Chat` | Submit message |
| `agent::ChatWithFollow` | Submit + activate follow mode |
| `agent::SendImmediately` | Bypass queue |
| `agent::Cancel` | Stop generation |
| `agent::NewThread` | Nouveau thread |
| `agent::NewTerminalThread` | Nouveau thread terminal |
| `agent::Toggle` / `ToggleFocus` / `FocusAgent` | Toggle/focus panel |
| `agent::OpenSettings` | Settings agent |
| `agent::ToggleModelSelector` | Toggle dropdown modèle |
| `agent::AddSelectionToThread` | Quote sélection en context |
| `agent::PasteRaw` | Paste sans formatting |
| `agent::ResetAgentZoom` | Reset zoom |
| `agent::ReauthenticateAgent` | Re-auth |
| `agent::OpenSkillCreator` / `OpenRulesLibrary` | Outils |
| `agent::ToggleOptionsMenu` | Ouvre menu options |
| `agent::OpenAddContextMenu` | + menu (composer) |
| `agent::ExpandMessageEditor` | Expand input |
| `agent::ToggleThinkingMode` | Toggle thinking |
| `agent::ScrollOutput*` | Scroll (page/line/message) |
| `agent::CyclePreviousInlineAssist` / `CycleNextInlineAssist` | Cycle inline |
| `agent::ArchiveSelectedThread` / `RemoveSelectedThread` | Archive list |

Bindings Linux par défaut (`assets/keymaps/default-linux.json`) :

| Key | Action | Context |
|---|---|---|
| `enter` | `agent::Chat` | `MessageEditor` (sans use_modifier) |
| `ctrl-enter` | `agent::ChatWithFollow` | `MessageEditor` |
| `ctrl-shift-enter` | `agent::SendImmediately` | `MessageEditor` |
| `escape` | `editor::Cancel` → `MessageEditorEvent::Cancel` | `MessageEditor` |
| `ctrl-n` | `agent::NewThread` | Panel/thread list |
| `ctrl-alt-c` | `agent::OpenSettings` | Panel |
| `shift-alt-i` | `agent::ToggleOptionsMenu` | Panel |
| `ctrl-alt-/` | `agent::ToggleModelSelector` | Panel |
| `ctrl-;` | `agent::OpenAddContextMenu` | Panel |
| `ctrl->` | `agent::AddSelectionToThread` | Editor/global |
| `ctrl-shift-v` | `agent::PasteRaw` | MessageEditor |
| `shift-alt-escape` | `agent::ExpandMessageEditor` | Panel |
| `ctrl-alt-k` | `agent::ToggleThinkingMode` | Panel |
| `pageup/down` | `agent::ScrollOutputPage*` | Panel |
| `ctrl-enter` | `assistant::InlineAssist` | Editor |
| `ctrl-[` / `ctrl-]` | Cycle inline assist | `InlineAssistant` |
| `alt-y` / `ctrl-alt-y` / `ctrl-alt-z` | Keep / Reject inline | `BufferCodegen` |
| `shift-backspace` | `agent::ArchiveSelectedThread` | Thread list |
| `backspace` | `agent::RemoveSelectedThread` | Thread list |
| `ctrl-?` | `agent::ToggleFocus` | Global |

---

## 22. Streaming, scroll, follow

Détails déjà couverts §12. Récap :

- `ListState::new(0, ListAlignment::Top, px(2048.))` + `set_follow_mode(FollowMode::Tail)`.
- Indicator de génération comme item virtuel supplémentaire (`render_generating`).
- Sur send : `list_state.scroll_to_end()` explicite (1432).
- `should_be_following` track le mode user. Pendant génération, lié à `workspace.follow(CollaboratorId::Agent, ...)` (1363).
- User scroll-away : `FollowMode::Tail` détecte offset, peut pauser following. `toggle_following` action (2475). En idle, `should_be_following` source unique.
- Scroll position persistée par thread : restored from `thread.ui_scroll_position()` (`conversation_view.rs:1101`).
- Actions scroll : `ScrollOutputToTop/Bottom`, `ScrollOutputPageUp/Down`, `ScrollOutputLineUp/Down`, `ScrollOutputToNextMessage`, `ScrollOutputToPreviousMessage` (9511).

---

## 23. Erreurs, annulation, notifications

### Erreurs

Taxonomy (`conversation_view.rs:127-160`) : `PaymentRequired`, `RateLimitExceeded { provider }`, `ServerOverloaded { provider }`, `AuthenticationRequired`, `PromptTooLarge`, `NoApiKey`, `StreamError`, `InvalidApiKey`, `PermissionDenied`, `RequestFailed`, `MaxOutputTokens`, `NoModelSelected`, `ApiError`, `Refusal`, `Other { message, acp_error_code }`.

**Affichage inline** dans la liste (pas en toast) via `handle_thread_error`. Texte user-readable avec contexte ("Rate limited by Anthropic").

**Retry ACP** : event `AcpThreadEvent::Retry(retry)` → store `RetryStatus` sur `ThreadView::thread_retry_status` (1529-1534). UI render countdown / retry button.

**Inline assistant errors** : `workspace.show_toast(...)` avec `NotificationId::composite::<InlineAssistantError>(assist_id.0)` (1734-1738). Prompt editor montre l'erreur inline : `IconName::RotateCw` restart button (`inline_prompt_editor.rs:839`) ; Enter re-submit.

### Annulation

**Panel-level cancel** : `MessageEditorEvent::Cancel` émis (escape → `editor::Cancel` propagated, `message_editor.rs:1998`). `ThreadView` handle via `cancel_generation` (`thread_view.rs:978`, 1693-1698) → set `user_interrupted_generation = true` et envoie signal cancel ACP.

**Stop button visuel** (`thread_view.rs:4339-4345`) — `IconButton(IconName::Stop)`, tooltip "Stop Generation" bound à `editor::actions::Cancel`. Visible seulement pendant génération.

**Partial output** : sur interruption, contenu déjà streamé reste dans la liste. Flag `user_interrupted_generation` empêche le prochain message en queue d'auto-fire (1565-1569).

**Inline assistant stop** : `StopRequested` → `BufferCodegen` drop sa task (task cancellation via drop). Diff hunks déjà appliqués restent dans le buffer ; user accept/reject normalement.

**Terminal stop** : `TerminalCodegen::stop` set status `Done` et drop task (`terminal_codegen.rs:151-156`). Texte déjà écrit au PTY reste.

### Notifications

`ConversationView::notify_with_sound` (2547) appelé à fin de génération. Gated par `!self.agent_status_visible(window, cx)` (2627) — pas de notif si panel visible et focused dans active window.

**Popup** : `show_notification` → `pop_up` crée une nouvelle fenêtre GPUI avec `AgentNotification` view (2694, 2709). Selon `notify_when_agent_waiting` :
- `PrimaryScreen` — un popup primary display.
- `AllScreens` — un par display.
- `Never` — rien.

**Triggers** :
- Génération complète → "New message" OU "Finished running tools" (selon usage tools, 1589-1598).
- Refusal → "{model} refused to respond to this request" avec `IconName::Warning` (1613-1616).
- Error stop → "Agent stopped due to an error" avec `IconName::Warning` (1633-1638).

Audio notif (`#[cfg(feature = "audio")]`) quand panel non visible (2598-2606).

Click popup → `AgentNotificationEvent::Accepted` → focus le thread relevant.

---

## 24. Système visuel : tokens, espacements, icônes

### Couleurs (tokens `cx.theme().colors()`)

| Usage | Token |
|---|---|
| Fond panel | `panel_background` |
| Toolbar / tab bar | `tab_bar_background` |
| Fond message editor | `editor_background` |
| Activity bar | `activity_bar_bg` (computed) |
| Borders | `border`, `border_variant`, `border_focused` |
| Drop target | `drop_target_background` |
| Texte muted | `text_muted` |
| Icons désactivés | `icon_disabled`, `icon_muted` |
| Warning | `theme.status().warning` |
| Texte accent (links) | `text_accent` |
| Tool card border | `tool_card_border_color` |
| Block quote border | `block_quote_border_color` |

### Espacement (échelle Tailwind-like GPUI)

Valeurs dominantes observées :
- `p_2` (8px) — composer outer padding
- `gap_1` (4px) — entre toolbar buttons
- `gap_2` (8px) — dans message rows
- `gap_0p5` (2px) — icon+label group serré
- `px_5`, `py_1p5` — assistant message
- `py_3`, `px_2` — user message outer
- `px_2p5`, `pt_3`, `pb_1` — archive bucket headers
- `DynamicSpacing::Base04.rems(cx)` — toolbar left padding (scale avec user font-size pref)

Content centred avec `mx_auto()` capped à `max_content_width` configurable — readability sur grand écran.

### Iconographie

Toutes les icônes viennent de l'enum centralisée `IconName` (icons Zed, SVG). Icons externes : `Icon::from_external_svg(path)` — point d'extensibilité pour agents tiers.

Variantes dominantes :

| Icon | Usage |
|---|---|
| `ZedAgent` / `ZedAssistant` | Identité native |
| `Sparkle` | Agent externe générique |
| `Plus` | New thread, add context |
| `Ellipsis` | Menu options |
| `ChevronDown/Up` | Tous les selector triggers |
| `Maximize/Minimize` | Full-screen, editor expand |
| `Send` / `QueueMessage` / `Stop` | États send button |
| `Archive` / `Trash` | Gestion threads |
| `MagnifyingGlass` | Search archive |
| `Pencil` | Edit title (hover-reveal) |
| `Star` / `StarFilled` | Favoris models |
| `Check` | Selected model |
| `ArrowLeft` | Back nav overlay |
| `Terminal` | Thread type terminal |
| `FastForward` / `FastForwardOff` | Fast-mode |
| `RotateCw` | Restart inline |
| `Undo` | Restore checkpoint |
| `ToolSearch/Pencil/Terminal/Think/Web/Hammer` | Categories de tools |
| `FileMarkdown` | Open thread as markdown |
| `ForwardArrow` / `ArrowUp` | Scroll nav |
| `FastForward` | Mode fast |
| `Return` | Send/submit |

**Pattern de tailles** :
- Icon-only buttons (`IconButton`) → `IconSize::Small` par défaut.
- Chevrons et trailing indicators → `IconSize::XSmall`.
- Icon+label buttons → icon color = label color, flip `Color::Muted` ↔ `Color::Accent` sur selection/deployment.

### Typographie

- Body conversation : `agent_ui_font_size` (police UI)
- Code : `agent_buffer_font_size` (police mono)
- Line height : `buffer_font_size * 1.75`
- Heading scale : H1 1.15rem → H6 0.875rem (overrides agent-spécifiques)

---

## 25. Patterns UI à voler pour Paneflow

### 1. Toolbar context-adaptive (empty vs active)

La toolbar se **reshape complètement** selon qu'il y a des messages ou non. En empty state, toute la moitié gauche devient un gros bouton agent-picker avec icon + nom de l'agent. Une fois un thread actif, ça collapse en petite icon indicator + titre éditable inline. Affordance "getting started" sans gaspiller d'espace en état actif (`agent_panel.rs:5362-5477`).

### 2. Gradient fade + edit button hover-reveal sur titre

Un `GradientFade` 64px fade le bg toolbar sur le bord droit du titre, puis un `visible_on_hover` group reveal un bouton `IconName::Pencil` positionné. Affordance edit cachée jusqu'au besoin **sans aucun layout shift** — le fade masque l'overflow text pendant que le bouton apparaît au même endroit (4727-4756).

### 3. `HoldForDefault` modifier-key affordance

Dans les selectors (mode/model), holding ⌘ pendant le click set l'item comme défaut au lieu de juste le sélectionner. Communiqué uniquement dans le tooltip/documentation-aside via `HoldForDefault` : "Hold ⌘ to set as default" en `text_sm` `Color::Muted`, séparé par border-top. **Zéro chrome ajouté** au list item principal.

### 4. Activity bar comme floating rounded card

`rounded_t_md` + `border_1.border_b_0` + ombre subtile (`black().opacity(0.12)`, 1px offset, 2px blur). Visuellement "flotte" au-dessus du composer. Sections séparées par `Divider::horizontal()`. **Apparait seulement si contenu** — sinon 0 hauteur. Cache l'éphémère sans noise.

### 5. Send button à 3 états (pas un disabled)

Send button n'est **pas un seul bouton à disabled** — c'est **trois éléments différents** :
- Vide idle → ghost icon disabled `Muted`
- Contenu idle → `Send` filled `Accent`
- Generating vide → `Stop` rouge `Tinted(Error)`
- Generating contenu → `QueueMessage` filled `Accent` + tooltip à deux lignes expliquant "Queue and Send" vs "Send Immediately" comme actions distinctes keybindables

Évite l'ambiguïté disabled. À voler intégralement.

### 6. Code block numéroté custom pour `read_file`

Pour les sorties `read_file` (format `cat -n`), Zed parse et render avec une **vraie gutter de numéros** alignée à droite en `text_muted`, plus syntax highlighting tree-sitter du contenu, plus un bouton copy custom positionné absolu top-right visible au hover du groupe. Le bouton copy par défaut du markdown est supprimé. Le résultat semble être un mini-éditeur, pas un code block.

### 7. Mention chips comme creases

Les `@file`/`@symbol`/etc dans l'input ne sont **pas des éléments UI séparés** — ce sont des **creases inline** dans le display map de l'`Editor`. Rendu comme `ButtonLike` `Outlined` / `Tinted(Accent)` quand toggled, hauteur = `line_height - 1px`, icon `XSmall` + label buffer-font compact. Loading state = animation pulsante 2s. Click → ouvre la ref. Tooltip = preview image pour les pasted images. Pattern fondamental pour un input qui mélange texte et entités.

### 8. Thinking blocks à 4 modes d'affichage

`Auto` (collapsed + auto-expand du dernier streaming), `Preview` (collapsed mais 256px preview avec gradient fade panel_bg → transparent au top), `AlwaysExpanded`, `AlwaysCollapsed`. User toggle manuel tracké séparément (`user_toggled_thinking_blocks`). Le `Preview` avec linear-gradient est particulièrement élégant — donne envie d'expand sans bloquer la lecture.

### 9. Tool call card avec status-driven layout

Le visuel du tool call card change selon état :
- Normal → header + body collapsible (`expanded_tool_calls` tracké).
- `WaitingForConfirmation` → forcé open, header bg `tool_card_header_bg`.
- Failed/rejected → `border_dashed`.
- Edit failed + diff revealed → `DecoratedIcon` avec badge warning triangle sur l'icon.

Et un **gradient overlay 48px** linear-gradient → transparent sur le bord droit du label truncated, pour fade plutôt que ellipsis. Beau.

### 10. Permission UI inline sur la tool card

Les options "Allow Once / Allow Always / Reject" sont rendues **dans la tool card elle-même**, pas dans un modal. Variantes flat ou dropdown selon nombre d'options. Les variantes "Always allow pattern" sont auto-supprimées sur shells non-POSIX où le pattern matching serait unsafe. Pattern d'interaction in-context bien meilleur qu'un dialog.

### 11. Loop authorization avec settings live reload

`run_authorization_loop` race **user input vs settings file changes**. Si user modifie settings.json pendant prompt pending (ex : "always allow grep"), la loop re-évalue et auto-résout sans action utilisateur. Pattern réactif inhabituel.

### 12. Resume + Replay pour les threads sauvegardés

Le `DbThread` est zstd-compressé JSON sérialisé du `Thread` complet. Au resume, `NativeAgentConnection::load_thread` reconstruit puis **replay tous les push_* events** sur un nouvel `AcpThread`. Aucune logique spéciale dans l'UI pour distinguer "thread loaded" vs "thread live" — le flux est identique.

### 13. Title-as-Editor click-to-edit

Le titre du thread est un `Editor` inline, pas un label. Click → édition immédiate avec animation alpha 2s pendant que l'agent génère le titre auto. Réduit la friction de renommage à un seul clic.

### 14. Drafts persisted in KV store

Drafts sauvegardés à part dans KV store namespace `"agent_draft_prompts"`, valeur = `Vec<acp::ContentBlock>` JSON-sérialisé (préserve les mentions). Survie aux thread switches ET aux restarts du process. Le label display dans la sidebar clean les `[@Foo](file://...)` en `@Foo` simple.

### 15. Sidebar archive séparée du panel

L'historique n'est **pas dans le panel** — c'est une sidebar séparée toggleable. Buckets temporels (Today / Yesterday / This Week / Past Week / Older) avec separator rows injectés. Search inline fuzzy avec highlight positions stockés. Filter All/ArchivedOnly avec auto-reset si plus rien à montrer. Action slot droit hover-reveal (Archive / Trash / Close).

---

## 26. Recommandations concrètes pour Paneflow

Tu pars d'un seul crate `paneflow-ai-hook` — donc tu as une carte blanche relative. Voici l'ordre de bataille recommandé :

### Phase 0 — Architecture

1. **Définis un trait `AgentConnection`-like dès le départ**, même si tu n'as qu'un seul backend au début. C'est le pivot qui te permet d'ajouter Claude Code, ChatGPT, ou n'importe quoi d'autre plus tard sans refactor massif. Sépare bien :
   - **Moteur** (le truc qui appelle un LLM et exécute des outils)
   - **Display model** (`AcpThread`-like) — la liste d'entries que l'UI traverse
   - **Connection** (la glue entre les deux)

2. **Sépare en 3 stockages** :
   - Liste rapide pour sidebar (`ThreadMetadata`) — quelques colonnes typées dans SQLite.
   - Blob complet du thread (zstd JSON).
   - Drafts (KV store séparé, par thread_id).

3. **Le data flow d'un turn est une boucle, pas un appel-réponse**. Tant que le modèle émet `tool_use`, on continue le loop. Sans ça tu n'as pas un agent, tu as un chat.

### Phase 1 — UI structurale

4. **Panel = `v_flex().justify_between()`** avec 4 zones (toolbar, optional banner, message stream, composer). Dock left/right only (interdis bottom — c'est moche pour les conversations). `MIN_PANEL_WIDTH = 300px`.

5. **Toolbar context-adaptive** (pattern §25.1) — l'empty state mérite un vrai bouton "Démarrer un thread" qui prend de la place. L'active state collapse en petite icon + titre éditable.

6. **Composer en bas** avec footer bar de selectors + send button. Le composer wrap un éditeur multiline auto-height (pas un `<textarea>`). Send button = **3 éléments différents** selon état, pas un disabled (pattern §25.5).

7. **Message stream = liste virtualisée** avec follow-mode tail. Sur grand écran, cap la colonne de texte à `max_content_width` configurable + center.

### Phase 2 — Rendering

8. **Markdown avec font Agent dédiée** (séparée de l'éditeur de code). Heading scale serrée (1.15 → 0.875rem, pas 2rem). Inline code en buffer font avec fond subtil 0.08 opacity. Links accent + underline 0.5 opacity. Pas de styles "chat bubble" — l'assistant est flush sur le bg du panel, sans background.

9. **Tool call cards collapsibles** avec icon par catégorie (search/edit/terminal/think/web). Header truncated avec **gradient fade** au lieu d'ellipsis. Status driven : open forcé pendant `WaitingForConfirmation`, `border_dashed` en cas de failed.

10. **Thinking blocks à 4 modes** (§25.8). Le mode `Preview` avec gradient fade est worth implementing.

11. **Mention chips = creases dans l'éditeur**, pas des éléments UI séparés. Hauteur exacte = `line_height - 1px`. ButtonStyle Outlined par défaut, Tinted(Accent) quand toggled. Loading state = pulse 2s.

### Phase 3 — Workflows

12. **Slash commands + mentions @** dans le même completion provider. Triggers : `/` au début d'un word, `@` avec word boundary avant. Tu peux commencer avec File/Symbol et étendre.

13. **Permissions inline sur la tool card** (§25.10), pas en modal. Variantes Allow Once / Allow Always / Reject. Pour Always, persiste dans settings.

14. **Profils** comme conteneurs (tools enabled + modèle préféré + system prompt overlay). Trois built-in : write / ask / minimal. Switch profile = peut swap le modèle.

15. **Drafts** dès le départ (§25.14). Sauvegarde async sur change, lit synchrone à l'ouverture. Sinon les users perdent leur prompt sur thread switch et te haïssent.

### Phase 4 — Polish

16. **Title-as-Editor** (§25.13) — click-to-edit, animation alpha pendant l'auto-génération.

17. **Sidebar archive séparée** (§25.15) avec buckets temporels et fuzzy search. C'est ce qui transforme un chat tool en "outil de pensée long terme".

18. **Notifications de fin** si panel non visible, configurables (PrimaryScreen / AllScreens / Never). Audio en option.

19. **Cancel sur escape** dans l'input, pas un bouton Stop standalone — l'utilisateur n'a pas à viser. Stop button visible seulement pendant génération, rouge `Tinted(Error)`.

### Ce que je ne ferais PAS

- ❌ **Pas de chat bubbles** style WhatsApp. L'assistant est flush, pas de background. Ça scale beaucoup mieux pour du texte long.
- ❌ **Pas d'avatars circulaires** à côté de chaque message. Zed n'en met pas — économise du chrome.
- ❌ **Pas de timestamps individuels** par message. Ça pollue. Met-les en hover ou en footer du thread.
- ❌ **Pas de "typing indicator"** style ChatGPT (les trois dots en bubble). Le spinner inline de la liste suffit.
- ❌ **Pas de dock bottom**. C'est une conversation longue, pas un terminal.
- ❌ **Pas de tabs visibles** dans le panel pour les threads. L'historique est dans une sidebar séparée. Le panel = un seul thread actif. Évite la complexité visuelle.

---

## Annexe — Fichiers à lire en priorité dans Zed

Si tu veux aller plus loin sur un point précis :

| Sujet | Fichier prioritaire |
|---|---|
| Boucle d'un turn | `crates/agent/src/thread.rs:1979` (`run_turn`) |
| Trait connection | `crates/acp_thread/src/connection.rs:47` |
| Panel root | `crates/agent_ui/src/agent_panel.rs:870` (struct), `:5839` (render) |
| Vue conversation | `crates/agent_ui/src/conversation_view/thread_view.rs:4830` (`render_entries`) |
| Input éditeur | `crates/agent_ui/src/message_editor.rs:452` (`new`) |
| Markdown rendering | `crates/markdown/src/markdown.rs:147` (style themed) |
| Tool authorization | `crates/agent/src/thread.rs:4092` (`run_authorization_loop`) |
| Persistence schema | `crates/agent/src/db.rs:398` |
| Settings | `crates/agent_settings/src/agent_settings.rs:137` |
| Archive view | `crates/agent_ui/src/threads_archive_view.rs:140` |
| Inline assistant | `crates/agent_ui/src/inline_assistant.rs:206` (`inline_assist`) |

---

*Document généré par exploration parallèle de 5 sub-agents (architecture, UI/UX, conversation rendering, tools/MCP, composer/state) sur Zed `main` @ 2026-05-23.*

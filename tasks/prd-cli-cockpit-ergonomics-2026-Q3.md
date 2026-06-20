[PRD]
# PRD: Cli Cockpit - Ergonomics & Fleet Awareness (2026-Q3)

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-06-10 | Arthur Jean | Draft initial - 6 epics / 19 stories : steering in-app (Composer, Broadcast Groups), triage & launch (Attention Queue, Launch Pad), Command Outcome Marks OSC 133 + export, AgentState enrichi (Errored/Stalled), fleet observability (Fleet Bar, identity pill, ports par pane, idle-dim), scale ergonomics (match-rail, fleet grep, font zoom) |
| 1.1 | 2026-06-10 | Arthur Jean | Passe de review adverse (3 reviewers) : US-010 Errored promu P0 ; défauts clavier revus (zéro shadow readline/TUI : Composer secondary-shift-space, jump-to-prompt secondary-shift-up/down) ; anatomie du tab unifiée (EP-005 + FR-11) ; Q3/Q4 tranchées dans le PRD ; dépendances purgées (US-009/016/018) ; EP-003 DoD rescopée ; ACs unhappy-path ajoutés (US-013/016/019) ; Stalled défaut ON 300 s ; US-005 v1 new-branch-only ; phases datées |

## Problem Statement

1. **Steerer un agent reste un side-channel CLI.** Le prefill mécanique existe (`send_text` sans `\r`, `view.rs:628` ; `paneflow send`, settle-poll `output_generation` `ipc_handler.rs:854`) mais aucune surface in-app : pour injecter un prompt il faut cliquer dans la pane et taper à l'aveugle, et un paste multi-ligne hors bracketed-paste soumet à la première newline (`\n` -> `\r`, `input.rs:843`). Envoyer la même consigne à N agents = N copier-coller, ou `send --broadcast` phase-blind depuis un terminal externe (`send_cmd.rs:72`) qui peut corrompre le stdin d'un agent en pleine génération.
2. **Le triage des agents bloqués est un tour de workspaces.** `JumpNextWaiting` (`focus.rs:90`) saute en aveugle : aucune vue d'ensemble "qui attend quoi depuis combien de temps". La question sanitizée existe (`AgentSession.message`, `ai_types.rs:90`) mais n'est lisible qu'en atteignant physiquement chaque pane (peek overlay, US-020 orchestration-v2). Friction n°1 du marché multi-agents : « constantly jumping between terminals to see which session needed input » ([HN ccmux](https://news.ycombinator.com/item?id=47223142)).
3. **Le rituel worktree+agent reste TOML/CLI-only.** Le moteur est livré (worktree-per-agent, copie `.env`, `${port_offset}`, teardown - EP-002 orchestration-v2) mais le geste unique in-app n'existe pas : éditer un TOML et relancer `paneflow up` pour chaque agent isolé, là où un modal ferait worktree + split + launch + prefill en un raccourci ([dev.to/rohansx](https://dev.to/rohansx/every-ai-agent-tool-creates-git-worktrees-none-of-them-make-worktrees-actually-work-3ae9)).
4. **Le scrollback d'un run d'agent est un mur plat.** Aucune frontière de commande : retrouver LA commande qui a échoué dans 2 000 lignes = deviner une string à chercher. Ghostty/WezTerm/kitty/iTerm2/VS Code ont tous OSC 133 ; alacritty_terminal 0.26 droppe ces OSC silencieusement (vendored `event_loop.rs:154`, enum `Event` sans variante OSC) et l'ancien byte-scanner Paneflow a été retiré (`pty_session.rs:269`).
5. **Un agent qui a planté et un agent qui a fini sont indistinguables.** `AgentState` = {Thinking, WaitingForInput, Finished} (`ai_types.rs:67-76`) : un exit code 1 et un turn réussi donnent le même état, la même notif « agent finished » (`ipc_handler.rs:95`). Un agent silencieusement bloqué reste « Thinking » pour toujours. `ChildExit` ne porte que l'exit du shell, pas de l'agent (`pty_session.rs:837-854`).
6. **Aucune vue flotte, aucune identité par pane.** L'agrégat agents est par-workspace dans la sidebar (`sidebar/mod.rs:570-576`), les ports sont par-workspace (`ws.active_ports`, `event_handlers.rs:851`), la détection AI par PID est par-workspace (`ports.rs:307`). À 4-8 agents le dev tient la topologie de tête : quel agent dans quelle pane, quel port à qui, qui tourne/attend/a planté - tout cross-workspace est invisible.

**Why now :** orchestration-v2 vient de livrer le socle complet (message stocké US-016, mapping PID->surface US-017, glow US-018, jump US-019, peek US-020, moteur worktree EP-002). C'est la fenêtre pour transformer ces primitives en ergonomie quotidienne avant que Warp/cmux ne descendent leurs cockpits en local cross-platform. Paneflow est le seul outil qui possède à la fois l'émulateur VT et le modèle d'état des agents - les wrappers tmux scrapent `capture-pane` à la regex, structurellement incapables de faire mieux.

## Overview

Six chantiers sur la surface cockpit du mode Cli uniquement - ni Diff ni Agents view :

**EP-001 - Steering.** Le Composer (secondary-shift-space) : barre de prompt multi-ligne ancrée sous la pane focus, livraison bracketed-paste, jamais d'auto-submit, consciente de l'état agent. Les Broadcast Groups : panes taguées en groupe nommé (liseré coloré), un prompt tapé une fois pré-rempli dans chaque membre - seulement les membres safe (WaitingForInput/Finished), buffer pour les Thinking jusqu'à leur transition.

**EP-002 - Triage & launch.** L'Attention Queue (Ctrl+Shift+K) : overlay listant chaque pane en WaitingForInput cross-workspace avec question sanitizée + temps d'attente + focus en un Enter. Le Launch Pad (Ctrl+Shift+L) : modal agent/branche/prompt qui enchaîne worktree off-thread + copie `.env` + split + launch CLI + prefill - le moteur orchestration-v2 derrière un geste unique.

**EP-003 - Command Outcome Marks.** Reconnaissance OSC 133 A/B/C/D via un tap byte-stream pré-parse (fork minimal d'alacritty_terminal - décision rendue par le spike en tête de US-006, cadrée en Open Question Q1), snippet shell opt-in (extension de `setup_shell_integration`, `shell.rs:268`), pastilles exit-code dans la marge, jump-to-prompt clavier, et l'export structuré CommandBlock (markdown/JSON) en P2.

**EP-004 - AgentState enrichi.** `Errored` (P0) piloté par le shim (qui wrappe le binaire agent et connaît son vrai exit status, `exec.rs:184`) via une nouvelle frame `ai.exit` ; `Stalled` via timestamp d'activité + sweep existant. Corps de notification distincts par état.

**EP-005 - Fleet observability.** Socle de scan par-pane (désagrégation du pool PID plat), pill d'identité agent par pane persistée en session, badges ports par pane + détection collision, Fleet Bar agrégat cross-workspace, et idle-dim opt-in (P2, gated par Q2). L'epic définit l'anatomie du tab (slots, priorités, repli) que toute story d'adornment référence.

**EP-006 - Scale ergonomics.** Match-rail sur la scrollbar (ticks proportionnels click-to-jump), fleet grep (une regex sur toutes les panes live), font zoom par pane.

Décisions clés : human-in-loop strict (le Composer pré-remplit ; la soumission est un geste humain distinct et documenté, jamais un défaut - tranché, ex-Q3) ; `Errored` hook-driven et non ChildExit (l'exit du shell n'est pas l'exit de l'agent) ; marks OSC 133 session-local en v1 (le restore strip les OSC, `pty_session.rs:1370`) ; aucun raccourci par défaut ne shadow un chord shell/readline/TUI courant ; réutilisation systématique des scaffolds existants (theme_picker pour les overlays, TextArea pour la saisie, ScrollbarMetrics pour le rail, moteur worktree pour le Launch Pad). Ordre d'implémentation recommandé en Phase 1 : US-004 -> US-001 -> US-005 -> US-010 -> US-002/003 -> EP-003 (les quick wins sur scaffolds livrés d'abord, le chantier fork-bearing en dernier).

## Goals

Phase 1 = stories P0 (Tier 1 + Errored), cible ≤ 4 semaines après kickoff. Phase 2 = stories P1/P2 (Tier 2), cible ≤ 8 semaines après kickoff.

| Goal | Phase-1 Target (P0) | Phase-2 Target (P1/P2) |
|------|---------------------|------------------------|
| Triager N agents bloqués sans tour de workspaces | Attention Queue : état + question + focus en 1 raccourci, < 2 s pour choisir | Fleet Bar : compte cross-workspace permanent, click-to-warp par état |
| Steerer 1..N agents depuis le cockpit | Composer + Broadcast Groups : 0 copier-coller multi-pane, 0 soumission accidentelle multi-ligne, 0 corruption stdin sur 20 broadcasts mixtes (test scripté) | Buffer étendu au Composer single-pane vérifié en dogfooding |
| Réduire le setup d'un agent isolé | Launch Pad : 1 raccourci + 1 formulaire (vs édition TOML + CLI), < 10 s jusqu'au prompt pré-rempli | - |
| Naviguer un run d'agent par commandes | Marks 133 : jump-to-prompt + exit-dots sur les 4 shells supportés | Export CommandBlock markdown/JSON (presse-papier) |
| Distinguer fini / planté | Errored : notif + ring distincts, 0 faux « finished » sur exit non-zéro (agents via PATH shimé) | Stalled : signal silence ON par défaut (300 s), 1 notif/épisode |
| Identité et ressources par pane | - | Pill agent + badges ports par pane, persistés ; collision port signalée |

## Target Users

### Le dev orchestrateur (Arthur et profils similaires)
- **Role :** solo dev / indie maker qui pilote 3-8 agents CLI (Claude Code, Codex, OpenCode...) en parallèle dans la grille Cli.
- **Behaviors :** lance via `paneflow up` ou la tab bar, supervise la grille, steere au fil de l'eau, review dans les panes.
- **Pain points :** taper/coller dans la bonne pane à l'aveugle ; copier-coller la même consigne N fois ; scanner la grille pour trouver qui attend ; éditer un TOML pour chaque agent isolé ; scroller 2 000 lignes pour trouver l'échec ; confondre un agent planté avec un agent fini.
- **Current workaround :** `paneflow send` depuis un terminal externe, tour manuel des workspaces, TOML + `paneflow up`, Ctrl+Shift+F avec une string devinée, clic dans chaque pane pour lire l'état réel.
- **Success looks like :** ne jamais quitter le cockpit : steerer, triager, lancer et naviguer au clavier, n'être interrompu que par des notifications qui disent vraiment ce qui s'est passé.

### Le power-user terminal (sans agents)
- **Role :** dev qui utilise Paneflow comme multiplexer quotidien (builds, dev servers, shells).
- **Behaviors :** splits, workspaces par projet, recherche scrollback, thèmes.
- **Pain points :** pas de frontières de commande dans le scrollback ; recherche match-par-match sans carte spatiale ; taille de police globale (impossible de rétrécir les panes de monitoring).
- **Current workaround :** recherche texte + scroll manuel ; config font globale.
- **Success looks like :** jump-to-prompt, exit-dots, match-rail et font zoom par pane fonctionnent sans aucun agent - valeur terminal pure.

## Research Findings

### Competitive Context
- **Warp** : blocks + command palette, mais modèle single-session, closed-source, pas de topologie multi-agents. Le block model est le canon ; Paneflow l'aborde par OSC 133 + le modèle d'état agents que Warp n'a pas.
- **Ghostty / WezTerm / kitty / iTerm2 / VS Code** : OSC 133 natif (jump-to-prompt, exit marks) - table-stakes que Paneflow n'a pas, aucun ne le lie à un modèle d'agents.
- **cmux** : per-pane font zoom (Cmd+=/-/0), identité worktree/branche par pane - macOS-only, Swift, non scriptable.
- **tmux + wrappers (Harness, Amux, ccmux)** : broadcast `synchronize-panes` binaire et phase-blind ; état agent scrapé par regex sur `capture-pane` (fragile). Structurellement incapables de busy-aware ou d'état push-based.
- **GitHub Agent HQ** : queue d'approbations cross-agents - cloud/enterprise ; aucun équivalent OSS local.
- **Market gap :** aucun terminal OSS cross-platform ne combine émulateur possédé + état agent push-based. Le busy-aware broadcast, l'attention queue locale et les exit-marks liés à l'état agent sont du territoire vierge.

### Best Practices Applied
- OSC 133 émis par snippet shell opt-in (`PS0`/`precmd`/`preexec` bash-zsh, `fish_prompt`, PSReadLine pwsh) - le standard de facto (FinalTerm -> iTerm2 -> Ghostty/WezTerm).
- Broadcast gated par l'état du destinataire (livrer seulement quand le stdin est sûr) - leçon des corruptions `synchronize-panes` documentées.
- Notifications sémantiques distinctes par cause (fini / erreur / bloqué / silencieux) plutôt que bell-on-everything - anti notification-fatigue.
- Défauts clavier qui ne volent jamais les chords shell/readline/TUI courants (Ctrl+K, Ctrl+flèches, Ctrl+-) - la règle Ghostty/kitty : jamais de bare Ctrl+lettre en contexte terminal.
- Jamais de travail bloquant sur le render-thread GPUI : subprocess via `paneflow_process::run_with_timeout` + `smol::unblock` (audit 2026-06-04).
- Texte terminal = UNTRUSTED : sanitization 512 chars + strip bidi/zero-width par le chemin existant (`ipc_handler.rs:116`), jamais interprété.

## Assumptions & Constraints

### Assumptions (to validate)
- Le fork minimal d'alacritty_terminal 0.26 (callback passthrough pré-parse) est maintenable à coût marginal faible - à valider par le spike de US-006 (et l'upstream récent peut l'avoir rendu inutile, Q1).
- Le shim voit la terminaison du binaire agent dans les cas dominants (claude/codex lancés via PATH shimé) - les agents lancés en bypassant le PATH (chemin absolu) n'émettront pas `ai.exit` ; dégradation : état Finished comme aujourd'hui.
- Les snippets shell OSC 133 n'entrent pas en conflit avec les frameworks de prompt usuels (starship, oh-my-zsh, p10k) - à vérifier sur les 4 shells pendant US-007.
- Un seuil de silence par défaut de 300 s donne un signal Stalled utile sans noyer de faux positifs (long thinking légitime) - dedup par épisode + seuil configurable + désactivable pour mitiger.

### Hard Constraints
- **Cross-platform Linux/macOS/Windows obligatoire** (CLAUDE.md) : chaque story a un chemin par OS ou un stub documenté cohérent avec l'existant (notif Windows = stub `ipc_handler.rs:188-193`, ports/AI-detect Windows = stubs `ports.rs:283/:371`).
- **Human-in-loop** : aucune action IA n'auto-soumet. Le Composer pré-remplit par défaut (pas de `\r`) ; la soumission est un geste humain explicite, distinct et documenté (parité sémantique `--submit`). Le flush du buffer broadcast ne fait que pré-remplir, jamais soumettre.
- **Aucun raccourci par défaut en contexte Global/Terminal ne shadow un chord shell/readline/TUI courant** (Ctrl+K kill-line, Ctrl+flèches navigation TUI, Cmd+K clear-buffer macOS). Exception documentée et remappable uniquement (US-019 : convention zoom des terminaux Linux assumée).
- **Scope Cli cockpit uniquement** : aucun changement dans `src-app/src/diff/` ni `src-app/src/agents_view/`. La TitleBar reste push-only (`title_bar.rs:34-44`).
- **Texte terminal/agent = UNTRUSTED** (FR-08 orchestration-v2) : affiché verbatim, sanitizé par les chemins existants, jamais interprété. Les champs relus de session.json sont validés à l'ingress (parité US-057/EP-010).
- `MAX_PANES = 32`, `MAX_WORKSPACES = 20` (`limits.rs`) respectés (Launch Pad échoue proprement à la limite).
- Jamais d'I/O bloquant sur le render-thread : git/FS/scan via `smol::unblock` + `paneflow_process::run_with_timeout` (`lib.rs:79`).
- Le slot border de la pane (`pane.rs:1744-1745`) appartient au glow attention - le liseré de groupe utilise un élément distinct.
- La règle visuelle « amplifier l'actif, jamais dégrader l'inactif » (US-018 orchestration-v2) reste l'invariant par défaut - l'idle-dim est opt-in OFF et gated par Q2.
- Le budget taille du shim est gardé par un test de cap (re-baseline justifié dans le commit si `ai.exit` le dépasse, parité 8c8dc36).
- Convention commits : `feat(module): US-NNN - description`, atomiques par story.

## Quality Gates

These commands must pass for every user story:
- `cargo fmt --check` - formatage canonique (gate CI release, 4 legs)
- `cargo clippy --workspace -- -D warnings` - zéro warning
- `cargo test --workspace` - tous les tests workspace

Pour les stories UI (US-001, US-002, US-003, US-004, US-005, US-008, US-010, US-013, US-014, US-015, US-016, US-017, US-018, US-019) :
- Vérification visuelle manuelle dans l'app (GPUI non testable headless) - noter « UI non vérifiée GUI » dans le commit si la passe visuelle n'a pas eu lieu.

## Epics & User Stories

### EP-001: Steering - saisie pilotée par l'état agent

Faire du cockpit la surface de saisie : un composer par pane et un broadcast par groupe, tous deux conscients de l'état agent pour ne jamais corrompre un stdin en pleine génération.

**Definition of Done:** un prompt multi-ligne se rédige dans le Composer et atterrit pré-rempli (jamais soumis par défaut) dans la pane focus ou dans chaque membre safe d'un groupe, les membres occupés le recevant à leur prochaine transition, le tout sans quitter le clavier.

#### US-001: Pre-fill Composer (secondary-shift-space)
**Description:** As a dev orchestrateur, I want une barre de prompt multi-ligne ancrée sous la pane focus qui pré-remplit l'input de l'agent en bracketed-paste so that je steere sans cliquer dans la pane ni risquer une soumission à la première newline.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given le focus sur une pane terminal, when `secondary-shift-space` (libre, aucun shadow readline/TUI - `secondary-k` écarté : Ctrl+K = kill-line, Cmd+K = clear-buffer macOS), then un overlay s'ancre au bord bas de la pane focus avec un champ multi-ligne (réutilise `TextArea`, `widgets/text_area.rs:209` ; `register_keybindings` appelé au bootstrap, `:112`) et prend le focus
- [ ] Given un texte multi-ligne dans le Composer, when je valide (Enter), then le texte est livré via le chemin bracketed-paste (`write_paste_text`, `input.rs:833`) - les newlines restent littérales, aucun `\r` ajouté, l'overlay se ferme et le focus revient au terminal
- [ ] Given la livraison par défaut, then AUCUNE soumission : le prompt est visible dans l'input de l'agent, c'est l'humain qui soumet dans le terminal (invariant `send_text`, `view.rs:628`)
- [ ] Given un geste explicite distinct (`secondary-enter` dans le Composer), when déclenché, then le texte est livré PUIS soumis (`\r` séparé) - geste humain documenté dans le tooltip, jamais le défaut (décision actée : équivalent moral de taper Enter dans le terminal, safeguards = distinct + tooltip + interdit en broadcast)
- [ ] Given la session mappée à la pane focus est `Thinking` (`AgentSession.state` via `surface_id`, `ai_types.rs:79-96`), when le Composer est ouvert, then un chip d'état l'indique et la validation est bloquée (message « agent en cours de génération ») - comportement v1, remplacé par le buffer unifié à la livraison de US-003 ; `Shift+Enter` insère une newline, `Escape` ferme sans rien envoyer
- [ ] Given une pane sans session agent (shell nu), when validation, then la livraison fonctionne à l'identique (le gate n'exige pas d'agent - terminal pur)
- [ ] Given la pane focus fermée pendant la rédaction, when validation, then no-op propre (pas de panic, overlay fermé) - pattern WeakEntity du prefill (`ipc_handler.rs:854`)
- [ ] L'overlay suit le modèle theme_picker (FocusHandle dédié + `handle_*_key_down` + backdrop `deferred`, `theme_picker.rs:43,75,240`) ; le terminal ne reçoit aucune frappe pendant l'édition

#### US-002: Broadcast Groups - modèle, assignation, liseré
**Description:** As a dev orchestrateur, I want taguer des panes dans un groupe nommé avec un liseré coloré partagé so that la cible d'un broadcast soit explicite et visible d'un coup d'oeil avant tout envoi.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given une pane focus, when j'invoque l'action `ToggleBroadcastMember` (action + binding remappable, triptyque `actions!`/`ActionMeta`/`DefaultBinding`), then la pane rejoint/quitte le groupe actif ; un picker (scaffold theme_picker) permet de créer/renommer/choisir le groupe actif
- [ ] Given une pane membre d'un groupe, when rendu, then un liseré vertical 3 px de la couleur du groupe s'affiche sur son bord gauche via un élément DISTINCT du border attention (`pane.rs:1744-1745` reste au glow) - enfant left-edge dans le flex de la pane
- [ ] Les couleurs de groupe sont 8 slots `UiColors` nommés (`group_1..group_8`, précédent `vc_*`, `theme/model.rs:298-340`), lisibles sur One Dark et PaneFlow Light - pas de hex inline ; à 8 groupes existants, la création d'un 9e est refusée avec message explicite
- [ ] Given une pane fermée, when le groupe est relu, then le membre disparaît silencieusement (pas d'entrée fantôme) ; un groupe vide reste valide
- [ ] Les groupes vivent sur `PaneFlowApp` (état single-thread, conventions GPUI - pas d'Arc/Mutex) et référencent les terminaux par `EntityId` ; une pane appartient à au plus un groupe (v1, documenté)
- [ ] Given 0 groupe défini, when j'ouvre le picker, then un état vide explicite propose la création (pas de liste vide muette)

#### US-003: Broadcast busy-aware depuis le Composer
**Description:** As a dev orchestrateur, I want taper un prompt une fois et le pré-remplir dans chaque membre du groupe réellement prêt à le recevoir so that je steere N agents d'un geste sans jamais corrompre le stdin d'un agent en génération.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001, US-002

**Acceptance Criteria:**
- [ ] Given un groupe actif avec ≥ 1 membre, when le Composer est ouvert en mode broadcast (toggle dans le Composer, indicateur « groupe : N membres »), then la validation pré-remplit (bracketed-paste, jamais de `\r`) chaque membre dont la session est `WaitingForInput`/`Finished` ou sans session agent
- [ ] Given un membre `Thinking`, when broadcast, then le texte est mis en buffer pour ce membre et livré (pré-rempli uniquement) à sa prochaine transition hors `Thinking` (flush dans `upsert_session_state`, main thread sérialisé, `ipc_handler.rs:1963`) - jamais de soumission au flush
- [ ] Le même buffer s'applique au Composer single-pane : viser une pane `Thinking` bufferise au lieu de bloquer (remplace le blocage v1 de US-001, même mécanique, même indicateur - sémantique unifiée)
- [ ] Given un buffer en attente sur une pane, when rendu, then son tab porte un indicateur discret (« 1 en attente », slot anatomie du tab EP-005) ; un nouveau broadcast vers la même pane REMPLACE le buffer (latest-wins, documenté) ; le buffer est annulable depuis le Composer (mode broadcast) ou le menu contextuel de la pane
- [ ] Given une pane membre fermée avant le flush, when transition, then le buffer est jeté silencieusement (pas de panic, pas de livraison orpheline)
- [ ] Given le mode broadcast, when la validation part, then un récapitulatif transitoire (auto-dismiss 4 s) indique livrés/bufferisés (ex. « 3 livrés, 2 en attente ») - jamais d'envoi silencieux
- [ ] Le geste de soumission explicite (`secondary-enter`) est INDISPONIBLE en mode broadcast (v1 : le broadcast ne soumet jamais, même explicitement - réduit le rayon d'accident)

---

### EP-002: Triage & launch - le cockpit en un geste

Compresser les deux boucles les plus coûteuses : « qui m'attend » (queue ordonnée) et « lancer un agent isolé » (modal worktree+launch).

**Definition of Done:** depuis n'importe où, Ctrl+Shift+K affiche la file ordonnée des agents bloqués avec leurs questions et téléporte sur Enter ; Ctrl+Shift+L crée worktree + pane + agent + prompt pré-rempli en un formulaire, sans TOML.

#### US-004: Attention Queue (Ctrl+Shift+K) + `waiting_since`
**Description:** As a dev orchestrateur, I want une file ordonnée de toutes les panes en attente d'input cross-workspace, avec la question de chaque agent et son temps d'attente so that je triage N prompts d'approbation en un raccourci au lieu d'un tour de workspaces.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `AgentSession` gagne `waiting_since: Option<Instant>` (`ai_types.rs:79-96`), stampé à la transition vers `WaitingForInput` (`ipc_handler.rs:1717-1730`), cleared sur toute autre transition - net-new, vérifié absent
- [ ] Given ≥ 1 session `WaitingForInput` avec `surface_id` résolu, when `secondary-shift-k` (libre - `secondary-shift-j` reste `jump_next_waiting`, `defaults.rs:100`), then un overlay (scaffold theme_picker) liste chaque pane en attente : outil + workspace + question sanitizée (`message`, déjà 512 chars bidi-stripped, `ipc_handler.rs:116`) + temps d'attente relatif, triée par attente décroissante
- [ ] Given la file ouverte, when Enter (ou clic) sur une entrée, then focus de la pane cible (réutilise la mécanique `handle_jump_next_waiting` : switch workspace, activation du tab caché, focus - `focus.rs:90`), overlay fermé
- [ ] Given une session en attente SANS `surface_id` résolu, then elle apparaît en fin de liste, marquée non-navigable (cohérence US-019 orchestration-v2 : la nav exige le mapping)
- [ ] Given 0 session en attente, when raccourci, then un état vide explicite (« aucun agent n'attend ») se ferme sur Escape - pas de no-op muet
- [ ] Given une session qui se débloque pendant que la file est ouverte, when l'event hook arrive, then la ligne disparaît au prochain repaint (la file lit l'état live, pas un snapshot)
- [ ] Given une entrée dont la pane a été fermée entre le rendu et l'Enter, when activation, then no-op propre + retrait de la ligne (pas de panic)
- [ ] La question affichée est du texte inerte : pas de liens, pas d'interprétation ANSI (untrusted)

#### US-005: Launch Pad (Ctrl+Shift+L) - worktree + agent + prompt en un geste
**Description:** As a dev orchestrateur, I want un modal agent/branche/prompt qui crée le worktree, copie les `.env`, splitte une pane au bon path, lance la CLI choisie et pré-remplit mon prompt so that le rituel 6 étapes par agent isolé devienne un geste unique - sans éditer de TOML.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given le mode Cli, when `secondary-shift-l` (libre, vérifié), then un modal (scaffold `custom_buttons_modal.rs:87` ModalView::Form) collecte : agent (picker sur `TerminalAgent::ALL`, 16 entrées, `agent_launcher.rs:49`, les non-installés grisés via `is_installed()` `:278`), nom de NOUVELLE branche (champ texte - le picker de branche existante et l'option « nouveau workspace » sont des follow-ups explicites hors v1), prompt (TextArea multi-ligne)
- [ ] Given un repo git détecté pour le workspace courant, when je confirme, then la création du worktree s'exécute HORS render-thread (`smol::unblock` + `worktree::add_worktree`, `worktree.rs:197`, deadline 120 s `:29`) en sibling `<repo>.worktrees/<branche>` (convention EP-002 orchestration-v2), puis `copy_env_files` (`:243`, no-clobber) - AUCUNE réimplémentation du moteur : réutilisation stricte
- [ ] Given le worktree créé, then la pane focus est splittée (direction du preset actif, fallback Vertical, ratio 0.5) au path du worktree, la CLI agent lancée (`launch_command`, `agent_launcher.rs:320`, qui honore `claude_code_bypass_permissions` `:287`) et le prompt pré-rempli via le settle-poll existant (`schedule_prompt_prefill`, `ipc_handler.rs:854`) - jamais soumis
- [ ] Given l'échec de la création du worktree (branche verrouillée, git absent, timeout), when retour, then le modal affiche l'erreur verbatim et AUCUNE pane n'est créée (atomicité : pas de pane orpheline sans worktree)
- [ ] Given le worktree créé, then il est enregistré comme `ManagedWorktree` (`worktree.rs:65`) dans le workspace (parité teardown avec `paneflow up` - le Launch Pad ne crée pas une 2e population de worktrees)
- [ ] Given `MAX_PANES` (32) atteint ou cwd sans repo git, when confirmation, then erreur explicite dans le modal, rien n'est exécuté
- [ ] Given un nom de branche invalide pour git, when confirmation, then l'erreur git est montrée verbatim (pas de validation maison divergente de git)
- [ ] Le modal est annulable à tout moment avant confirmation (Escape) sans effet de bord ; pendant l'exécution, un état « création... » désactive la re-soumission (pas de double worktree)

---

### EP-003: Command Outcome Marks - OSC 133 + export

Donner des frontières de commande au scrollback : le primitive block que tous les terminaux modernes ont, lié ici au cockpit agents. Scanner pré-parse + snippet shell opt-in + navigation + export.

**Definition of Done:** avec le snippet installé, chaque commande porte sa pastille exit-code dans la marge et secondary-shift-up/down saute de prompt en prompt dans le scrollback (US-006/007/008) ; sans snippet, coût zéro et zéro régression. Stretch P2 : l'export des derniers blocs (cmd/output/exit/durée) en markdown ou JSON dans le presse-papier (US-009, sacrifiable sans bloquer l'epic).

#### US-006: Tap byte-stream pré-parse + store de marks
**Description:** As a power-user terminal, I want que Paneflow reconnaisse les séquences OSC 133 A/B/C/D du flux PTY so that les frontières et exit codes de commandes existent comme données structurées par terminal.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] SPIKE bloquant en tête de story (résout Q1) : vérifier si alacritty_terminal > 0.26 (crates.io) expose OSC 133 ou un hook pré-parse ; si oui, bump de la dépendance au lieu du fork ; sinon, fork minimal pinné (un seul changement : callback `&[u8]` avant `parser.advance`, vendored `event_loop.rs:154`) - décision tracée dans le commit et reportée dans Q1
- [ ] Given le tap actif, when des séquences `ESC]133;A|B|C|D[;params]ST|BEL` traversent le flux, then un `CommandMark { abs_line, kind, exit_code: Option<i32>, at }` est enregistré dans un ring borné par terminal (cap documenté, ex. 1 000 marks - drop oldest) keyé sur le compteur de lignes absolu du grid
- [ ] Given un terminal SANS snippet shell (aucun OSC 133 entrant), then le scanner ne fait AUCUNE allocation par chunk et le débit PTY ne régresse pas de plus de 5 % sur un `cat` de 100 Mo (mesuré, parité runbook heaptrack)
- [ ] Given des payloads OSC 133 malformés ou hostiles (exit code non numérique, params géants), when parse, then ils sont ignorés sans panic ni allocation non bornée (le flux PTY est untrusted)
- [ ] Given un trim du scrollback (cap `MAX_CHARS`, `cap_scrollback_at_char_boundary` `pty_session.rs:1197`), when des lignes sortent de l'historique, then les marks correspondants sont purgés (jamais de mark pointant hors grid)
- [ ] Les marks sont session-local : non persistés dans session.json en v1 (le restore strip les OSC, `pty_session.rs:1370` - documenté en Non-Goal)
- [ ] Le tap est du traitement d'octets platform-neutre : chemin identique sur Linux/macOS/Windows (aucune API OS)
- [ ] Tests unitaires du state-machine de scan : séquences scindées entre deux chunks de lecture, ST vs BEL, interleaving avec du contenu binaire

#### US-007: Snippet shell-integration OSC 133 (zsh, bash, fish, pwsh)
**Description:** As a power-user terminal, I want un snippet opt-in qui fait émettre OSC 133 par mon shell so that les marks existent sans que je configure quoi que ce soit à la main.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Les 4 constantes rc existantes (`ZSH_OSC7` `shell.rs:24`, `BASH_OSC7` `:51`, `FISH_OSC7` `:71`, `PWSH_OSC7` `:90`) sont étendues pour émettre `133;A` (prompt), `133;C` (pré-exec) et `133;D;$?` (fin de commande + exit) via les hooks idiomatiques par shell (`precmd`/`preexec` zsh, `PROMPT_COMMAND`/`PS0` bash, `fish_prompt`/`fish_preexec`, PSReadLine pwsh) - distribution par le chemin `setup_shell_integration` déjà livré (`shell.rs:268`)
- [ ] Given un shell dont l'utilisateur a son propre framework de prompt (starship, p10k), when le snippet s'installe, then il chaîne les hooks existants au lieu de les écraser (append, pas replace) - vérifié manuellement sur zsh+starship au minimum
- [ ] Given l'intégration shell désactivée (config), then aucun snippet n'est injecté et le terminal se comporte exactement comme avant (opt-out propre)
- [ ] Given un shell non supporté (autre que les 4), then aucun snippet, aucun message d'erreur récurrent - dégradation silencieuse documentée
- [ ] Le snippet n'émet aucune séquence quand le shell n'est pas interactif (scripts, subshells non-tty)
- [ ] Note : livrable indépendamment de US-006 - les séquences émises sans tap sont droppées par le parser (comportement actuel), zéro régression

#### US-008: Exit-dots dans la marge + jump-to-prompt (secondary-shift-up/down)
**Description:** As a dev orchestrateur, I want une pastille verte/rouge par commande et un saut clavier de prompt en prompt so that je trouve LA commande qui a échoué dans un run d'agent de 2 000 lignes sans deviner une string à chercher.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-006, US-007

**Acceptance Criteria:**
- [ ] Given des marks `D` avec exit code dans le viewport, when paint, then une pastille (vert exit 0 / rouge non-zéro, slots `UiColors`) est peinte dans la marge gauche du contenu (origin padding, `geometry.rs:14`) sur la ligne du prompt correspondant - paint_quad, aucun reflow du texte
- [ ] Given le focus terminal, when `secondary-shift-up` / `secondary-shift-down` (libres, convention iTerm2 - `ctrl-up/down` écartés : ils voleraient la navigation des TUI vim/nano via CSI 1;5A/B), then le viewport saute au mark `A` précédent/suivant (réutilise la math de `scroll_to_match`, `search.rs:156-180`) ; aux extrémités, no-op silencieux ; bindings remappables via le registre existant
- [ ] Given un terminal sans marks (pas de snippet), when jump, then no-op silencieux - pas d'erreur, pas de toast
- [ ] Given un resize/reflow du grid, when repaint, then les pastilles restent sur leurs lignes de prompt (ancrage au compteur absolu, tolérance documentée pour le reflow v1)
- [ ] Given un hover sur une pastille, then un tooltip montre `exit <code>` + horodatage relatif
- [ ] Vérification visuelle manuelle sur One Dark et PaneFlow Light (contraste pastilles)

#### US-009: Export CommandBlock (markdown / JSON)
**Description:** As a dev orchestrateur, I want exporter les N derniers blocs (commande, output, exit, durée) d'une pane en markdown ou JSON dans le presse-papier so that « ce que l'agent a réellement exécuté » atterrisse dans une PR description ou un audit sans copier-coller manuel.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-006, US-007

**Acceptance Criteria:**
- [ ] Given des marks A/C/D présents, when l'action `ExportBlocks` (palette/raccourci remappable, contexte Terminal), then les ≤ 50 derniers blocs complets sont assemblés : `cmd_line` (texte entre A et C), `output` (texte entre C et D, extrait du grid via le chemin `bounds_to_string` existant), `exit_code`, `duration` (delta horodatages C->D)
- [ ] Given l'export markdown, then chaque bloc est une fence annotée (commande en titre, exit + durée en métadonnées, output dans la fence) ; l'export JSON est un tableau d'objets - les deux atterrissent dans le presse-papier via le chemin existant (`input.rs:765`)
- [ ] Given un output volumineux, then l'export est borné (cap documenté par bloc + cap total, ex. 256 KiB) avec troncature marquée `[truncated]` - jamais d'alloc non bornée
- [ ] Given aucun mark dans la pane, when action, then message d'état explicite (« shell integration requise ») - pas d'export vide silencieux
- [ ] Le contenu exporté est du texte brut sanitizé (escapes strippées par les chemins existants) - untrusted, jamais ré-interprété

---

### EP-004: AgentState enrichi - Errored & Stalled

Casser l'ambiguïté du `Finished` unique : un agent qui plante, un agent qui finit et un agent qui se fige deviennent trois signaux distincts, routés en notifications distinctes.

**Definition of Done:** un exit non-zéro du binaire agent produit un état `Errored` (ring + notif dédiés), un silence prolongé produit `Stalled` (ON par défaut, dédupliqué), et plus aucun échec ne se déguise en « agent finished ».

#### US-010: `Errored` via le shim (`ai.exit`) + ring et notif distincts
**Description:** As a dev orchestrateur, I want que la terminaison en erreur du binaire agent soit détectée et signalée distinctement so that je n'apprenne plus un crash en cliquant dans une pane « finished ».

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Le shim émet une frame `ai.exit { exit_code }` quand le binaire agent wrappé se termine (`run_real` connaît le statut réel, `exec.rs:39,184`) - chemin choisi car `ChildExit` ne porte que l'exit du SHELL, pas de l'agent (`pty_session.rs:837-854`, vérifié) ; le test de cap taille du shim reste vert (re-baseline justifié sinon)
- [ ] Windows : `run_real` y est un spawn+wait (`exec.rs:8-13`, split cfg unix/windows) qui connaît l'exit status - `ai.exit` est émis à l'identique ; seule la notif desktop reste le stub documenté
- [ ] `AgentState` gagne `Errored` (`ai_types.rs:67-76`) ; `state_rank` (`:140`) et `aggregate_by_tool` (`:150`) sont mis à jour (matches exhaustifs : le compilateur force la couverture)
- [ ] Given `ai.exit` avec code non-zéro, when traité, then la session passe `Errored` ; given code 0, then `Finished` (inchangé) ; given une terminaison par SIGINT/Ctrl+C (130 ou signal), then `Finished` - une interruption humaine n'est PAS une erreur
- [ ] Given une session `Errored`, when rendu, then le tab porte un point/ring de slot couleur dédié (`UiColors`, distinct de l'attention, slot anatomie du tab EP-005) et la sidebar agrège l'état (rendu `aggregate_by_tool` existant)
- [ ] Given la transition vers `Errored` fenêtre non focusée, when notif desktop, then le body est distinct : « {workspace} : agent exited (exit {code}) » - via le choke point existant (`fire_desktop_notification`, `ipc_handler.rs:94-193`)
- [ ] Given un agent lancé en bypassant le PATH shimé (chemin absolu), then aucun `ai.exit` n'arrive et le comportement actuel est préservé (dégradation documentée : `Finished` via `ai.stop`)
- [ ] Given un nouvel event `ai.session_start`/`ai.prompt_submit` sur la même pane, when traité, then `Errored` est remplacé par le nouvel état (pas d'erreur collante)
- [ ] Vérification visuelle manuelle du ring/point Errored sur les deux thèmes

#### US-011: `Stalled` via timestamp d'activité (ON par défaut, dédupliqué)
**Description:** As a dev orchestrateur, I want qu'un agent silencieux depuis N secondes en pleine « réflexion » soit marqué Stalled so that un agent figé (rate-limit, subprocess bloqué, boucle) ne reste pas « Thinking » pour toujours.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-010 (non fonctionnel : sérialise le churn sur AgentState/state_rank/aggregate_by_tool et le choke point notif - les deux stories touchent les mêmes matches exhaustifs)

**Acceptance Criteria:**
- [ ] `AgentSession` gagne `last_activity: Instant`, rafraîchi par chaque event hook de la session (`ai.prompt_submit`, `ai.tool_use`, `ai.notification`, `ai.stop` - handlers `ipc_handler.rs:1573-1896`)
- [ ] Given la feature active (défaut ON, seuil défaut 300 s, configurable et désactivable) et une session `Thinking` dont `last_activity` dépasse le seuil, when le sweep périodique passe (porteur : sweep 30 s existant, `event_handlers.rs:657-702` - granularité 300±30 s documentée), then l'état devient `Stalled` (nouvelle variante `AgentState`, matches exhaustifs mis à jour)
- [ ] Given une session `Stalled`, when un event hook arrive, then retour immédiat à l'état porté par l'event (Stalled n'est jamais collant)
- [ ] Given la transition vers `Stalled` fenêtre non focusée, when notif, then body distinct (« {workspace} : agent silencieux depuis {N} s ») - UNE seule notif par épisode de stall (dedup : pas de répétition à chaque sweep tant que l'épisode dure)
- [ ] Given un agent légitimement long (gros raisonnement sans tool calls), then le dedup par épisode + le seuil configurable + le kill-switch bornent le coût d'un faux positif à une notif - documenté dans la description du setting
- [ ] Given la feature désactivée, then zéro changement de comportement (aucun état Stalled possible)
- [ ] La Fleet Bar (US-015, si livrée) compte les sessions `Stalled` - match exhaustif forcé par le compilateur à l'ajout de la variante

---

### EP-005: Fleet observability - identité, ressources, agrégat

Donner au cockpit la vue flotte : qui tourne où, sur quel port, dans quel état - par pane et cross-workspace, sans scan manuel.

**Anatomie du tab (référentiel commun, FR-11).** Un tab porte au plus 2 adornments simultanés, dans cet ordre de priorité quand l'espace manque : point d'état (attention/Errored) > chip buffer en attente (US-003) > pill identité agent (US-013) > badges ports (US-014) > badge match transitoire (US-018). Règles de repli : la pill dégrade en point coloré, les ports se replient en `+N`, le badge match s'efface en premier. Troncature partagée : le titre cède l'espace aux adornments jusqu'à un minimum de 6 chars + ellipsis. Toute story d'adornment référence ce bloc.

**Definition of Done:** chaque pane affiche son agent et ses ports selon l'anatomie du tab, les collisions de ports sont signalées, et une Fleet Bar agrège les états cross-workspace avec click-to-warp ; le tout alimenté par un scan par-pane unique et borné. (US-016 idle-dim, P2 gated par Q2, est hors du coeur de la DoD.)

#### US-012: Socle scan par-pane (désagrégation du pool PID)
**Description:** As a dev orchestrateur, I want que le scan ports/process tourne par pane et non plus en pool plat par workspace so that l'identité agent et les ports soient attribuables à une pane précise.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `run_port_scan` (`event_handlers.rs:803-870`) est refactoré : collecte de paires (leaf `EntityId`, `child_pid`) au lieu du Vec plat (`:809-823`), UNE seule lecture de la table process par tick (snapshot partagé puis partition par sous-arbre - pas de N walks `/proc` pour N panes), `detect_ports` et `detect_ai_processes` (`ports.rs:58,:307`) évalués par sous-arbre de pane
- [ ] Le résultat est stocké par-pane (map leaf -> {ports, agents}) ET l'agrégat workspace existant (`ws.active_ports`, `ws.service_labels`) reste alimenté à l'identique - zéro régression des chips sidebar (`sidebar/mod.rs:494-565`)
- [ ] Given une pane fermée entre le scan et le dépôt du résultat, when application, then l'entrée est jetée (pas de panic, pas d'entrée orpheline)
- [ ] Le scan reste off-render-thread avec la cadence/debounce existants (500 ms + burst, `event_handlers.rs:776-799`) ; le coût d'un tick à 32 panes ne dépasse pas 2x le coût actuel (mesuré une fois, noté dans le commit)
- [ ] Windows : les stubs existants (`ports.rs:283,:371`) continuent de produire des résultats vides sans erreur (parité plateforme documentée)
- [ ] La liste de binaires détectés est dérivée de `TerminalAgent::ALL` (`agent_launcher.rs:49`, 16 binaires) au lieu du tableau divergent `AI_PROCESS_NAMES` (`ports.rs:294`, 3 entrées) - unification des deux vocabulaires (décision actée : source PID en v1, unification avec le vocabulaire hooks `AiTool` différée)

#### US-013: Pane identity pill - agent auto-détecté, persisté
**Description:** As a dev orchestrateur, I want une pill compacte sur le tab indiquant quel agent tourne dans la pane, qui survit au restart so that après 20 min ailleurs je sache encore qui fait quoi sans focuser chaque pane.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-012

**Acceptance Criteria:**
- [ ] Given le scan par-pane détecte un binaire agent dans le sous-arbre PID d'une pane, when rendu, then son tab porte une pill colorée au nom court de l'agent (couleurs outil promues en slots `UiColors` - actuellement inline `sidebar/mod.rs:586-588` ; slot anatomie du tab EP-005) - source PID authoritative, PAS l'heuristique de titre OSC (`pane.rs:538-586`, spoofable, qui reste pour le titre seul)
- [ ] Given l'agent se termine, when le scan suivant passe, then la pill disparaît (état live, pas de pill fantôme)
- [ ] La pill est persistée en session : champ optionnel `agent: Option<String>` sur `SurfaceDefinition` (`#[serde(default, skip_serializing_if)]`, pattern `schema.rs:474-496` - les session.json antérieurs restent lisibles)
- [ ] Given un champ `agent` relu de session.json inconnu de `TerminalAgent::ALL` (ou non conforme : longueur anormale, chars de contrôle), when restore, then la valeur est jetée silencieusement et aucune pill n'est rendue - whitelist à l'ingress, parité invariant US-057/EP-010 (session.json est local-only mais validé quand même)
- [ ] Given un restore de session, when rendu avant le premier scan, then la pill restaurée s'affiche atténuée (opacité 0.6, « dernier connu ») puis est confirmée ou retirée par le premier scan (burst 0/2 s existant)
- [ ] Given plusieurs binaires agents dans le même sous-arbre, then la pill affiche le plus proche de la racine (l'agent lancé, pas ses sous-process) ; cas documenté en test
- [ ] Given une pane sans agent, then aucune pill - zéro bruit sur les shells nus

#### US-014: Badges ports par pane + détection de collision
**Description:** As a dev orchestrateur, I want voir le port de dev-server de chaque pane sur son tab et être prévenu quand une pane annonce un port possédé par une autre so that les collisions de ports entre worktrees cessent d'être invisibles.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-012

**Acceptance Criteria:**
- [ ] Given le scan par-pane attribue des ports LISTEN à une pane, when rendu, then son tab porte un badge `:{port}` (≤ 2 badges + overflow `+N`, slot anatomie du tab EP-005) ; cliquable vers l'URL quand le service est frontend (`ws.service_labels` existant)
- [ ] Given le service_detector annonce une URL dans la pane B dont le port appartient au sous-arbre LISTEN de la pane A (A ≠ B), when rendu, then le badge de B passe en style alerte (slot `UiColors`) avec tooltip nommant la pane propriétaire - signal heuristique de niveau info, jamais bloquant ; les faux positifs connus (proxies, port-forwards, ré-annonces) sont documentés dans le code et tolérés v1
- [ ] Given le port libéré ou le conflit résolu, when scan suivant, then le badge revient à l'état normal (signal live)
- [ ] Given une pane aux ports multiples non-frontend, then badges textuels simples (pas de lien) - parité comportement sidebar
- [ ] Windows : aucun badge (stub ports vide) sans erreur ni placeholder cassé
- [ ] Vérification visuelle manuelle : les badges respectent l'anatomie du tab à 6+ tabs (truncation propre)

#### US-015: Fleet Bar - agrégat cross-workspace click-to-warp
**Description:** As a dev orchestrateur, I want une bande fine agrégeant l'état de tous les agents de tous les workspaces (« 3 running / 1 waiting / 1 errored ») avec click-to-warp so that l'état de la flotte soit lisible en permanence sans ouvrir ni sidebar ni queue.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-010

**Acceptance Criteria:**
- [ ] Given ≥ 1 session agent (tous workspaces confondus), when mode Cli, then une bande de 24 px ancrée sous la TitleBar et au-dessus de la grille (la grille perd 24 px quand la bande est présente ; chrome cockpit rendu par `PaneFlowApp` - PAS via la TitleBar push-only, `title_bar.rs:34-44`) affiche les comptes par état : Thinking / WaitingForInput / Errored, fold sur `workspaces[].agent_sessions`
- [ ] Given 0 session agent, then la bande est absente (zéro chrome mort pour le power-user sans agents)
- [ ] Given un clic sur le chip « waiting », then warp vers la prochaine pane en attente (réutilise `handle_jump_next_waiting`, `focus.rs:90`) ; un clic sur « errored », then warp vers la prochaine pane errored (extension du même helper de cycle `next_in_cycle` `:143`)
- [ ] Les comptes n'incluent que les sessions avec `surface_id` résolu (cohérence nav US-019 orchestration-v2) ; les non-mappées sont comptées à part dans le tooltip
- [ ] Given une transition d'état, when l'event hook arrive, then la bande se met à jour au prochain repaint (piloté par `cx.notify` existant - pas de timer dédié)
- [ ] Les couleurs des chips sont des slots `UiColors` ; vérification visuelle sur les deux thèmes
- [ ] Given le mode Diff ou Agents, then la bande n'est pas rendue (scope Cli strict)
- [ ] Note de séquencement : à construire après ≥ 1 semaine de dogfooding de l'Attention Queue (US-004) - si le triage on-demand suffit, évaluer d'abord l'alternative zéro-chrome (comptes dans le header de la sidebar) avant de payer 24 px permanents

#### US-016: Idle-dim opt-in - les panes Thinking reculent
**Description:** As a dev orchestrateur, I want que les panes dont l'agent génère reculent légèrement (teinte de fond basse-alpha) so that la pane qui bascule en attente ressorte comme seule surface pleine du cockpit. (Gated par Open Question Q2 : arbitrage de la règle « jamais dégrader l'inactif » avant toute implémentation.)

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given le setting `dim_thinking_panes` activé (défaut OFF), when une session mappée à la pane (via `surface_id`, mapping hooks orchestration-v2 US-017) est `Thinking`, then un quad basse-alpha (≤ 0.08) est peint au-dessus du fill de fond et SOUS le texte (précédent bell-flash, `background.rs:33`) - le TEXTE n'est jamais éclairci (orthogonal au SGR DIM, `element/mod.rs:798-800`)
- [ ] L'axe de dim est l'ÉTAT AGENT, jamais le focus : une pane inactive sans agent Thinking n'est JAMAIS atténuée (la règle « amplifier l'actif, jamais dégrader l'inactif » reste l'invariant focus)
- [ ] Given la transition hors `Thinking`, when repaint, then la teinte disparaît immédiatement, SANS animation (GPUI n'expose aucune API reduced-motion - vérifié ; statique par construction, pas de pulse)
- [ ] Given des transitions Thinking<->WaitingForInput rapprochées (< 1 s, agent qui flappe), then aucun flicker perceptible : le quad suit l'état au repaint normal, aucun repaint dédié n'est déclenché par la teinte
- [ ] Given une session `Thinking` mappée à une pane fermée, then aucun quad orphelin (le rendu lit l'état live des leaves existantes)
- [ ] Given le setting OFF (défaut), then zéro changement de rendu, zéro coût
- [ ] Vérification visuelle manuelle sur les deux thèmes : le scrollback reste lisible sous la teinte (contraste mesuré au pire cas)

---

### EP-006: Scale ergonomics - chercher et lire à l'échelle de la flotte

Les outils de lecture qui manquent quand 6 panes crachent du log : carte spatiale des matches, grep cross-panes, zoom par pane.

**Definition of Done:** une recherche montre OÙ ses hits se concentrent sur la scrollbar et se fan-out sur toutes les panes en un toggle ; chaque pane peut avoir sa propre taille de police.

#### US-017: Scrollbar match-rail - ticks proportionnels click-to-jump
**Description:** As a power-user terminal, I want des ticks proportionnels sur la scrollbar pour chaque hit de recherche dans tout le scrollback so that je voie où les erreurs se concentrent dans 50 k lignes sans naviguer match par match.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given une recherche active avec matches, when paint, then chaque `SearchMatch.start.line` est projeté en tick 1-2 px sur le track de la scrollbar (math `ScrollbarMetrics` existante : `track_top/track_height/history_size`, `scrollbar.rs:14,62`) - le compteur « N/M » (`view.rs:980`) et le highlight-all (`overlay.rs:18`) existent déjà et ne sont PAS re-spécifiés
- [ ] Given un clic sur un tick, then le viewport saute à l'offset correspondant (réutilise `offset_for_y`, `scrollbar.rs:36`)
- [ ] Given > track_height/2 matches (ticks plus denses que les pixels), then les ticks sont décimés par bucket de 2 px (un tick par bucket occupé) - paint borné par la hauteur du track, jamais par le nombre de matches (cap 10 000, `search.rs:17`)
- [ ] Given la recherche fermée, then le rail disparaît au même repaint
- [ ] Given 0 match, then aucun tick (le « 0 results » existant suffit)
- [ ] Couleur du rail : slot `UiColors`, contraste vérifié sur les deux thèmes

#### US-018: Fleet grep - une regex sur toutes les panes live
**Description:** As a dev orchestrateur, I want étendre ma recherche à toutes les panes de tous les workspaces en un toggle so that « quelque chose a cassé, quelle pane ? » se résolve en une requête au lieu de N Ctrl+Shift+F.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given la search overlay ouverte (`ctrl-shift-f` existant, `defaults.rs:224-228`), when je bascule le scope « pane -> flotte » (toggle cliquable + action remappable), then la requête (plain ou regex, moteur `search_term` existant, `search.rs:39`) s'exécute sur chaque terminal de chaque workspace (`collect_leaves` `queries.rs:43` puis `Pane::terminals` `pane.rs:433`)
- [ ] Le fan-out s'exécute hors render-thread (executor background ; `search_term` prend `&Arc<FairMutex<Term>>` et locke en interne) ; à 32 panes x 4 000 lignes, résultat complet < 500 ms (mesuré une fois, noté dans le commit)
- [ ] Given des résultats, then la search overlay liste les panes matchantes (nom + workspace + count) ; les tabs des panes matchantes portent un badge count transitoire (auto-dismiss 4 s ou à la fermeture de la recherche, slot anatomie du tab EP-005) ; Enter/clic sur une entrée focuse la pane avec la recherche locale pré-armée sur la même requête (le rail US-017, s'il est livré, s'affiche automatiquement)
- [ ] Given une regex invalide, then l'erreur unique du moteur existant s'affiche (pas de N erreurs dupliquées)
- [ ] Given 0 match flotte, then « 0 results » global ; given une pane fermée pendant le scan, then son résultat est jeté silencieusement
- [ ] Le cap par pane (10 000 matches) reste ; le total flotte est borné en mémoire (counts + premier match par pane, pas les Vec complets de N panes)

#### US-019: Per-pane font zoom (secondary-= / secondary-- / secondary-0)
**Description:** As a dev orchestrateur, I want zoomer/dézoomer la police de la pane focus indépendamment des autres so that je rétrécisse 5 panes de monitoring et agrandisse celle que je pilote.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `TerminalView` gagne `font_size_override: Option<Pixels>` (absent aujourd'hui, vérifié `view.rs:90-179`) ; `measure_cell`/`font_size` (`font.rs:300,305` - aujourd'hui sans paramètre, lisent le cache global) sont modifiés pour accepter l'override en paramètre - le défaut global (config + cache 500 ms, `font.rs:200-226`) reste la source quand l'override est None
- [ ] Given le focus terminal, when `secondary-=` / `secondary--` (libres dans defaults.rs ; shadow readline assumé et documenté : convention zoom des terminaux Linux gnome-terminal/Ghostty, exception actée à la Hard Constraint clavier, remappable), then la taille effective de LA pane focus change par pas de 1 px, clampée à [8.0, 32.0] (bornes existantes) ; `secondary-0` réinitialise (override = None)
- [ ] Given un changement de taille, then la géométrie de cellule est recalculée et le PTY resized (cols/rows recalculés -> les TUI fullscreen comme vim reflowent correctement - même chemin que le resize fenêtre)
- [ ] Given la borne atteinte, when zoom, then no-op silencieux (pas de toast)
- [ ] Les autres panes du même workspace ne changent PAS (l'override est strictement par-view)
- [ ] L'override est persisté en session (champ optionnel `font_size: Option<f32>` sur `SurfaceDefinition`, serde default - parité US-013) et restauré au restart
- [ ] Given un session.json avec `font_size` non fini (NaN/inf) ou hors [8.0, 32.0], when restore, then la valeur est clampée aux bornes ou jetée (override = None), jamais propagée à la géométrie de cellule - validation à l'ingress + test unitaire (parité invariant US-057/EP-010)
- [ ] Given un changement du font size global (settings), then les panes SANS override suivent le global ; les panes avec override le conservent (priorité documentée)

## Functional Requirements

- FR-01: Le Composer et le broadcast NE DOIVENT JAMAIS soumettre par défaut : la livraison standard est un prefill sans `\r` ; la soumission exige un geste humain distinct, et le broadcast ne soumet jamais (v1).
- FR-02: Le buffer broadcast NE DOIT livrer son contenu qu'à une pane dont la session n'est pas `Thinking`, et uniquement en prefill.
- FR-03: L'Attention Queue et la Fleet Bar DOIVENT refléter l'état live des sessions (`agent_sessions`) - jamais un snapshot périmé navigable vers une pane fermée.
- FR-04: Le Launch Pad DOIT réutiliser le moteur worktree existant (`worktree.rs`) et enregistrer ses worktrees comme `ManagedWorktree` - aucune seconde implémentation, aucun worktree non tracké.
- FR-05: Le scanner OSC 133 DOIT être inerte (zéro allocation par chunk, zéro mark) quand aucune séquence 133 n'entre, et ignorer sans panic tout payload malformé.
- FR-06: `Errored` DOIT provenir de l'exit réel du binaire agent (frame shim `ai.exit`) ; une interruption humaine (SIGINT) NE DOIT PAS produire `Errored`.
- FR-07: Toute donnée affichée issue du terminal, des hooks OU relue de session.json (questions, titres, commandes exportées, identité agent, font size) est UNTRUSTED : sanitizée/validée à l'ingress, affichée verbatim, jamais interprétée ni exécutée.
- FR-08: Toute nouvelle surface visuelle (pills, badges, chips, rail, liseré, pastilles) DOIT sourcer ses couleurs dans `UiColors` (slots nommés) - zéro hex inline dans le code de rendu.
- FR-09: Le scan par-pane DOIT lire la table process au plus une fois par tick, quel que soit le nombre de panes.
- FR-10: Aucune feature de ce PRD NE DOIT modifier le comportement des modes Diff et Agents, ni lire `AppMode` depuis la TitleBar (contrat push-only).
- FR-11: Tout adornment de tab DOIT respecter l'anatomie du tab définie en EP-005 (max 2 simultanés, ordre de priorité, règles de repli, troncature partagée).
- FR-12: Aucun binding par défaut en contexte Global/Terminal NE DOIT shadow un chord shell/readline/TUI courant ; toute exception est documentée dans l'AC et remappable.

## Non-Functional Requirements

- **Performance :** débit PTY avec scanner 133 actif sans séquences : régression ≤ 5 % sur `cat` 100 Mo. Fleet grep 32 panes x 4 000 lignes : < 500 ms hors render-thread. Paint du match-rail et des exit-dots : borné par la hauteur du track / le viewport, jamais par le volume de matches/marks. Latence keystroke->pixel : delta p95 ≤ 5 % vs baseline, mesuré via `PANEFLOW_LATENCY_PROBE` sur le scénario du runbook perf.
- **Robustesse :** zéro `unwrap()`/`panic` en chemin nominal (lints workspace) ; tout subprocess via `run_with_timeout` (deadlines existantes : git 10 s, worktree add 120 s, notif 10 s) ; rings bornés (marks ≤ 1 000/terminal, export ≤ 256 KiB, buffers broadcast 1/pane).
- **Sécurité :** parsing OSC 133 borné sur flux untrusted (pas d'alloc dépendante de payload hostile) ; questions/exports sanitizés par les chemins existants (512 chars, strip bidi/zero-width, strip escapes) ; champs session.json validés à l'ingress (whitelist agent, clamp font_size) ; aucune donnée de prompt dans la télémétrie (parité PII guard existant).
- **Cross-platform :** chaque story liste son chemin Linux/macOS/Windows ou un chemin platform-neutre explicite ; les stubs Windows existants (notifs, ports, AI-detect) sont préservés et documentés - jamais d'erreur visible côté Windows pour une feature dégradée.
- **Accessibilité :** aucune information portée par la couleur seule (pastilles exit : tooltip texte ; chips : libellés) ; pas d'animation introduite (idle-dim statique) ; toutes les actions nouvelles sont au clavier et remappables via le registre existant.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Queue vide | Ctrl+Shift+K sans agent en attente | Overlay avec état vide, Escape ferme | « Aucun agent n'attend » |
| 2 | Pane fermée sous la queue/le buffer | Fermeture entre rendu et activation/flush | No-op propre, entrée/buffer jetés | - |
| 3 | Échec worktree (branche verrouillée, git absent, timeout 120 s) | Confirmation Launch Pad | Erreur verbatim dans le modal, aucune pane créée | Message git verbatim |
| 4 | MAX_PANES atteint | Launch Pad / split | Erreur explicite, rien d'exécuté | « Limite de 32 panes atteinte » |
| 5 | Composer sur agent Thinking | Validation | v1 : bloquée avec chip d'état ; post-US-003 : bufferisée avec indicateur | « Agent en cours de génération » |
| 6 | Broadcast : tous les membres Thinking | Validation | 0 livré, N bufferisés, récap affiché (4 s) | « 0 livré, N en attente » |
| 7 | OSC 133 malformé / hostile | Payload non numérique, params géants | Ignoré, pas de panic, pas d'alloc non bornée | - |
| 8 | Marks après trim scrollback | Cap MAX_CHARS atteint | Marks hors historique purgés | - |
| 9 | Jump-to-prompt sans marks | secondary-shift-up sans snippet installé | No-op silencieux | - |
| 10 | Export sans marks | Action ExportBlocks | Pas d'export, message d'état | « Shell integration requise » |
| 11 | `ai.exit` absent (agent hors PATH shimé) | Lancement par chemin absolu | Comportement actuel (Finished via ai.stop), documenté | - |
| 12 | SIGINT humain sur l'agent | Ctrl+C (exit 130/signal) | Finished, jamais Errored | - |
| 13 | Stall faux-positif (long thinking) | Seuil 300 s dépassé sans tool call | 1 notif par épisode (dedup), seuil configurable, désactivable | « Agent silencieux depuis N s » |
| 14 | Regex invalide en fleet grep | Saisie utilisateur | Erreur unique du moteur existant | « Invalid regex » |
| 15 | Restore avec pill/zoom persistés | Session restaurée | Pill whitelistée et atténuée (0.6) jusqu'au 1er scan ; font_size clampé/jeté si non conforme | - |
| 16 | Windows : ports/notifs/AI-detect | Toute feature dépendante | Dégradation silencieuse via stubs existants, zéro erreur visible | - |
| 17 | Agent qui flappe Thinking<->Waiting | Transitions < 1 s répétées | Pas de flicker (idle-dim), pas de spam de notifs (transitions portées par les hooks existants) | - |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Le fork alacritty_terminal (US-006) crée une dette de maintenance durable et reverse la migration vers crates.io | Med | High | Spike obligatoire (upstream récent d'abord) ; fork minimal un-seul-hook pinné ; issue upstream ouverte pour un hook passthrough officiel ; chemin de repli : EP-003 décalé sans bloquer les 5 autres epics (seul US-009 en dépend) ; EP-003 séquencé en dernier dans la Phase 1 |
| 2 | Snippets OSC 133 en conflit avec les frameworks de prompt (starship, p10k, omz) | Med | Med | Chaînage des hooks (append, jamais replace), test manuel zsh+starship, opt-in/opt-out propre, dégradation = pas de marks (jamais de prompt cassé) |
| 3 | Le buffer broadcast livre au mauvais moment (course entre transition d'état et flush) | Low | High | Flush uniquement dans `upsert_session_state` sur le main thread (sérialisé par GPUI) ; prefill-only au flush ; latest-wins documenté ; test de la course pane-fermée |
| 4 | La désagrégation par-pane (US-012) multiplie le coût du scan | Med | Med | Un seul snapshot process par tick + partition ; budget 2x mesuré ; cadence/debounce inchangés |
| 5 | `ai.exit` gonfle le shim au-delà du cap taille | Low | Med | Test de cap existant en gate ; re-baseline justifiée dans le commit si nécessaire (précédent 8c8dc36) |
| 6 | 19 stories = plafond de PRD ; dérive de scope en cours de route | Med | Med | Phasage strict P0 (9 stories, ≤ 4 semaines) avant P1/P2 (≤ 8 semaines) ; chaque epic livrable indépendamment ; US-009/US-016 explicitement P2 sacrifiables ; US-005 v1 volontairement réduit (new-branch only) |
| 7 | Idle-dim contredit l'invariant visuel établi | Med | Low | Gated par Q2 (arbitrage explicite avant implémentation) ; opt-in OFF ; axe agent-state strictement séparé de l'axe focus |
| 8 | Surcharge d'adornments sur le tab strip (5 stories en ajoutent) | Med | Med | Anatomie du tab unique (EP-005 + FR-11) : max 2 simultanés, priorités, repli - chaque story d'adornment la référence |

## Non-Goals

- **Aucune soumission automatique, nulle part :** ni le Composer, ni le broadcast, ni le Launch Pad, ni un flush de buffer ne soumettent un prompt sans geste humain explicite. Le double-gate scripté (`PANEFLOW_IPC_SCRIPTING`) reste le seul chemin programmatique, inchangé.
- **Pas de persistance des marks OSC 133 en v1 :** le restore strip les escapes (sécurité, `pty_session.rs:1370`) ; les marks et CommandBlocks sont session-local. Un sidecar persisté est envisageable en v2 si l'usage le justifie.
- **Pas de re-spec du moteur orchestration :** worktrees, flow engine, broadcast CLI, submit gate, glow/jump/peek sont livrés (orchestration-v2) et réutilisés tels quels.
- **Rien dans Diff ni Agents view :** les deux autres modes sont gelés pour ce PRD.
- **Pas de resume-agents-after-restart :** la pill restaurée (US-013) étiquette la pane, elle ne relance rien - standing cut respecté.
- **Pas de features coupées re-proposées :** pas de Feed/right-sidebar, pas de trust par répertoire.
- **Pas de blocks Warp complets :** pas de sélection/collapse par bloc dans le grid en v1 - les marks servent la navigation, les pastilles et l'export ; le block model interactif complet est un chantier ultérieur.
- **Pas de picker de branche existante ni de création de workspace dans le Launch Pad v1 :** new-branch + split workspace courant uniquement ; les deux options sont des follow-ups une fois l'adoption dogfooding prouvée.
- **Pas d'i18n dans ce PRD** (backlog P3 existant, framework cible déjà choisi).

## Files NOT to Modify

- `src-app/src/diff/**` et `src-app/src/agents_view/**` - modes hors scope, gelés.
- `src-app/src/window_chrome/title_bar.rs` (au-delà d'un éventuel champ pushé) - contrat push-only : la TitleBar ne lit jamais `AppMode` (`title_bar.rs:34-44`) ; la Fleet Bar vit dans le chrome cockpit rendu par `PaneFlowApp`.
- `src-app/src/update/**` et `.github/workflows/release.yml` - pipeline release signé, aucun rapport avec ce PRD.
- `crates/paneflow-mcp/**` - bridge MCP read-only, contrat stable.
- Le slot `border_color` de la pane (`pane.rs:1744-1745`) - propriété du glow attention ; le liseré de groupe est un élément séparé.

## Technical Considerations

Cadré comme questions pour l'engineering, pas comme mandats :

- **OSC 133 - fork vs bump vs reader maison :** le spike US-006 tranche (et résout Q1). Recommandé : bump si l'upstream expose un hook ; sinon fork minimal pinné (un callback pré-`parser.advance`, vendored `event_loop.rs:154`), le scan restant dans le thread reader (zéro thread, zéro lock supplémentaires). Le reader maison (réimplémenter `EventLoop` : shutdown, drain, signal-mask, dup macOS) est écarté sauf blocage - blast radius trop large vs le hardening récent.
- **Frame `ai.exit` - nommage et transport :** nouvelle méthode vs extension de `ai.session_end` ? Recommandé : méthode dédiée `ai.exit { exit_code }` émise par `run_real` à la terminaison du wrapped child, pour ne pas surcharger la sémantique hook existante. Le handler serveur suit le pattern des six handlers existants (`ipc_handler.rs:1573-1896`).
- **Stockage des groupes broadcast :** sur `PaneFlowApp` (volatile) ou persisté en session ? Recommandé : volatile en v1 (les groupes vivent le temps d'une session de travail) ; persistance = extension future si demandée.
- **Buffer broadcast - point de flush :** dans `upsert_session_state` (main thread, sérialisé) en observant les transitions sortantes de `Thinking`. Alternative event-emitter dédiée jugée surdimensionnée.
- **Marks et resize/reflow :** v1 ancre au compteur de lignes absolu et accepte une dérive au reflow (resize de colonne). Une ré-ancre exacte exigerait un suivi de reflow par ligne - coût disproportionné, à réévaluer si la dérive gêne.
- **Fleet grep - stratégie de lock :** `search_term` locke le `FairMutex` par terminal ; le fan-out séquentiel en background (lock court par pane) évite de contendre 32 locks simultanément avec le render-thread. Paralléliser par chunks si la mesure < 500 ms n'est pas tenue.
- **Font zoom et `term.resize` :** le recalcul cols/rows passe par le chemin resize existant (mêmes events PTY). Vérifier le comportement du settle-poll prefill (`output_generation`) pendant un resize - le poll est robuste au churn mais à confirmer en test.
- **Vocabulaire agents :** v1 = source PID (`TerminalAgent::ALL`) pour la détection par pane ; l'unification avec le vocabulaire hooks (`AiTool`, 2 variantes + fallback) est différée - les deux coexistent sans conflit (sources disjointes : process-walk vs frames IPC).

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Temps pour triager 4 agents en attente (voir les questions + focuser le bon) | Tour de workspaces : ~30-60 s, 4+ focus | < 5 s, 1 raccourci + 1 Enter | Phase 1 (≤ 4 sem.) | Chrono manuel dogfooding (scénario scripté 4 agents) |
| Setup d'un agent isolé sur worktree (geste -> prompt pré-rempli) | Édition TOML + `paneflow up` : ~60-90 s | < 10 s via Launch Pad | Phase 1 (≤ 4 sem.) | Chrono manuel dogfooding |
| Localiser la commande échouée dans un run d'agent ≥ 2 000 lignes | Recherche string devinée + scroll : ~30 s+ | < 5 s (jump + pastilles) | Phase 1 (≤ 4 sem.) | Chrono manuel, scénario reproductible (runbook perf) |
| Faux « finished » sur exit non-zéro | 100 % (état unique Finished) | 0 % pour les agents lancés via PATH shimé | Phase 1 (≤ 4 sem.) | Test d'intégration shim (`ai.exit`) |
| Corruption stdin sur broadcast multi-état | Possible (CLI phase-blind) | 0 sur 20 broadcasts mixtes (test scripté) | Phase 1 (≤ 4 sem.) | Test d'intégration busy-aware |
| Adoption dogfooding (Arthur) | N/A (features inexistantes) | Composer + Queue + Launch Pad utilisés chaque session de travail ≥ 1 semaine | Phase 1 + 2 semaines | Auto-évaluation dogfooding |
| Matière distribution | N/A | ≥ 2 GIFs démo publiables (Launch Pad, Broadcast busy-aware) | Phase 1 (≤ 4 sem.) | Production des assets au fil des stories |

## Open Questions

- **Q1 - OSC 133 : bump, fork, ou reporter ?** Résolue PAR le spike en tête de US-006 (AC1) : vérifier l'upstream alacritty_terminal récent (hook pré-parse ou support 133) avant tout fork. Si fork : qui porte la charge de rebase aux releases alacritty ? La décision est tracée dans le commit du spike et reportée ici. - Owner : Arthur (validation du verdict du spike). Bloque : la suite de US-006, US-008, US-009 (US-007 est indépendante : les séquences émises sans tap sont droppées sans régression).
- **Q2 - Idle-dim vs règle « jamais dégrader l'inactif » :** la teinte Thinking est sur l'axe agent-state, pas focus, opt-in OFF - mais visuellement c'est une atténuation de panes non-actives. Valider explicitement ce renversement partiel (ou tuer US-016, seule story concernée). - Owner : Arthur. Bloque : US-016 uniquement.
[/PRD]

[PRD]
# PRD: Durcissement du plan de contrôle agent (Agent Control Plane Hardening - 2026-Q3)

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-06-19 | Arthur Jean | Draft initial - 5 epics / 12 stories. Follow-up de `prd-agent-control-plane-2026-Q3.md` après une démo réelle du conducteur. Rend le pilotage robuste et reproductible: soumission déterministe (bracketed paste), hooking fiable des agents conducteurs, attente serveur sans poll (`wait --idle`), récupération fiable des résultats (alt-screen -> fichier), ergonomie CLI + refonte du SKILL.md conducteur. |

## Problem Statement

Le plan de contrôle livré par `prd-agent-control-plane-2026-Q3.md` fonctionne: une démo réelle a fait piloter par un Claude Code conducteur deux agents CLI hétérogènes (Claude + Codex) pour auditer en lecture seule la vue diff, et la synthèse cross-vendor a été produite. Mais le chemin pour l'obtenir a été laborieux et non reproductible. La trace de la démo (2026-06-19) établit six défauts, dont deux causes mères:

1. **La soumission via `send --submit` n'est pas déterministe.** `surface.send_text` écrit le texte brut sans bracketed paste (`src-app/src/terminal/view.rs:645`, choix explicite "no wrapping") et le `\r` du flag `submit` est un second write PTY séparé (`src-app/src/app/ipc_handler.rs:2047`). L'agent TUI applique sa propre heuristique "burst = paste" (Claude Code affiche `[Pasted text #1]`) et le `\r` arrive avant qu'il ait fini de bufferiser le paste, donc il est avalé. `submit:true` est retourné alors que rien n'est soumis: le conducteur croit avoir dispatché, l'agent ne démarre pas, et il faut renvoyer un `\r` à la main. C'est le défaut le plus visible.

2. **Les agents conducteurs n'étaient pas hookés.** Lancés via `paneflow send <shell> "claude …" --submit` dans un shell nu au lieu de `paneflow up`, ils apparaissent `state: unknown_running` (`src-app/src/app/ipc_handler.rs:799`, `hooked:false`), ne reçoivent jamais d'event `ai.stop`/`ai.notification`, et n'ont pas de `last_result`. Le shim est pourtant dans le PATH de chaque pane (`src-app/src/terminal/pty_session.rs:1761`, `inject_ai_hook_env`); la cause fine est probablement `install_hook_guard` qui retourne `None` (`crates/paneflow-shim/src/main.rs:105`). Conséquence en cascade: sans `ai.stop`, le conducteur a dû bricoler un poller bash.

3. **Aucune primitive d'attente serveur de quiescence.** `paneflow wait` ne bloque que sur un pattern regex via poll 500ms (`src-app/src/cli/wait_cmd.rs`); il n'existe aucun mode "idle". Le conducteur a dû écrire un poller bash sur `output_generation`, qui a multiplié les pannes: faux positifs (compteur vide compté comme stable), contention IPC, et surtout les shells lancés en arrière-plan par Claude Code n'atteignent pas le socket IPC (le poller lisait `NA` et s'arrêtait). Arthur a dû le relancer plusieurs fois.

4. **Les résultats d'un agent en alt-screen sont irrécupérables.** `surface.read` en alternate screen ne retourne que le viewport courant (`src-app/src/terminal/pty_session.rs:1214`, `extract_scrollback`): les points 1 à 5 du rapport de Claude Code (TUI plein écran) ont défilé hors écran et ont été perdus. Le canal structuré `last_result` aurait dû combler ça mais il est quasi toujours `null` (le Stop hook de Claude Code ne porte qu'un chemin de transcript, pas le texte du tour, `ipc_handler.rs:132`).

5. **Confusion de nommage CLI/MCP.** Le conducteur a tapé `paneflow search_pane` (le nom de l'outil MCP) alors que le verbe CLI est `search` (`src-app/src/cli/mod.rs:42`). Un argv qui n'est pas un verbe connu tombe dans le fallback "lance la GUI", ce qui a déclenché le singleton guard au lieu d'une erreur actionnable.

6. **Pas de moyen de soumettre un composeur déjà rempli.** `send <pane> ""` est refusé par le guard `text.is_empty()` (`src-app/src/app/ipc_handler.rs:2022`), donc le conducteur n'a aucun verbe propre pour "appuyer Entrée"; il a dû passer par `$'\r'`.

**Why now:** la démo était la première validation end-to-end du wedge "agent cockpit cross-platform" sur lequel Paneflow se positionne. Le wedge est prouvé (cross-vendor + interactif + visible), mais tant que le pilotage reste fragile il n'est ni démontrable proprement (objectif: une démo filmable) ni utilisable au quotidien par le power-user. La recherche concurrentielle confirme que le piège submit (bracketed paste + Enter avalé) et l'attente de fin de tour sont des problèmes connus avec des patterns établis (claude-session-driver #20, awslabs/cli-agent-orchestrator): on applique des solutions éprouvées, pas de la R&D spéculative.

## Overview

Cinq chantiers qui ferment les six défauts, ordonnés par effet de levier. Le fil conducteur: éliminer les deux causes mères (soumission non déterministe, agents non hookés), puis l'outillage qui en découle (attente serveur, lecture de résultats), puis l'ergonomie et le manuel.

**EP-001 - Soumission déterministe.** `surface.send_text` gagne un mode bracketed paste explicite (`ESC[200~` … `ESC[201~`) et la soumission devient un `\r` écrit séparément, après un délai calibré (et idéalement une confirmation d'écho), jamais dans le même burst que le texte. Exposé via `send --paste` et adopté automatiquement par `--submit` vers un agent TUI. Le comportement brut reste disponible pour les shells. On débloque aussi `send --submit ""` (soumettre un composeur déjà rempli).

**EP-002 - Agents conducteurs hookés.** Un spike diagnostique pourquoi un agent démarré dans une pane existante ne hooke pas (instrumentation `PANEFLOW_HOOK_LOG`), puis on garantit le hooking, et on rend `fleet.list`/`status` explicites sur l'état hooked/unhooked et sa raison. Le skill spawnera désormais via `paneflow up` (hooké, label stable, cwd, session_id), pas via `send "claude"`.

**EP-003 - Attente sans poll.** Une primitive serveur `paneflow wait --idle <sel> [--for <ms>]` qui s'abonne au flux `surface_changed` déjà poussé (EP-002 du PRD parent) et débloque dès que `output_generation` est stable, supprimant tout poll côté client. Combinée au pattern sentinel (`wait --pattern`) pour la robustesse.

**EP-004 - Récupération fiable des résultats.** La discipline canonique pour l'alt-screen: faire écrire le rapport de l'agent dans un fichier que le conducteur lit, encodée dans le skill et outillée. En option, enrichir `last_result` en lisant le transcript pointé par le Stop hook.

**EP-005 - Ergonomie CLI et manuel.** Alias `search_pane` -> `search`, un verbe inconnu renvoie une erreur actionnable au lieu de lancer la GUI, et une refonte du `SKILL.md` conducteur qui encode toutes ces disciplines (spawn via up, submit fiable, attente par `wait --idle`/sentinel, rapport en fichier, bons noms de verbes).

Décisions structurantes: aucun gate n'est affaibli (le bracketed paste enveloppe le contenu, il ne contourne ni le gate scripting ni le mode unrestricted); tout I/O reste hors du render-thread GPUI (le `\r` différé passe par un timer non bloquant `cx.spawn`); on ne désactive jamais le bracketed paste de l'agent côté serveur (ce serait retirer une protection à l'utilisateur), on l'utilise.

## Goals

| Goal | Cible | Mesure |
|------|-------|--------|
| Soumission `--submit` fiable vers les agents TUI | >= 99% de soumissions effectives sur Claude + Codex (vs ~50% observé: 2 dispatches sur 2 ratés au premier coup) | test d'intégration + essais manuels US-003 |
| Agents conducteurs hookés à coup sûr | 100% des agents spawnés par le flux recommandé sont `hooked:true` et émettent `ai.stop` (vs `unknown_running`) | `fleet.list` + `watch --type ai.stop` |
| Attendre une fin de tour sans poll client | `wait --idle` détecte la quiescence en <= `for` + 100 ms, 0 poll côté conducteur (vs poller bash fragile) | test US-007 |
| Récupérer un résultat complet même en alt-screen | 100% du rapport récupéré (vs viewport seul, points 1-5 perdus) | démo + US-009 |
| Démo conducteur reproductible de bout en bout | 0 intervention manuelle de soumission, 0 relance de poller | rejeu de la démo |

## Target Users

### Le dev orchestrateur power-user (Arthur et profils similaires)
- **Role:** solo dev / indie maker qui pilote 3-8 agents CLI hétérogènes en parallèle dans Paneflow, via un agent conducteur.
- **Behaviors:** lance un conducteur (Claude Code) avec le skill `paneflow-conductor`, lui confie une tâche multi-agents, et n'intervient que pour les décisions.
- **Pain points:** la démo a exigé de soumettre les prompts à la main (Enter avalé), de relancer le poller de surveillance, et a perdu une partie des rapports (alt-screen). Le pilotage "marche" mais demande une babysitter.
- **Current workaround:** renvoyer des `$'\r'` à la main, bricoler des moniteurs bash, lire les panes au jugé.
- **Success looks like:** lancer le conducteur, le voir spawner les agents hookés, dispatcher de façon fiable, attendre les fins de tour par push, lire les rapports complets, sans toucher au clavier entre deux décisions.

### Le conducteur (agent CLI ou orchestrateur externe)
- **Role:** Claude Code / Codex tournant dans une pane via le skill + la CLI publique.
- **Behaviors:** `paneflow up` pour spawner, `send --submit` pour dispatcher, `wait --idle`/`watch` pour attendre, lecture d'un fichier de rapport, re-dispatch adaptatif.
- **Pain points:** `submit:true` mensonger, pas d'`ai.stop` sur les agents qu'il lance, pas de primitive d'attente propre, scrollback alt-screen illisible, `search_pane` qui lance une GUI.
- **Current workaround:** poller bash, `$'\r'`, lecture de viewport tronqué.
- **Success looks like:** chaque primitive fait ce que son retour annonce; la discipline du skill suffit à un workflow multi-agents sans bricolage.

## Research Findings

### Competitive Context
- **tmux `send-keys`:** sans `-l`, interprète les noms de touches; avec `-l`, envoie littéralement mais ne gère pas le bracketed paste. Le pattern fiable établi est de construire `ESC[200~ … ESC[201~` puis d'envoyer `Enter` séparément après un délai. Paneflow doit faire pareil côté serveur.
- **claude-session-driver (#20):** cas firsthand du piège paste+Enter avec Claude Code dans tmux, identique au nôtre. Mitigation recommandée: délai configurable + vérification que le tour a démarré (retry), ou attente d'un écho visuel avant le `Enter`.
- **awslabs/cli-agent-orchestrator:** détecte la fin de tour par sentinel (marqueur imprimé par l'agent) lu sur le flux PTY, avec quiescence en fallback. Confirme que la quiescence seule est fragile (l'agent "pense" en silence).
- **pexpect:** injecte un `delaybeforesend` pour fiabiliser l'envoi vers une app PTY, et attend un pattern avant d'envoyer la suite. Conforte le pattern délai + confirmation.
- **Market gap:** personne ne combine hétérogène + interactif + visible + pilotage fiable cross-vendor; c'est le wedge de Paneflow, mais il exige que ces primitives soient solides.

### Best Practices Applied
- Soumission: bracketed paste explicite en une écriture, `\r` en écriture séparée après signal (délai calibré, idéalement écho). Ne jamais inclure le `\r` dans le burst.
- Ne pas désactiver le bracketed paste de l'agent (`ESC[?2004l`): ce serait retirer une protection à l'utilisateur. On enveloppe, on ne désactive pas.
- Fin de tour: sentinel (le skill fait déjà imprimer `RAPPORT_TERMINE`) comme signal primaire, quiescence `output_generation` comme fallback borné par timeout.
- Alt-screen (`ESC[?1049h`): pas de scrollback récupérable; contournement canonique = faire écrire le résultat dans un fichier.

*Sources tracées dans la conversation de recherche (terminalguide bracketed-paste, xterm invisible-island, claude-session-driver #20, awslabs/cli-agent-orchestrator, pexpect, microsoft/terminal #15920).*

## Assumptions & Constraints

### Assumptions (to validate)
- `paneflow up` (agent=claude/codex) hooke effectivement l'agent (label + cwd + session_id), contrairement à `send "claude"`. US-004 (spike) valide.
- La cause du non-hooking via `send` est `install_hook_guard` qui retourne `None` (cwd/permission au moment où le shim s'exécute). US-004 confirme via `PANEFLOW_HOOK_LOG`.
- Un délai serveur calibré (ordre de 60-80 ms) entre le paste et le `\r` suffit dans la quasi-totalité des cas; un mode confirmation par écho couvre la longue traîne. US-001 mesure.
- `surface_changed` (output_generation) est poussé pour toutes les panes y compris non hookées (vérifié dans la trace de démo): `wait --idle` peut s'y abonner. US-007 s'appuie dessus.

### Hard Constraints
- **Cross-platform Linux/macOS/Windows obligatoire** (CLAUDE.md): chaque story a un chemin par OS ou un stub documenté cohérent avec l'existant.
- **Aucun gate affaibli:** la soumission reste gatée (gate scripting OU mode unrestricted); le bracketed paste enveloppe le contenu, il ne contourne aucun contrôle. Le fence anti-injection reste intact.
- **Jamais d'I/O bloquant sur le render-thread GPUI** (audit 2026-06-04): le `\r` différé et toute attente passent par un timer/`cx.spawn` non bloquant.
- Exit codes CLI réutilisés (0 OK, 1 runtime, 3 target, 4 timeout). `MAX_PANES = 32`, `MAX_WORKSPACES = 20`.
- **Free + OSS:** levier de distribution, jamais derrière un plan payant.
- Convention commits: `feat(module): US-NNN - description`, atomiques par story. GPL-3.0. Aucune attribution Claude.

## Quality Gates

These commands must pass for every user story:
- `cargo fmt --check` - formatage canonique (gate CI release, 4 legs)
- `cargo clippy --workspace -- -D warnings` - zéro warning
- `cargo test --workspace` - tous les tests workspace

Pour les stories qui touchent le SKILL.md ou exigent un agent réel (US-004, US-009, US-012):
- Vérification manuelle documentée (un agent live est requis; noter "vérifié sur instance live" ou "non vérifié, à exécuter par Arthur" dans le commit).

## Epics & User Stories

### EP-001: Soumission déterministe

Rendre `--submit` fiable à >= 99% vers les agents TUI en enveloppant le texte en bracketed paste et en soumettant par un `\r` séparé et calibré, sans affaiblir aucun gate.

**Definition of Done:** un dispatch `send --submit` vers Claude et Codex démarre le tour du premier coup; le comportement brut reste disponible pour les shells; `send --submit ""` soumet un composeur déjà rempli; tests couvrant le wrapping et l'ordre d'écriture.

#### US-001: Mode bracketed paste + soumission différée dans `surface.send_text`
**Description:** As a conducteur, I want que l'envoi de texte à un agent soit enveloppé en bracketed paste et soumis par un `\r` séparé so that l'agent ne traite pas mon prompt comme un paste non confirmé et démarre le tour de façon fiable.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given un appel `surface.send_text` avec `paste:true`, when il s'exécute, then le texte est écrit enveloppé de `ESC[200~` et `ESC[201~` en une seule écriture PTY, sans `\r` à l'intérieur
- [ ] Given `submit:true` (avec `paste:true`), when la soumission a lieu, then le `\r` est écrit dans une écriture PTY SÉPARÉE, après un délai configurable (défaut documenté, ordre 60-80 ms), planifié hors du render-thread (`cx.spawn`/timer non bloquant)
- [ ] Given un agent encore en train de bufferiser le paste, when le délai par défaut est insuffisant, then un mécanisme de confirmation (écho du paste détecté via `output_generation`/scan, ou retry borné du `\r`) garantit la soumission sans boucle infinie
- [ ] Given `paste:false` (défaut historique), when `send_text`, then le comportement brut actuel est strictement inchangé (rétrocompatibilité shells)
- [ ] Given la pane est fermée entre l'écriture du paste et le `\r` différé, when le timer se déclenche, then aucun write orphelin ni panic; l'erreur sentinelle/exit propre est respectée
- [ ] Given une soumission, when elle a lieu, then elle reste soumise au gate (scripting OU unrestricted): le bracketed paste ne contourne aucun contrôle
- [ ] Tests: le wrapping (présence des sentinelles, `\r` hors burst), l'ordre d'écriture, et le cas pane-fermée sont couverts

#### US-002: Câblage CLI `send --paste` + auto-paste sur `--submit` agent
**Description:** As a conducteur, I want une option CLI pour le mode paste et que `--submit` l'adopte automatiquement vers un agent so that je dispatche un prompt sans connaître les détails terminal.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `paneflow send <agent-pane> "<prompt>" --submit`, when la cible est un agent (hooké, tool connu), then le mode bracketed paste est utilisé automatiquement et le tour démarre du premier coup
- [ ] Given un flag explicite `--paste`, when passé, then il force le mode bracketed paste indépendamment de la détection
- [ ] Given `--submit` sans `--paste` vers un shell nu (pas un agent), then le comportement brut est conservé (une commande shell simple reste soumise comme avant)
- [ ] Given le gate scripting fermé et pas de mode unrestricted, when `--submit`/`--paste`, then refus clair (exit non-zéro), parité avec l'existant
- [ ] Given un selector ambigu/inexistant, when `send`, then exit 3 (parité `resolve_target`)
- [ ] Tests: parsing des flags, sélection du mode (agent vs shell), refus sous gate fermé

#### US-003: `send --submit ""` soumet un composeur déjà rempli
**Description:** As a conducteur, I want soumettre une pane dont le composeur contient déjà du texte so that je dispose d'un verbe propre pour "appuyer Entrée" sans bricoler `$'\r'`.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `paneflow send <pane> "" --submit`, when exécuté, then seule la soumission (`\r`) est envoyée, sans insérer de texte, et c'est accepté (le guard `text.is_empty()` ne s'applique plus quand `submit:true`)
- [ ] Given `send <pane> ""` SANS `--submit`, when exécuté, then refus inchangé (rien à faire, message actionnable)
- [ ] Given la soumission vide, when elle a lieu, then elle reste gatée comme tout submit (scripting/unrestricted)
- [ ] Given la pane n'existe plus, when `send "" --submit`, then exit 3, aucun write partiel
- [ ] Tests: submit vide accepté sous gate ouvert, refusé sous gate fermé, no-op sans submit

### EP-002: Agents conducteurs hookés

Garantir que les agents pilotés par le conducteur sont turn-tracked (events `ai.*`, `last_result`), pour débloquer le push et tuer le polling.

**Definition of Done:** la cause du non-hooking via `send` est diagnostiquée; les agents spawnés par le flux recommandé sont `hooked:true` et émettent `ai.stop`; `fleet.list`/`status` exposent clairement hooked/unhooked et la raison.

#### US-004: Spike - diagnostiquer le non-hooking d'un agent démarré dans une pane existante
**Description:** As a mainteneur, I want comprendre pourquoi `send "claude"` ne hooke pas alors que le shim est dans le PATH so that je corrige la bonne cause au lieu de deviner.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given une instance live avec `PANEFLOW_HOOK_LOG` activé, when un agent est lancé via `send "claude"` dans un shell existant, then le log montre où la chaîne casse (shim atteint? `install_hook_guard` retourne `None`? cwd/permission sur `.claude/settings.local.json`? session_id absent?)
- [ ] Given le même test via `paneflow up` (agent=claude), when comparé, then on confirme que `up` hooke (ou on documente que non) et la différence exacte (session_id, cwd, séquencement)
- [ ] Given le diagnostic, when conclu, then une note `tasks/` documente la cause racine et oriente US-005 (cette story est un spike, vérifiée sur instance live d'Arthur, pas par les gates seuls)
- [ ] Given le shim atteint mais `install_hook_guard` None, when observé, then le cas d'échec (writable check, cwd) est isolé avec une repro minimale

#### US-005: Garantir le hooking (selon le diagnostic US-004)
**Description:** As a conducteur, I want que tout agent que je spawne soit hooké so that je reçois `ai.stop`/`ai.notification` et `last_result` sans polling.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**
- [ ] Given la cause identifiée en US-004, when corrigée, then un agent spawné par le flux recommandé (`paneflow up`) est `hooked:true` dans `fleet.list` et son `ai.stop` est reçu par `watch --type ai.stop`
- [ ] Given `install_hook_guard` échouait silencieusement, when le fix est en place, then l'échec d'installation du hook est journalisé (warn) avec la raison, jamais silencieux
- [ ] Given un agent lancé manuellement (`send "claude"`) reste non recommandé, when il démarre, then soit il hooke aussi, soit `fleet.list` indique explicitement `hooked:false` + raison (pas un état ambigu)
- [ ] Given macOS et Windows, when la story est livrée, then le chemin par OS est traité ou un stub documenté cohérent existe (le hook guard écrit dans un fichier de config par-OS)
- [ ] Tests: la transition vers un état hooké après spawn est couverte là où c'est testable headless; le reste est vérifié sur instance live

#### US-006: `fleet.list`/`status` - hooked vs unknown_running explicites
**Description:** As a conducteur, I want distinguer sans ambiguïté un agent tracké d'un agent seulement détecté so that je sais si je peux compter sur ses events ou si je dois un fallback.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**
- [ ] Given un agent hooké et un agent seulement détecté via scan, when `fleet.list`, then le premier porte `hooked:true` + un `state` réel, le second `hooked:false` + `state:"unknown_running"` + une raison courte (`reason:"no_hook"`)
- [ ] Given `surface.status <unhooked>`, when appelé, then il indique que l'état précis est indisponible (pas de faux `thinking`/`idle` trompeur dérivé du scan)
- [ ] Given aucun agent, when `fleet.list`, then `{agents:[]}` exit 0 (parité)
- [ ] Tests: sérialisation des deux formes (hooked/unhooked + reason)

### EP-003: Attente sans poll

Donner au conducteur une primitive d'attente serveur qui supprime tout poll client fragile.

**Definition of Done:** `paneflow wait --idle <sel> [--for <ms>]` bloque jusqu'à stabilisation de `output_generation` via abonnement push, avec timeout et exit codes propres; aucun poll côté client.

#### US-007: `paneflow wait --idle <sel> [--for <ms>]` côté serveur
**Description:** As a conducteur, I want bloquer jusqu'à ce qu'une pane cesse de produire de la sortie so that je sais qu'un tour est fini sans écrire de poller.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `paneflow wait --idle <sel>`, when la pane n'a pas changé d'`output_generation` pendant `--for` ms (défaut documenté), then la commande retourne exit 0 sans aucun poll côté client (abonnement au flux `surface_changed`)
- [ ] Given une pane qui produit en continu (spinner), when `wait --idle --timeout <s>`, then elle ne déclare jamais idle et sort en exit 4 au timeout (cas documenté: combiner avec un sentinel `--pattern`)
- [ ] Given aucune instance Paneflow, when `wait --idle`, then exit 3 + message actionnable (pas de hang)
- [ ] Given SIGINT pendant l'attente, when reçu, then sortie propre, abonnement libéré côté serveur
- [ ] Given un selector ambigu, when `wait --idle ba`, then exit 3 + candidats (parité)
- [ ] Tests: quiescence détectée, timeout sur flux continu, libération d'abonnement

#### US-008: Robustesse de `wait` (heartbeat, exit codes, parité `--pattern`)
**Description:** As a conducteur, I want que `wait` se comporte de façon prévisible et détecte une connexion morte so that je ne reste pas bloqué indéfiniment.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] Given `wait --idle` et `wait --pattern`, when l'un ou l'autre, then les exit codes sont cohérents (0 succès, 3 instance/target, 4 timeout)
- [ ] Given une connexion d'abonnement morte (serveur disparu), when `wait --idle` attend, then la mort est détectée (via heartbeat 30s hérité d'EP-002) et la commande sort en exit 1, pas un hang infini
- [ ] Given `--idle` et `--pattern` passés ensemble, when exécuté, then la sémantique est définie (idle ET/OU pattern) et documentée, pas une erreur silencieuse
- [ ] Tests: matrice exit codes, détection de mort

### EP-004: Récupération fiable des résultats

Récupérer l'intégralité d'un rapport d'agent même quand il tourne en alt-screen (TUI plein écran).

**Definition of Done:** la discipline "rapport en fichier" est outillée et documentée dans le skill; optionnellement `last_result` est enrichi via le transcript du Stop hook.

#### US-009: Discipline "rapport en fichier" outillée
**Description:** As a conducteur, I want récupérer le rapport complet d'un agent en alt-screen so that je ne perds pas les lignes défilées hors viewport.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given un agent en alt-screen (Claude Code), when le skill lui demande d'écrire son rapport dans un fichier convenu (chemin temp passé dans le prompt) plutôt que dans le terminal, then le conducteur lit le fichier intégralement (zéro troncature viewport)
- [ ] Given le skill, when il prescrit ce pattern, then il explique pourquoi (alt-screen = pas de scrollback) et fournit un exemple de chemin temp + nettoyage
- [ ] Given le fichier de rapport, when le tour est fini, then un mécanisme de nettoyage est documenté (pas de fuite disque), cohérent avec le canal de contexte >64 KiB (US-015 du PRD parent)
- [ ] Given un agent NON en alt-screen (Codex), when il rend dans le terminal, then `surface.read` reste suffisant (pas de régression, le fichier est optionnel selon l'agent)
- [ ] Vérification: démontré sur instance live avec Claude Code (alt-screen) et Codex (non alt-screen)

#### US-010: Enrichir `last_result` via le transcript du Stop hook (optionnel)
**Description:** As a conducteur, I want que `last_result` porte le résumé du dernier tour quand c'est possible so that j'ai un canal structuré sans demander un fichier.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] Given le Stop hook fournit un chemin de transcript (pas le texte), when `ai.stop` est traité, then `last_result` est rempli en lisant le transcript (dernier message/résumé) au lieu de rester `null`, lecture bornée et hors render-thread
- [ ] Given le transcript est absent/illisible/trop gros, when lu, then `last_result:null` proprement (pas d'erreur, pas de blocage), bascule fichier au-delà d'un cap documenté
- [ ] Given un agent sans transcript (Codex/autres), when `ai.stop`, then `last_result:null` sans erreur
- [ ] Tests: extraction depuis un transcript fixture, cas absent/oversize

### EP-005: Ergonomie CLI et manuel conducteur

Supprimer les pièges de surface (noms de verbes, fallback GUI) et réécrire le manuel pour encoder toutes les disciplines apprises.

**Definition of Done:** `search_pane` est un alias de `search`, un verbe inconnu renvoie une erreur actionnable, et le `SKILL.md` conducteur encode spawn-via-up, submit fiable, attente par `wait --idle`/sentinel, et rapport-en-fichier.

#### US-011: Alias `search_pane` -> `search` + verbe inconnu = erreur actionnable
**Description:** As a conducteur, I want que les noms d'outils MCP fonctionnent ou échouent proprement en CLI so that une typo de verbe ne lance pas une instance GUI.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `paneflow search_pane <args>`, when exécuté, then il est traité comme `paneflow search` (alias ajouté à `VERBS`) ou renvoie une erreur le pointant; il ne lance JAMAIS la GUI
- [ ] Given un argv qui ressemble à une commande mais n'est pas un verbe connu (`paneflow blah`), when exécuté, then exit non-zéro + message "verbe inconnu; voir `paneflow --help`", au lieu de lancer la GUI/déclencher le singleton guard
- [ ] Given `paneflow` nu (aucun argv), when lancé, then la GUI démarre comme aujourd'hui (comportement préservé)
- [ ] Given les alias MCP connus (`read_pane`, `list_panes`), when tapés en CLI, then même traitement (alias ou erreur claire), cohérent
- [ ] Tests: `is_cli_verb`/dispatch pour alias, verbe inconnu, argv vide

#### US-012: Refonte du `SKILL.md` conducteur
**Description:** As a power-user, I want un manuel conducteur qui encode les disciplines fiables so that le conducteur reproduit la démo sans bricolage.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002, US-007, US-009, US-011

**Acceptance Criteria:**
- [ ] Given le SKILL.md, when lu, then il prescrit de spawner les agents via `paneflow up` (hooké), pas via `send "claude"`, avec un exemple de `workspace.toml`
- [ ] Given la soumission, when documentée, then elle utilise `send --submit` (mode paste fiable) et explique de vérifier que le tour a démarré (status/output_generation) avant d'enchaîner
- [ ] Given l'attente, when documentée, then elle privilégie `paneflow wait --idle`/`watch --type ai.stop` (push) et le pattern sentinel; elle interdit explicitement le poller bash en arrière-plan (qui n'atteint pas le socket)
- [ ] Given la lecture d'un résultat d'agent en alt-screen, when documentée, then elle prescrit le rapport en fichier (US-009)
- [ ] Given les verbes, when cités, then ce sont les verbes CLI réels (`search`, pas `search_pane`)
- [ ] Given une action destructrice/ambiguë, when le skill guide, then il rend la main à l'humain (discipline conservée du PRD parent)
- [ ] Vérification: rejeu manuel de la démo (audit vue diff) sans intervention de soumission ni relance de poller

### EP-006: Parité Windows du plan de contrôle

L'audit Windows (2026-06-19) établit que le control plane est déjà câblé à ~90% sur Windows (named pipe IPC serveur+client, shim copie+PATHEXT, ConPTY OS-agnostique, Ctrl-C -> ai.stop, paths, selectors). Le seul gap fonctionnel est le bus push (`events.subscribe` Unix-only -> `paneflow watch` renvoie `-32004`, `ipc.rs:873`), plus deux chemins compile-only à valider en runtime.

**Definition of Done:** `events.subscribe`/`watch` fonctionnent sur Windows (named pipe persistant abonné, sans le `break` one-request); les chemins Windows compile-only (Ctrl-C -> ai.stop, bracketed paste ConPTY) sont smoke-testés sur une vraie machine.

#### US-013: Bus push `events.subscribe` sur Windows (named pipe persistant)
**Description:** As a conducteur sur Windows, I want recevoir les events poussés sur le named pipe so that `watch`/`wait --idle` et le pilotage par `ai.stop` fonctionnent comme sur Unix.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None (le push Unix existe déjà via `serve_subscription`; cette story le porte sur Windows)

**Acceptance Criteria:**
- [ ] Given Windows, when `events.subscribe`, then `serve_subscription` est invoqué (dispatch `#[cfg(windows)]` symétrique de l'Unix `ipc.rs:809`, `return` avant le `break` one-request `ipc.rs:911`) au lieu du stub `-32004` (`ipc.rs:873`)
- [ ] Given un abonné Windows qui se déconnecte pendant que le serveur écrit un event, when le write a lieu, then la déconnexion est traitée comme une fin propre (PAS d'abort process `STATUS_STACK_BUFFER_OVERRUN`): le write-abort named pipe est neutralisé (write fail -> éviction RAII), comportement vérifié par smoke runtime Windows
- [ ] Given un abonné Windows lent, when la file dépasse le cap borné, then même backpressure + marqueur `dropped` que sur Unix (pas de fuite)
- [ ] Given `paneflow watch` sur Windows, when instance live, then il stream les events en JSONL (parité Unix), heartbeat 30s, Ctrl-C sort proprement
- [ ] Given le host de build est non-Windows, when la story est livrée, then elle compile via `cargo check --target x86_64-pc-windows-msvc` et le runtime est marqué "à smoke-tester sur Windows" (parité avec `exec.rs` US-017 du PRD parent)
- [ ] Given la DACL par défaut du named pipe, when un abonné se connecte, then l'accès reste restreint à l'owner (parité d'esprit avec peer-UID; SDDL hardening documenté si différé)

#### US-014: Smoke tests Windows runtime (Ctrl-C -> ai.stop, bracketed paste ConPTY)
**Description:** As a mainteneur, I want valider en runtime Windows les chemins jusqu'ici compile-only so that la parité Windows est prouvée, pas supposée.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001, US-013

**Acceptance Criteria:**
- [ ] Given un agent hooké sur Windows, when l'utilisateur fait Ctrl-C mid-turn, then `ai.stop` est émis et le spinner se libère (valide `exec.rs:404`, jusqu'ici compile-verified only)
- [ ] Given US-001 (bracketed paste) sur Windows via ConPTY, when un prompt est dispatché à un agent, then le paste arrive intact (`ESC[200~`/`ESC[201~` non filtrés par ConPTY) et le tour démarre; si ConPTY filtre, le fallback est documenté
- [ ] Given un agent sans bracketed paste activé (`ESC[?2004h` absent), when paste, then comportement défini et documenté (pas de corruption silencieuse)
- [ ] Given ces smokes, when exécutés, then les résultats sont consignés (procédure documentée, parité runbook heaptrack)

## Functional Requirements

- FR-01: `surface.send_text` doit supporter un mode bracketed paste (`ESC[200~`/`ESC[201~`) et écrire le `\r` de soumission dans une écriture PTY séparée et différée.
- FR-02: La CLI `send` doit exposer `--paste` et adopter automatiquement le mode paste pour `--submit` vers un agent TUI, en conservant le mode brut pour les shells.
- FR-03: `send --submit ""` doit soumettre un composeur déjà rempli (lever le guard texte vide quand `submit:true`).
- FR-04: Le système doit garantir (ou diagnostiquer puis corriger) le hooking des agents spawnés par le flux recommandé, et journaliser tout échec d'installation de hook.
- FR-05: `fleet.list`/`surface.status` doivent distinguer explicitement `hooked:true` d'`unknown_running` avec une raison.
- FR-06: Le système doit fournir `paneflow wait --idle <sel> [--for <ms>]` bloquant jusqu'à quiescence d'`output_generation` via abonnement push, sans poll client.
- FR-07: Le skill conducteur doit prescrire la récupération des résultats d'agents alt-screen via fichier.
- FR-08: `search_pane` (et autres noms d'outils MCP) ne doivent jamais lancer la GUI en CLI; un verbe inconnu doit renvoyer une erreur actionnable.
- FR-09: Aucun de ces chemins ne doit contourner le gate scripting/unrestricted ni le fence anti-injection.

## Non-Functional Requirements

- **Reliability (submit):** >= 99% de soumissions effectives sur Claude + Codex (essais répétés); le `\r` n'est jamais inclus dans le burst de paste; mécanisme de confirmation/retry borné (pas de boucle infinie).
- **Performance (submit):** délai paste -> `\r` par défaut dans l'ordre 60-80 ms, configurable; planifié hors render-thread (0 appel bloquant dans un handler de render).
- **Performance (wait --idle):** débloque en <= `for` + 100 ms après la dernière mutation; latence de l'event push hérité < 100 ms P95; 0 poll côté client.
- **Reliability (wait):** heartbeat 30 s pour détecter une connexion morte; timeout configurable -> exit 4.
- **Reliability (hooking):** 100% des agents spawnés par le flux recommandé sont `hooked:true`; tout échec d'installation de hook est journalisé (jamais silencieux).
- **Reliability (results):** 100% du rapport récupéré en alt-screen via fichier (vs viewport seul); nettoyage du fichier en fin de tour (0 fuite disque).
- **Security:** aucun gate affaibli; le bracketed paste enveloppe le contenu sans contourner les contrôles; peer-UID + 0600 conservés; le fence reste intact.
- **Compatibility:** Linux (Wayland+X11), macOS (Intel+Apple Silicon), Windows 10/11; chaque organe a un chemin par OS ou un stub documenté.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Pane fermée pendant le `\r` différé | Close entre paste et soumission | Aucun write orphelin ni panic, exit/erreur propre | - |
| 2 | Délai par défaut trop court (charge/latence) | Agent encore en buffering | Confirmation par écho ou retry borné du `\r`, jamais de boucle infinie | - |
| 3 | Submit vide sans composeur rempli | `send "" --submit` sur input vide | Soumet une ligne vide (no-op agent), pas une erreur | - |
| 4 | Agent non hooké | Lancé hors flux recommandé | `fleet.list` -> `hooked:false` + `reason`, pas d'état trompeur | - |
| 5 | `wait --idle` sur flux continu | Spinner permanent | Jamais idle -> exit 4 au timeout (combiner sentinel) | "timed out waiting for idle" |
| 6 | `wait --idle` sans instance | Pas de Paneflow | Exit 3, message actionnable | "No running Paneflow instance" |
| 7 | Rapport agent en alt-screen sans fichier | Agent rend dans le terminal plein écran | Skill prescrit le fichier; sinon résultat tronqué documenté | - |
| 8 | `paneflow search_pane` (nom MCP en CLI) | Typo de verbe | Alias vers `search` ou erreur, jamais la GUI | "did you mean `paneflow search`?" |
| 9 | Verbe inconnu | `paneflow blah` | Exit non-zéro + message, pas de GUI ni singleton clash | "unknown verb; see `paneflow --help`" |
| 10 | `last_result` transcript illisible/oversize | Transcript absent/trop gros | `last_result:null` proprement, bascule fichier au-delà du cap | - |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Le délai paste -> `\r` calibré reste insuffisant sous forte charge | Med | High | Confirmation par écho (`output_generation`/scan) + retry borné; délai configurable (US-001) |
| 2 | Le fix hooking exige une instance live qu'on ne peut tester headless | Med | Med | Spike dédié (US-004) sur l'instance d'Arthur via `PANEFLOW_HOOK_LOG`; fix guidé par le diagnostic (US-005) |
| 3 | Quiescence anti-spinner: `wait --idle` ne déclenche jamais | Med | Med | Timeout -> exit 4 + recommander le sentinel `--pattern` en complément (US-007/008, skill US-012) |
| 4 | Désactiver le bracketed paste casserait la protection user | Low | High | Décision: on enveloppe, on ne désactive jamais `ESC[?2004l` côté serveur |
| 5 | Auto-paste sur `--submit` casse l'envoi de commandes shell simples | Low | Med | Détection agent vs shell; mode brut conservé par défaut pour les shells; `--paste` explicite (US-002) |
| 6 | Régression de gate (soumission qui contourne le contrôle) | Low | High | Tests explicites: submit/paste refusés sous gate fermé (US-001/002/003) |

## Non-Goals

- **Réimplémenter les harness.** On pilote des process opaques par leur I/O; on ne touche ni leur auth ni leur boucle modèle.
- **Turn-tracking des 6 agents sans hooks** (copilot, kiro-cli, droid, agy, openclaw, amp). Cécité documentée, hors scope (comme le PRD parent).
- **Désactiver le bracketed paste de l'agent.** On l'enveloppe; on ne retire jamais cette protection à l'utilisateur.
- **Persistance de l'état de flotte au restart.** Hors scope (comme le PRD parent).
- **Optimisations de la vue diff** (git show storm, double build rows, sticky-header scan). C'est un backlog séparé issu de la même démo, traité ailleurs.
- **Monétisation.** Feature free + OSS.

## Files NOT to Modify

- `alacritty_terminal` (upstream crates.io 0.26) - VT emulation, ne pas forker.
- Le pin du fork GPUI (`ArthurDEV44/zed@paneflow/markdown-append-fix`) - ne pas remplacer par crates.io.
- La couche auth IPC (peer-UID/SO_PEERCRED, `ipc.rs:660-692`) - étendre, jamais affaiblir.
- Le fence anti-injection (`tools.rs`, `neutralize_sentinel`/`fence_id`) - intact, ne pas diverger.

## Technical Considerations

Frame comme questions pour l'engineering, pas mandats:

- **Soumission différée (US-001):** où planifier le `\r`? Recommandé: côté serveur via `cx.spawn`/timer non bloquant (un seul appel CLI, atomique), plutôt que deux round-trips CLI. Le mécanisme de confirmation: délai calibré simple, ou attente d'un écho du paste dans `output_generation` avant le `\r`? Recommandé: délai par défaut + écho-confirm optionnel pour la longue traîne.
- **Détection agent vs shell (US-002):** s'appuyer sur l'état hooké/le tool connu de la pane (`agent_sessions`) pour décider l'auto-paste? Ou un heuristique terminal (l'app a-t-elle envoyé `ESC[?2004h`)? Recommandé: l'état hooké d'abord, le flag `--paste` comme override.
- **`wait --idle` (US-007):** réutiliser l'abonnement `events.subscribe`/`surface_changed` côté client (le binaire CLI tient la connexion) plutôt qu'un nouveau endpoint serveur? Recommandé: client CLI abonné, logique de quiescence côté CLI sur le flux push (zéro poll, et le serveur reste mince).
- **Hooking (US-005):** si `install_hook_guard` échoue par cwd/permission, faut-il installer le hook depuis un emplacement stable (data_dir) plutôt que le cwd de la pane? À trancher après le spike US-004.
- **`last_result` transcript (US-010):** lire le transcript à chaque `ai.stop` (coût I/O par tour) vs à la demande sur `status`? Recommandé: à la demande/lazy pour ne pas charger le chemin chaud.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Soumissions `--submit` effectives (Claude+Codex) | ~50% (2/2 ratées au 1er coup en démo) | >= 99% | Phase-1 | test d'intégration + essais manuels |
| Agents spawnés `hooked:true` via flux recommandé | 0% (unknown_running) | 100% | Phase-1 | `fleet.list` + `watch ai.stop` |
| Poller bash côté conducteur | 1 par agent, fragile (NA, relances) | 0 (remplacé par `wait --idle`) | Phase-1 | rejeu démo |
| Rapport d'agent récupéré en alt-screen | viewport seul (points 1-5 perdus) | 100% via fichier | Phase-1 | démo US-009 |
| `paneflow search_pane` lance la GUI | oui (singleton clash) | non (alias/erreur) | Phase-1 | test US-011 |
| Interventions manuelles dans la démo conducteur | plusieurs (Enter, relances) | 0 | Phase-1 | rejeu de la démo |

## Open Questions

- Faut-il un délai paste -> `\r` adaptatif (calibré par mesure d'écho) ou un défaut fixe configurable suffit-il en pratique? À trancher en US-001 après mesure sur Claude + Codex.
- Le hooking d'un agent lancé manuellement (`send "claude"`) doit-il être réparé aussi, ou seulement documenté comme non recommandé au profit de `paneflow up`? À trancher après US-004.
- `wait --idle` et `wait --pattern` combinés: sémantique ET (les deux) ou OU (le premier qui arrive)? À trancher en US-008 selon l'usage du skill.
[/PRD]

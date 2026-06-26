# Brouillon Show HN - Paneflow

Document de travail. On construit le post étape par étape, section par section.

## Titre

```
Show HN: Paneflow - cross-platform GPUI app for parallel coding agents
```

70 caractères (limite Hacker News : 80). Angle retenu : app native cross-platform en GPUI pour piloter plusieurs agents en parallèle.

## Texte du post

### Version FR

Je fais tourner beaucoup d'agents de code en parallèle sur mes projets, notamment Claude Code, Codex, OpenCode ou d'autres CLI. Les lancer n'a jamais été le problème. Le vrai problème, c'était de garder une vue fiable de tout ce qui tourne : lequel réfléchit, lequel attend une réponse, lequel a fini, sur quelle branche.

Dans une grille tmux, ou dans une dizaine de fenêtres Ghostty avec plusieurs sessions dans chacune, la charge mentale monte vite. À un moment, ce n'est plus un problème de terminal : c'est un problème de coordination. Ghostty est très bien, mais chaque fenêtre relance son propre renderer GPU, donc cette façon de travailler finit aussi par peser sur la machine.

Alors je l'ai construit. Paneflow, un workspace natif en Rust sur GPUI, le framework de Zed. Il tourne sur Linux, macOS et Windows, sans WSL ni Electron. Il est pensé pour alléger à la fois la charge mentale et la charge machine.

Le vrai pari est la coordination. J'ai appelé ce système Paneflow Conductor : tout ce que je peux faire dans Paneflow, un agent peut le faire aussi par la même CLI et le même socket local. Je peux tout piloter à la main, ou laisser un agent en coordonner trois autres pendant que je supervise et reprends la main sur n'importe quelle pane. Quand plusieurs agents travaillent sur la même branche, ils ne restent pas chacun dans leur coin : Paneflow détecte les changements du repo, expose l'état de chaque pane, et permet à un agent de lire la sortie d'un autre avant d'agir pour réduire les chevauchements.

Si tu connais les Agent Teams de Claude Code ou les workflows de type swarm, le modèle mental est proche : un leader qui délègue. La différence, c'est que Paneflow Conductor n'est pas un swarm fermé dans un seul outil. Il transforme chaque pane en terminal pilotable et observable. Par exemple Claude Code peut piloter un pane Codex CLI, un OpenCode, un Droid CLI ou une autre CLI prise en charge dans Paneflow.

`paneflow ps` liste les panes et agents en cours avec leur état réel. `paneflow watch` streame les changements en JSONL, poussés par les hooks et l'event bus, sans polling. Un agent peut donc voir ce qui tourne, lire l'état d'un pane, envoyer un prompt à un autre agent et attendre un event, le tout via la CLI publique.

Par défaut, les prompts sont pré-remplis et c'est moi qui appuie sur Entrée. Le mode auto-submit est opt-in, et les lectures inter-panes restent encadrées par une protection anti-injection qui traite la sortie terminal comme non fiable.

Les agents peuvent aussi lire le terminal des autres en lecture seule via le serveur MCP intégré (`list_panes`, `read_pane`, `search_pane`). Claude Code dans un pane peut lire la sortie de test que Codex vient de produire dans un autre, sans copier-coller.

Quand je veux isoler le travail, chaque tâche tourne sur sa propre branche dans un worktree git dédié. Je peux revoir tous les diffs côte à côte dans la vue Review, une colonne par worktree, avec l'agent, le modèle et une estimation de coût par colonne, sans basculer entre des fenêtres ou éditeurs séparés.

Gratuit et open source (GPL-3.0-or-later), pensé pour les power users qui pilotent plusieurs agents, pas pour remplacer ton éditeur.

Démo : [lien vidéo]
Repo : https://github.com/ArthurDEV44/paneflow

### Version EN (à poster)

I run a lot of coding agents in parallel on my projects, mostly Claude Code and Codex, sometimes OpenCode or Gemini. Starting them was never the hard part. Keeping a reliable view of everything running was: which one is thinking, which one is waiting on me, which one finished, which branch it is on, and how to act on that without scraping scrollback or polling a terminal.

In a tmux grid, or worse a dozen Ghostty windows with a few sessions in each, I kept losing context. I love Ghostty, but each separate window brings its own GPU renderer, so that workflow also gets heavy fast.

So I built Paneflow: a native workspace in Rust on GPUI, the UI framework behind Zed. It runs on Linux, macOS and Windows. No WSL, no Electron. One process renders every pane; on my live Linux workstation it stays around 110 MB PSS with roughly thirty panes open.

The bigger bet is coordination. I called this system Paneflow Conductor: anything I can do in Paneflow, an agent can do through the same CLI and local socket. I can drive everything by hand, or let one agent coordinate three others while I keep the window open and can take over any pane. It also makes same-branch multi-agent work less blind: each agent can see the other panes' state, read their output, and react to changes landing in the repo.

If you have used Claude Code Agent Teams or swarm-style workflows, the mental model is similar: a lead agent delegates. The difference is that Paneflow Conductor is not a closed swarm inside one tool. It turns every pane into a controllable, observable terminal. For example, Claude Code can drive a Codex CLI pane, OpenCode, Droid CLI, or any other CLI supported in Paneflow.

`paneflow ps` lists the running panes and agents with their real state. `paneflow watch` streams state changes as JSONL, pushed by hooks and the event bus instead of polling. An agent can see what is running, read a pane's current state, send a prompt to another agent, and wait for an event, all through the public CLI.

By default, prompts are pre-filled and I still press Enter. Auto-submit is opt-in, and cross-pane reads stay wrapped as untrusted terminal output so a repo or agent transcript cannot silently hijack the run.

Agents can also read each other's terminals through the built-in read-only MCP bridge (`list_panes`, `read_pane`, `search_pane`). Claude Code in one pane can read the test output Codex just produced in another without me copy-pasting between windows.

When I want isolation, each task runs on its own branch in a dedicated git worktree. I can review every diff side by side in one view, one column per worktree, tagged with the agent, model, and estimated cost for that column, without switching across separate editors or windows.

Free and open source (GPL-3.0-or-later).
Demo: [video link]
Repo: https://github.com/ArthurDEV44/paneflow

### Notes de mesure RAM (méthode + mesures)

**Mesure (Linux, non-destructif, lecture /proc) :**

```bash
pid=$(ps -eo pid,rss,comm --sort=-rss | awk '$3 ~ /paneflow/ {print $1; exit}')
grep -E "^Rss:|^Pss:" /proc/$pid/smaps_rollup   # mémoire du renderer mutualisé
ps --ppid $pid -o comm= | grep -c zsh            # nombre de shells (proxy panes)
```

Le PSS (Proportional Set Size) déduplique les pages partagées : c'est la mémoire réellement attribuée au process, donc le chiffre honnête à citer. Le RSS surcompte le partagé.

**Mesures live (Fedora/Linux, instance de travail) :**

| Date | Panes | RSS | PSS |
|---|---:|---:|---:|
| 2026-06-17 | ~27 | 203 MB | 115 MB |
| 2026-06-18 | 27 | 126 MB | 105 MB |

Le 2026-06-18, compte autoritatif via le MCP (`list_panes`) : **27 panes sur 17 workspaces, dont 4 agents Claude Code actifs** (process up 1h39, 126 MB RSS / 104 MB PSS). Le chiffre "en action" est donc réel, pas synthétique : ~104 MB PSS pour 27 panes dont 4 agents qui tournent.

Le RSS varie avec le scrollback et l'activité (126 à 203 MB observés), le PSS reste stable autour de 104-115 MB. **Recommandation : citer le PSS (~110 MB pour une trentaine de panes), pas le RSS.**

**À vide :** ~40-50 MB observé sur Windows (gestionnaire des tâches = working set, proche du RSS). Métrique différente du PSS Linux : pour un chiffre homogène, mesurer le PSS à vide sur Linux (instance jetable, commande plus bas) ; sinon, si on te pousse en commentaire, préciser "40-50 MB working set on Windows".

**Les 2 chiffres imparables restant à mesurer (nécessitent des fenêtres GUI, à faire quand tu es devant) :**

1. À vide (0 pane). Pour ne pas toucher la session live, lancer une instance jetable isolée, ouvrir 0 pane, mesurer, fermer :

```bash
mkdir -p /tmp/pf-empty && chmod 700 /tmp/pf-empty
XDG_CONFIG_HOME=/tmp/pf-empty XDG_DATA_HOME=/tmp/pf-empty XDG_RUNTIME_DIR=/tmp/pf-empty paneflow &
sleep 3
pid=$(ps -eo pid,comm --sort=-rss | awk '$2 ~ /paneflow/ {print $1; exit}')
grep "^Pss:" /proc/$pid/smaps_rollup
```

2. Comparatif Ghostty (le chiffre qui tue) : ouvrir 10 fenêtres Ghostty (1 shell chacune), sommer leur PSS, comparer à 1 Paneflow avec 10 panes :

```bash
for p in $(pgrep -x ghostty); do grep "^Pss:" /proc/$p/smaps_rollup; done | awk '{s+=$2} END{printf "%.0f MB total\n", s/1024}'
```

Le delta = ce que coûtent N renderers GPU séparés vs 1 renderer mutualisé.

**Cadrage honnête (HN vérifiera) :** l'avantage RAM vaut pour l'usage "fenêtres séparées", pas "splits dans une fenêtre" (là tu paies déjà un seul renderer). Face aux splits, les vrais arguments sont la sidebar de statut, le MCP, et les worktrees.

## Réponses aux objections (préparées pour le jour J)

> Règle de ton : concéder quand l'objection est juste (HN récompense l'honnêteté, pas la défense), répondre en pair, jamais marketing, jamais défensif. Court.

### 1. Pourquoi pas tmux / Zellij ?

**FR :** tmux et Zellij sont excellents, surtout en SSH et en headless, et je m'en sers encore là. Paneflow est une app GUI locale, pas un multiplexeur que tu portes sur un serveur distant. Ce que j'ajoute par-dessus le multiplexage : un statut d'agent lisible (alimenté par les hooks de chaque CLI, pas du polling), la lecture inter-panes via MCP, et la review de diffs/worktrees au même endroit. Si ton travail est surtout distant, reste sur tmux ou Zellij, c'est le bon outil.

**EN:** tmux and Zellij are great, especially over SSH and headless, and I still use them there. Paneflow is a local GUI app, not a multiplexer you carry onto a remote box. What it adds on top of multiplexing: a readable per-agent status (driven by each CLI's hooks, not polling), cross-pane reads over MCP, and diff/worktree review in the same place. If your work is mostly remote, tmux or Zellij are still the right tool.

### 2. Pourquoi pas juste les splits de mon terminal (Ghostty / Kitty / WezTerm) ?

**FR :** Honnêtement, face aux splits dans une seule fenêtre, mon argument RAM tombe : tu paies déjà un seul renderer. L'écart se creuse uniquement contre l'habitude de N fenêtres séparées, chacune avec son renderer GPU. Contre des splits, les vraies différences sont ailleurs : la sidebar de statut hook-driven, le pont MCP entre panes, et la vue de review worktree-par-worktree. Si tes splits te suffisent et que tu suis tes agents de tête, tu n'as pas besoin de Paneflow.

**EN:** Honestly, against splits in a single window my RAM argument doesn't hold: you already pay for one renderer. The gap only shows against the habit of N separate windows, each with its own GPU renderer. Against splits, the real differences are elsewhere: the hook-driven status sidebar, the MCP bridge between panes, and the worktree-by-worktree review view. If your splits already work and you track your agents in your head, you don't need Paneflow.

### 3. En quoi c'est différent de cmux / Conductor / [autre orchestrateur d'agents] ?

**FR :** La plupart de ceux que j'ai testés sont mac-only ou des apps web/Electron. Mon angle, c'est un binaire natif unique sur Linux, macOS et Windows, sans WSL, plus deux choses que je n'ai pas trouvées ailleurs : les agents qui se lisent entre eux via MCP, et un moteur de flow scriptable (`flow.toml`) pour enchaîner des étapes d'agents. Je ne prétends pas réinventer l'orchestration ; je voulais juste qu'elle soit native et cross-platform.

**EN:** Most of the ones I tried are macOS-only or web/Electron apps. My angle is a single native binary on Linux, macOS and Windows, no WSL, plus two things I couldn't find elsewhere: agents reading each other over MCP, and a scriptable flow engine (`flow.toml`) to chain agent steps. I'm not claiming to reinvent orchestration; I just wanted it native and cross-platform.

### 4. Un pont MCP qui laisse un agent lire la sortie d'un autre pane, ce n'est pas un vecteur de prompt-injection ?

**FR :** Bonne question, et oui c'est un vrai sujet. Trois précisions. Le pont est strictement read-only : `list_panes`, `read_pane`, `search_pane`, aucun outil n'écrit ni ne pilote un pane. La sortie terminal est traitée explicitement comme non fiable côté serveur. Et le risque est exactement celui que tu prends déjà quand tu copies-colles la sortie d'un agent dans un autre, sauf qu'ici c'est explicite et borné à de la lecture. Ça ne supprime pas le risque que le modèle obéisse à du texte hostile lu dans un buffer, c'est au modèle de ne pas le faire, mais ça ne lui donne aucun pouvoir d'écriture.

**EN:** Fair question, and yes it's a real concern. Three things. The bridge is strictly read-only: `list_panes`, `read_pane`, `search_pane`, no tool writes to or drives a pane. Terminal output is explicitly treated as untrusted on the server side. And the risk is exactly the one you already take when you copy-paste one agent's output into another, except here it's explicit and bounded to reads. It doesn't remove the risk of a model obeying hostile text it reads from a buffer, that's on the model, but it grants no write power.

### 5. Un seul process rend tout : s'il crashe, je perds tous mes agents ?

**FR :** C'est le tradeoff d'un renderer mutualisé, je ne vais pas le cacher. Si le process meurt, les PTY enfants meurent avec lui, comme quand ton terminal crashe. Deux choses limitent la casse : la session est persistée (workspaces, layouts, CWD) et restaurée au redémarrage, et les agents comme Claude Code ou Codex ont leur propre reprise de session. Ce que tu perds, c'est l'état en vol d'un tour en cours, pas ta disposition de travail.

**EN:** That's the tradeoff of a shared renderer, I won't hide it. If the process dies, the child PTYs die with it, same as when your terminal crashes. Two things limit the blast radius: the session is persisted (workspaces, layouts, CWD) and restored on restart, and agents like Claude Code or Codex have their own session resume. What you lose is the in-flight state of a running turn, not your working layout.

### 6. Télémétrie ? Ça appelle la maison ?

**FR :** Pas par défaut. La télémétrie est opt-in via un modal au premier lancement, désactivée tant que tu ne dis pas oui. Si tu actives, elle envoie cinq events de cycle de vie (lancement, sortie, check et install d'update, session corrompue), jamais de contenu terminal, de chemins ni de prompts. `PANEFLOW_NO_TELEMETRY=1` est un kill switch inconditionnel, et le client entier est auditable dans `crates/paneflow-telemetry/`. En pratique je ne m'en sers même pas pour de l'analytics produit, mon seul analytics est sur le site web.

**EN:** Not by default. Telemetry is opt-in via a first-run modal, off until you say yes. If you enable it, it sends five lifecycle events (start, exit, update check and install, session corrupted), never terminal content, paths, or prompts. `PANEFLOW_NO_TELEMETRY=1` is an unconditional kill switch, and the whole client is auditable in `crates/paneflow-telemetry/`. In practice I don't even use it for product analytics, my only analytics is on the website.

### 7. Pourquoi GPL-3.0 ? Je peux l'utiliser au boulot ?

**FR :** Oui, utilise-le au boulot sans problème : la GPL ne contraint que la redistribution d'une version modifiée, pas l'usage. J'ai choisi le copyleft volontairement pour que les forks restent ouverts. C'est un outil que je veux voir rester libre, pas une lib que tu embarques dans un produit fermé.

**EN:** Yes, use it at work freely: GPL only constrains redistributing a modified version, not usage. I chose copyleft on purpose so forks stay open. It's a tool I want to keep free, not a library you'd vendor into a closed product.

### 8. Pourquoi GPUI, et c'est stable, surtout sur Windows ?

**FR :** GPUI est le framework d'interface derrière Zed, donc il tourne déjà en prod sur les trois plateformes, rendu GPU natif (Vulkan, Metal, DirectX). C'est un pari assumé : je le consomme depuis git, pas depuis crates.io. Sur Windows précisément, le MSI signé est live aujourd'hui ; les limitations connues sont listées dans `docs/WINDOWS.md`. La latence ressentie des apps Electron/Tauri que je fuyais vient du WebView, GPUI n'en a pas.

**EN:** GPUI is the UI framework behind Zed, so it already runs in production on all three platforms with native GPU rendering (Vulkan, Metal, DirectX). It's a deliberate bet: I consume it from git, not crates.io. On Windows specifically, the signed MSI is live today; known limitations are in `docs/WINDOWS.md`. The Electron/Tauri latency I was running from comes from the WebView, GPUI has none.

### 9. Ça me verrouille sur quels agents ?

**FR :** Aucun. Un pane est un vrai terminal, donc n'importe quel CLI tourne dedans, y compris un que je n'ai jamais vu. Ce qui est par-agent, c'est le confort : le statut hook-driven et `mcp install` couvrent Claude Code, Codex, Gemini et opencode aujourd'hui, plus d'autres via des shims. Lance autre chose et il tourne quand même, tu perds juste le statut riche et l'install MCP automatique.

**EN:** None. A pane is a real terminal, so any CLI runs in it, including one I've never seen. What's per-agent is the comfort layer: the hook-driven status and `mcp install` cover Claude Code, Codex, Gemini and opencode today, plus others via shims. Run something else and it still works, you just lose the rich status and the automatic MCP install.

### 10. Open source aujourd'hui, mais quel est le business model ? Ça restera gratuit ?

**FR :** Le core que tu vois reste OSS et gratuit pour toujours, sous GPL. Je pars sur un modèle open-core à la VSCode : si je monétise un jour, ce sera des features autour (collaboration, cloud, ce genre de choses), jamais en rognant ce qui est déjà là. Je ne vais pas te promettre que je ne facturerai jamais rien, ce serait malhonnête, mais le noyau ne deviendra pas payant.

**EN:** The core you see stays OSS and free forever, under GPL. I'm going with an open-core model like VSCode: if I ever monetize, it'll be features around it (collaboration, cloud, that kind of thing), never by walling off what's already there. I won't promise I'll never charge for anything, that'd be dishonest, but the core won't go paid.

### 11. Comment tu calcules le coût en dollars ?

**FR :** C'est une estimation, pas une facture : tokens consommés par l'agent, multipliés par une table de prix par modèle que je tiens dans le binaire. Utile pour comparer le coût relatif entre worktrees et repérer un agent qui part en boucle, pas pour réconcilier au centime avec ta facture provider.

**EN:** It's an estimate, not a bill: tokens consumed by the agent, multiplied by a per-model price table I keep in the binary. Useful to compare relative cost across worktrees and catch an agent that's looping, not to reconcile to the cent with your provider invoice.

### À vérifier avant de poster (claims qui touchent ces réponses)

- **Latence (objection 8)** : "latency I could feel" est subjectif et c'est ok de le dire tel quel. Ne pas claim de chiffre comparatif Electron vs GPUI sans benchmark. La sonde `PANEFLOW_LATENCY_PROBE` existe mais n'a pas de chiffre publié.
- **Coût en dollars** : FAIT (objection 11 + corps cadré comme "estimation"). Garder le cadrage "estimate, not a bill".
- **Crash behavior (objection 5)** : confirmer que la session restore relance bien layouts+CWD et pas les process agents (c'est ce que je décris, à re-tester une fois avant le jour J).
- **GPL** : FAIT, les deux versions du post disent `GPL-3.0-or-later`.
- **RAM** : FAIT côté méthode (section Notes de mesure RAM, citer le PSS ~110 MB). Reste à mesurer "à vide" + comparatif Ghostty quand tu es devant l'écran.

## Checklist jour J (timing, démo, dispo)

### Timing

- **Jour** : mardi, mercredi ou jeudi. Éviter lundi (noyé) et vendredi/week-end (audience basse).
- **Heure** : viser 6h-9h Pacific Time, soit **15h-18h heure française (CEST)**. C'est le réveil de l'audience US, la plus grosse sur HN.
- **Pourquoi cette fenêtre** : un post entre par la "new queue" (la file des nouveautés). Il atteint la front page seulement s'il accumule assez d'upvotes vite : la vélocité des 1-2 premières heures pèse plus que le total. Poster au pic d'audience maximise cette fenêtre.

### Pré-vol (la veille / le matin)

- [ ] Repo, site et README alignés (FAIT).
- [ ] Social preview GitHub uploadée (Settings > General), testée en collant le lien repo dans un champ X/LinkedIn.
- [ ] Démo vidéo uploadée, lien testé en navigation privée.
- [ ] Titre final tranché (angle cross-platform GPUI app, cf section Titre).
- [ ] Chiffres RAM "à vide" + comparatif Ghostty mesurés (section Notes RAM), screenshot prêt.
- [ ] Premier commentaire prêt à coller (clarifications + méthodo RAM).
- [ ] Liens du post testés un par un (repo, download, démo).

### Démo (décidé : muette + annotations texte, pas de voix off)

Raison : voix off anglaise non-native = risque downside élevé sur HN (lecture audible, accent au premier plan) pour un gain faible. Le muet annoté joue sur la force (anglais écrit nickel), marche sans le son (majorité des viewers), et est plus dense. Voix réservée à X/LinkedIn plus tard si besoin.

- **Capture** : OBS Studio. Sur Wayland (Fedora), source = **Screen Capture (PipeWire)** (la capture X11 classique ne marche pas) ; le portail GNOME laisse choisir la fenêtre Paneflow. Cadrer sur Paneflow (fenêtre seule ou plein écran maximisé), 1920x1080, 30 fps (60 si on veut les animations GPUI bien fluides), sortie mp4.
- **Montage** : kdenlive ou DaVinci Resolve (gratuits). Couper les temps morts pour tenir 60-90s, commencer direct dans l'action (pas d'intro), poser 4-5 annotations texte courtes en anglais : "Claude Code + Codex, same repo", "sidebar shows who's waiting on you", "one agent reads another's pane (MCP)", "review every worktree side by side".
- **Hébergement** : YouTube en **public/répertorié** (PAS unlisted). On veut la découverte YouTube gratuite (recherche + recommandations) et le compteur de vues comme preuve sociale. Référence : la vidéo cmux/Manaflow "terminal built for multitasking" est à 30k vues, en partie grâce au public. Coller l'URL dans le `[lien vidéo]` du post ET en haut du repo.
- **Packaging YouTube** (compte car public) : titre orienté recherche ("Paneflow: run Claude Code, Codex and any coding agent in parallel"), description avec liens repo + site + download + 2-3 lignes de pitch, thumbnail lisible et annotée. Chaîne : envisager une chaîne "créateur" unique (Paneflow + Distill + Rust Doctor) plutôt qu'une chaîne par produit, comme Manaflow regroupe ses produits sous @ManaflowAI.
- **Bonus** : régénérer `assets/images/demo.gif` à partir de la nouvelle vidéo (le GIF actuel date du 12/06 et peut montrer une UI périmée).

### Déroulé de soumission

1. Sur news.ycombinator.com/submit : **title** = `Show HN: Paneflow - cross-platform GPUI app for parallel coding agents`, **url** = `https://github.com/ArthurDEV44/paneflow` (le repo : stars + audience dev), **text** = vide (si l'URL est remplie, le champ texte n'est pas utilisé).
2. **Juste après**, poster la **Version EN (à poster)** en **premier commentaire** d'auteur, avec le lien démo YouTube + le lien download dedans.
3. Le post arrive dans /newest, invisible du grand public à ce stade.
4. Monter en front page (home, top ~30, le trafic) = upvotes rapides + peu de flags. L'algo favorise la vélocité des 1-2 premières heures, pas le total. D'où le créneau.
5. Si ça prend : pic de clics/stars/downloads. Si ça ne prend pas : un repost espacé est toléré si le 1er n'a eu aucune attention.
6. Rappel : un Show HN est one-shot. Ne pas poster si la vidéo n'est pas prête ou si les liens ne sont pas testés ; décaler à mardi plutôt que bâcler.

### Dispo (le facteur n°1 de survie d'un Show HN)

- Bloquer **3-4h juste après le post** pour répondre dans le thread sans délai. Un auteur présent et réactif est le plus gros signal positif : ça nourrit la discussion, donc la visibilité, donc les upvotes.
- Garder les réponses aux objections (section au-dessus) sous la main, mais reformuler à la main, pas copier-coller mot pour mot (HN sent le texte préfabriqué).
- Ton : concéder quand l'objection est juste, jamais défensif, parler en pair.

### Interdits HN (à ne pas faire)

- Ne **jamais** demander d'upvotes (ni publiquement ni en privé) : c'est contre les règles, ça peut faire flag ou sanctionner le post.
- Pas de "edit: thanks for the upvotes" ni de méta-commentaire sur le score.
- Pas de comptes amis qui upvotent en rafale depuis la même IP (détecté, pénalisé).

### Après

- Répondre à tout commentaire les 2 premières heures.
- Si ça prend, relayer sur X (build-in-public, EN) et LinkedIn (FR, ton terrain). Ne pas quémander d'upvotes depuis ces canaux ; raconter le lancement, c'est tout.
- **Article de blog paneflow.dev** : PAS un prérequis du Show HN (HN pointe vers le repo). À écrire en J+1 ou après, comme contenu durable (SEO/AEO) et surtout comme pivot du post LinkedIn FR. Ne pas s'y disperser le jour du lancement.

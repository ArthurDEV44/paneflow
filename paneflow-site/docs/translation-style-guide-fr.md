# Paneflow — Guide de style français

**Version :** 1.0
**Auteur :** Claude (AI agent), validé par Arthur Jean
**Source PRD :** `tasks/prd-i18n-fr-zh-Hans.md` section 6.1
**Mémoire de référence :** `feedback_brand_paneflow.md`, `feedback_simple_hyphens.md`
**Champ d'application :** toutes les traductions vers le français pour `messages/fr.json` (US-008), et toute future copie marketing FR du site `paneflow.dev`.

Ce document est normatif. US-008 doit le suivre à la lettre. US-010 (QA) l'utilise comme grille de référence.

---

## 1. Forme d'adresse

**`vous` partout.** Jamais `tu`.

Registre dev marketing professionnel, aligné sur la copie française de Linear, Vercel, Stripe. Le `tu` casse la cohérence et exclut une partie de l'audience B2B.

Cela vaut aussi pour les CTA : « Téléchargez Paneflow », pas « Télécharge Paneflow ».

---

## 2. Marque

**« Paneflow »** — toujours en un seul mot avec un P majuscule unique. Jamais « PaneFlow », jamais « paneflow » en milieu de phrase (les URL et les blocs de code sont exemptés), jamais traduit ni adapté phonétiquement.

Quand la marque apparaît au milieu d'une phrase, elle reste le token littéral `Paneflow` dans la valeur JSON. Les valeurs JSON ne contiennent jamais de placeholder `{brand}` — c'est une décision figée dans le catalogue de US-005.

Exemple correct : `« Paneflow lance vos sessions d'agents en parallèle. »`
Incorrect : `« paneflow lance... »`, `« PaneFlow lance... »`, `« Le multiplexeur lance... »` (perte du signal de marque).

---

## 3. Ton

**Direct, dense, sans inflation marketing.** Le ton miroir celui d'Arthur Jean (cf. `feedback_simple_hyphens` et les conventions `meta-speak`). Phrases courtes. Verbes actifs.

À éviter :

- « C'est l'outil qui vous permettra de… » (verbiage).
- « Découvrez X… » quand la source emploie un impératif direct.
- « N'hésitez pas à… » (formule creuse).
- Toute superlative non justifiée (« le meilleur », « ultime », « révolutionnaire »).

Préférer :

- « Lancez vos agents en parallèle. »
- « Reprenez votre session en une commande. »
- « Vos données restent locales. »

Si la source anglaise est neutre/descriptive, garder le même registre en français. Ne pas « marketer » une phrase technique.

---

## 4. Glossaire FR

Tous les termes ci-dessous sont **canoniques**. Les variantes orthographiques sont à proscrire (US-010 vérifie automatiquement).

| Anglais (source) | Français (canonique) |
|---|---|
| Terminal multiplexer | Multiplexeur de terminal |
| Terminal | Terminal |
| AI agents | Agents IA |
| Agent | Agent |
| Panes | Panneaux |
| Pane (singular) | Panneau |
| Splits | Divisions |
| Split (singular) | Division |
| Workspaces | Espaces de travail |
| Workspace (singular) | Espace de travail |
| Tabs | Onglets |
| Tab (singular) | Onglet |
| Drop-in replacement | Alternative directe |
| Open source | Open source (anglicisme accepté) |
| Self-hosted | Auto-hébergé |
| Built with Rust | Construit en Rust |
| Free and open source | Gratuit et open source |
| Get started | Commencer |
| Download | Télécharger |
| Compare | Comparer |
| Documentation | Documentation |
| Docs (short) | Docs |
| Coming soon | Bientôt disponible |
| Roadmap | Feuille de route |
| Release notes | Notes de version |
| Waitlist | Liste d'attente |
| Privacy policy | Politique de confidentialité |
| Terms of service | Conditions d'utilisation |
| GPU-accelerated | Accéléré par GPU |
| Cross-platform | Multi-plateforme |
| Native | Natif |
| Lightweight | Léger |
| Branch-aware | Conscient des branches |
| Session restore | Reprise de session |
| Dev server | Serveur de dev |
| Coding agent | Agent de codage |
| CLI agent | Agent CLI |
| Parallel | En parallèle (locution) / parallèle (adjectif) |
| Keyboard navigation | Navigation au clavier |

**Note diacritiques :** toutes les valeurs JSON de `messages/fr.json` doivent porter les accents (`é`, `è`, `à`, `ù`, `ç`, `ô`, `î`, `â`, `û`, etc.) en UTF-8 littéral. Ne jamais convertir en HTML entities (`&eacute;`) — les fichiers JSON sont déjà en UTF-8 et next-intl restitue tel quel. La sortie de `file messages/fr.json` doit indiquer `UTF-8 Unicode text`.

---

## 5. Ponctuation et typographie

### 5.1 Tirets

**Tiret ASCII `-` uniquement.** Jamais de cadratin `—` ni de demi-cadratin `–`.

Cette règle est invariante sur tout le projet (mémoire : `feedback_simple_hyphens.md`). Elle s'applique aux titres, listes, parenthèses et incises.

Correct : `« Paneflow gère vos agents - en parallèle, en local. »`
Incorrect : `« Paneflow gère vos agents — en parallèle... »`

### 5.2 Espaces insécables

La typographie française exige une **espace insécable avant** `:`, `;`, `?`, `!`, `%`, `»` et **après** `«`.

Dans les valeurs JSON, encoder l'espace insécable littéralement (caractère U+00A0). Exemple :

```json
"cta": "Téléchargez maintenant : c'est gratuit."
```

(L'espace entre « maintenant » et `:` est un U+00A0, pas un `space` U+0020. Vérifier avec `hexdump -C messages/fr.json | grep c2a0` qui doit retourner au moins une occurrence.)

Quand l'espace doit explicitement ne pas casser (CTA, label de bouton, étiquette UI étroite), utiliser `&nbsp;` à l'intérieur d'un fragment de rich text :

```json
"badge": "Open <strong>source&nbsp;100%</strong>"
```

(Le `&nbsp;` est résolu par `t.rich` côté React.)

### 5.3 Guillemets

**Guillemets français `« … »`** en prose courante. Espace insécable à l'intérieur (`« texte »`, pas `«texte»`).

Guillemets droits `"…"` ou `'…'` uniquement à l'intérieur de blocs de code ou d'identifiants techniques.

### 5.4 Apostrophes

Apostrophe ASCII `'` dans les valeurs JSON (compatibilité, lisibilité diff). Pas d'apostrophe typographique `'` — sauf si la phrase est destinée à un contexte purement éditorial sans contrainte de copier-coller.

### 5.5 Trois petits points

Caractère composé `...` (trois points ASCII), pas le caractère ellipse `…`. Cohérent avec la règle hyphen.

---

## 6. Constructions à proscrire

### 6.1 Calques de l'anglais

Éviter les empilements nominaux à l'anglaise :

- ❌ « fonctionnalité de gestion de projet »
- ✅ « gestion de projet »

- ❌ « Système de surveillance des sessions »
- ✅ « Supervision des sessions » ou « Surveillance des sessions »

### 6.2 Gérondifs marketing

Quand la source anglaise est un **nom** ou un **état**, ne pas convertir en gérondif français.

- Source : « Loading panes » (action en cours)
- ❌ « Chargeant les panneaux »
- ✅ « Chargement des panneaux »

- Source : « Running agents » (état)
- ❌ « Exécutant des agents »
- ✅ « Agents en cours d'exécution » ou simplement « Agents actifs »

### 6.3 « L'outil qui… »

Formule de remplissage à supprimer.

- ❌ « Paneflow est l'outil qui vous permet de gérer vos agents. »
- ✅ « Paneflow supervise vos agents. »

### 6.4 « Découvrez X »

Quand la source est un verbe d'action direct, garder l'impératif.

- Source : « Try Paneflow »
- ❌ « Découvrez Paneflow »
- ✅ « Essayez Paneflow » ou « Lancez Paneflow »

### 6.5 « N'hésitez pas à »

Toujours superflu.

- ❌ « N'hésitez pas à ouvrir une issue. »
- ✅ « Ouvrez une issue. »

### 6.6 Anglicismes inutiles

Quand un terme français standard existe, l'utiliser.

- ❌ « Process en arrière-plan »
- ✅ « Processus en arrière-plan »

- ❌ « Setup rapide »
- ✅ « Installation rapide » ou « Configuration rapide »

Exceptions tolérées (par usage tech dev) : `open source`, `dev server`, `pull request`, `commit`, `branch` (mais préférer « branche » en prose), `feature` (mais préférer « fonctionnalité » en prose), `bug`, `fix`.

---

## 7. Exemples EN → FR

### Exemple 1 — Hero headline

**Source EN :**
> A terminal workspace for orchestrating Claude Code, Codex, OpenCode, and custom CLI agents.

**FR canonique :**
> Un espace de travail terminal pour orchestrer Claude Code, Codex, OpenCode et vos agents CLI personnalisés.

Notes :
- `terminal workspace` → `espace de travail terminal` (glossaire).
- `custom CLI agents` → `vos agents CLI personnalisés` (`vous`-form implicite via `vos`, conserve la concision).
- Pas de virgule devant `et` à la française.
- Marques Claude Code, Codex, OpenCode : non traduites.

### Exemple 2 — CTA bouton

**Source EN :**
> Download Paneflow

**FR canonique :**
> Télécharger Paneflow

Notes :
- Infinitif (convention française pour les CTA boutons), pas impératif.
- Brand inchangée.

### Exemple 3 — Phrase avec ponctuation

**Source EN :**
> Free and open source: install in 30 seconds.

**FR canonique :**
> Gratuit et open source : installation en 30 secondes.

Notes :
- Espace insécable U+00A0 avant `:`.
- `install` (verbe action) → `installation` (nom) pour la fluidité.
- `open source` conservé (glossaire).

### Exemple 4 — Rich text avec placeholder

**Source EN (JSON value) :**
> Built with `<strong>Rust</strong>`, runs on Linux, macOS, and Windows.

**FR canonique (JSON value) :**
> Construit en `<strong>Rust</strong>`, fonctionne sur Linux, macOS et Windows.

Notes :
- `Built with Rust` → `Construit en Rust` (glossaire).
- Placeholder `<strong>` préservé tel quel.
- Liste FR : `Linux, macOS et Windows` (pas de virgule d'Oxford avant `et`).

### Exemple 5 — Phrase avec interpolation ICU

**Source EN (JSON value) :**
> You're in. We'll email you at `<strong>{email}</strong>`.

**FR canonique :**
> C'est noté. Vous recevrez un message à l'adresse `<strong>{email}</strong>`.

Notes :
- `You're in` → `C'est noté` (ton direct, idiomatique, sans calque).
- `vous`-form respecté.
- Placeholder `{email}` préservé.
- Apostrophe droite `'`.
- Pas d'espace insécable nécessaire ici (pas de `:` `;` `?` `!` `%`).

### Exemple 6 — Section heading

**Source EN :**
> Compare Paneflow vs other terminal workspaces

**FR canonique :**
> Comparer Paneflow et les autres espaces de travail terminal

Notes :
- `vs` → `et` (lisibilité).
- Pas de majuscules en titre (convention française vs Title Case anglaise).
- `terminal workspaces` → `espaces de travail terminal` (pluriel cohérent avec le glossaire singulier `espace de travail`).

### Exemple 7 — Liste de bénéfices

**Source EN :**
> - One pane per agent. Resize, navigate, focus from the keyboard.
> - One workspace per task. Restore everything after a restart.
> - Branch-aware sessions. Switch context without losing state.

**FR canonique :**
> - Un panneau par agent. Redimensionnez, naviguez et focalisez au clavier.
> - Un espace de travail par tâche. Restaurez tout après un redémarrage.
> - Sessions conscientes des branches. Changez de contexte sans perdre l'état.

Notes :
- `vous`-form sur les verbes d'action.
- `pane` → `panneau`, `workspace` → `espace de travail`, `branch-aware` → `conscient des branches` (glossaire).
- Listes : pas de virgule d'Oxford.

### Exemple 8 — Texte légal court

**Source EN :**
> By signing up, you accept our terms of service and privacy policy.

**FR canonique :**
> En vous inscrivant, vous acceptez nos conditions d'utilisation et notre politique de confidentialité.

Notes :
- `vous`-form (registre légal).
- `terms of service` → `conditions d'utilisation`, `privacy policy` → `politique de confidentialité` (glossaire).
- Pas d'espace insécable avant `,` (les virgules françaises n'en demandent pas).

---

## 8. Process de relecture

1. Première passe : traduction de chaque clé de `messages/en.json` vers `messages/fr.json` en suivant ce guide.
2. Deuxième passe : Arthur (locuteur natif) relit l'intégralité du fichier sur un déploiement Vercel preview, signale les phrases non idiomatiques ou off-tone.
3. Corrections appliquées, commit.
4. US-010 exécute le script `scripts/check-translations.ts` qui vérifie :
   - parité de clés en/fr,
   - absence de valeurs en anglais (sauf marques et termes glossaire-accepted comme `open source`),
   - présence de caractères accentués (`é`, `è`, `à`, `ç` au moins une occurrence chacun, en pratique des dizaines),
   - validité UTF-8.

Si l'une de ces vérifications échoue, le merge est bloqué.

---

## 9. Liens

- PRD source : `tasks/prd-i18n-fr-zh-Hans.md`
- Catalogue source : `messages/en.json`
- Mémoire : `feedback_brand_paneflow.md`, `feedback_simple_hyphens.md`, `feedback_linkedin_language.md`
- Script QA : `scripts/check-translations.ts` (livré en US-010)

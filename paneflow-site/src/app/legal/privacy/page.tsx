import type { Metadata } from "next";
import { Navbar } from "@/components/navbar";
import { Footer } from "@/components/footer";

export const metadata: Metadata = {
  title: "Politique de confidentialité — PaneFlow",
  description:
    "Politique de confidentialité de PaneFlow : données collectées sur paneflow.dev, télémétrie desktop opt-in, sous-traitants (PostHog EU, Vercel, GitHub), droits RGPD.",
  alternates: {
    canonical: "/legal/privacy",
    // hreflang signal — emits <link rel="alternate" hreflang="fr-FR" ...>.
    // Self-referencing because we do not yet host an English translation
    // of this page; the entry declares French as the page's language.
    languages: {
      "fr-FR": "/legal/privacy",
    },
  },
  openGraph: {
    title: "Politique de confidentialité — PaneFlow",
    description:
      "PaneFlow, site et application : données, sous-traitants, et droits RGPD.",
    type: "website",
  },
  robots: {
    // Legal pages are not ranking pages — let them be indexed for
    // transparency but avoid thin-content SEO noise. `index: true`
    // is the default; no overrides necessary here.
  },
};

// BreadcrumbList JSON-LD (US-011). Intermediate "/legal" omitted per
// the AC4 logged decision: there is no /legal page yet, and Google
// warns on non-resolving breadcrumb items. The position-2 label is in
// French to match the page's content language (matches the <main lang="fr">
// + hreflang fr-FR signals shipped in US-008).
const breadcrumbSchema = {
  "@context": "https://schema.org",
  "@type": "BreadcrumbList",
  itemListElement: [
    {
      "@type": "ListItem",
      position: 1,
      name: "Home",
      item: "https://paneflow.dev",
    },
    {
      "@type": "ListItem",
      position: 2,
      name: "Politique de confidentialité",
      item: "https://paneflow.dev/legal/privacy",
    },
  ],
};

// Last substantive update to the sub-processor list, retention, or
// data-subject rights procedure. Update this date whenever the content
// changes — it is the one piece of mutable copy on this page.
const LAST_UPDATED = "2026-04-23";

export default function PrivacyPage() {
  return (
    <>
      <script
        type="application/ld+json"
        dangerouslySetInnerHTML={{
          __html: JSON.stringify(breadcrumbSchema),
        }}
      />
      <Navbar />
      <main lang="fr">
        <section className="pt-36 sm:pt-40 pb-24">
          <div className="max-w-3xl mx-auto px-6">
            <header className="mb-12">
              <h1 className="text-3xl sm:text-4xl font-semibold tracking-tight">
                Politique de confidentialité
              </h1>
              <p className="mt-3 text-text-muted">
                Dernière mise à jour&nbsp;: {LAST_UPDATED}
              </p>
            </header>

            <div className="space-y-12 text-text-muted leading-relaxed">
              <Section title="1. Responsable du traitement">
                <p>
                  Le responsable du traitement au sens de l&rsquo;article&nbsp;4
                  du RGPD est&nbsp;:
                </p>
                <ul className="mt-3 list-disc pl-6 space-y-1">
                  <li>Strivex</li>
                  <li>
                    Contact&nbsp;:{" "}
                    <a
                      href="mailto:arthur.jean@strivex.fr"
                      className="text-text hover:text-accent-warm underline underline-offset-4"
                    >
                      arthur.jean@strivex.fr
                    </a>
                  </li>
                </ul>
              </Section>

              <Section title="2. Données collectées sur paneflow.dev">
                <p>
                  Le site <span className="font-mono text-text">paneflow.dev</span>{" "}
                  utilise deux outils d&rsquo;analytique complémentaires&nbsp;:
                </p>
                <ul className="mt-3 list-disc pl-6 space-y-2">
                  <li>
                    <strong className="text-text">Vercel Analytics</strong>{" "}
                    — métriques d&rsquo;infrastructure (pages vues, Web Vitals)
                    fournies par l&rsquo;hébergeur.
                  </li>
                  <li>
                    <strong className="text-text">PostHog</strong> en mode{" "}
                    <em>cookieless</em> — aucun cookie n&rsquo;est déposé, aucun
                    identifiant n&rsquo;est persisté dans le navigateur.
                    Sont collectés&nbsp;: pages vues anonymes, clics sur les
                    boutons de téléchargement, défilements jusqu&rsquo;aux
                    sections clés, paramètres UTM et{" "}
                    <span className="font-mono text-text">referer</span>{" "}
                    d&rsquo;arrivée. Les adresses IP ne sont{" "}
                    <strong className="text-text">jamais stockées</strong>{" "}
                    (masquage activé côté serveur PostHog).
                  </li>
                </ul>
                <p className="mt-3">
                  Aucune donnée nominative (nom, e-mail, identifiant de compte)
                  n&rsquo;est collectée par le site. Aucun profilage publicitaire
                  n&rsquo;est effectué.
                </p>
              </Section>

              <Section title="3. Télémétrie de l'application desktop (opt-in)">
                <p>
                  L&rsquo;application PaneFlow installée sur votre poste ne
                  transmet <strong className="text-text">aucune donnée</strong>{" "}
                  tant que vous n&rsquo;y avez pas explicitement consenti via la
                  fenêtre de consentement affichée au premier lancement.
                </p>
                <p className="mt-3">Si vous acceptez, les événements suivants sont transmis&nbsp;:</p>
                <ul className="mt-3 list-disc pl-6 space-y-1">
                  <li>
                    <span className="font-mono text-text">app_started</span>{" "}
                    — démarrage de l&rsquo;application
                  </li>
                  <li>
                    <span className="font-mono text-text">app_exited</span>{" "}
                    — arrêt de l&rsquo;application
                  </li>
                  <li>
                    <span className="font-mono text-text">update_installed</span>{" "}
                    — installation d&rsquo;une mise à jour
                  </li>
                </ul>
                <p className="mt-3">
                  Chaque événement est associé à un identifiant machine
                  anonyme (UUID&nbsp;v4 aléatoire, stocké localement et
                  réinitialisable par l&rsquo;utilisateur), au système
                  d&rsquo;exploitation et à la version de PaneFlow.{" "}
                  <strong className="text-text">
                    Aucun chemin de fichier, nom de projet, contenu de terminal,
                    commande shell ou information personnelle n&rsquo;est
                    transmis.
                  </strong>
                </p>
                <p className="mt-3">
                  La variable d&rsquo;environnement{" "}
                  <span className="font-mono text-text">PANEFLOW_NO_TELEMETRY=1</span>{" "}
                  désactive toute télémétrie de manière inconditionnelle, avant
                  même l&rsquo;affichage de la fenêtre de consentement. La
                  fenêtre de consentement peut être ré-ouverte à tout moment
                  depuis les préférences de l&rsquo;application pour réviser
                  votre choix.
                </p>
              </Section>

              <Section title="4. Sous-traitants">
                <p>
                  Conformément à l&rsquo;article&nbsp;28 du RGPD, PaneFlow
                  recourt aux sous-traitants suivants&nbsp;:
                </p>
                <div className="mt-3 space-y-4">
                  <SubProcessor
                    name="PostHog Inc."
                    region="EU Cloud — AWS Frankfurt (Allemagne)"
                    role="Analytique produit (site et télémétrie desktop opt-in)"
                    notes="Mode cookieless, adresses IP non stockées, rétention 12 mois, DPA signé avec SCC module 2 (contrôleur-processeur)."
                  />
                  <SubProcessor
                    name="Vercel Inc."
                    region="États-Unis (edge réseau en Europe)"
                    role="Hébergement statique du site paneflow.dev et Web Vitals"
                    notes="Accepté au moment de l&rsquo;onboarding Vercel. Aucun transfert de données personnelles identifiantes."
                  />
                  <SubProcessor
                    name="GitHub Inc. (Microsoft Corporation)"
                    region="États-Unis"
                    role="Hébergement du code source et des artefacts de version téléchargés depuis le site"
                    notes="Les logs de téléchargement relèvent des conditions de service GitHub."
                  />
                </div>
              </Section>

              <Section title="5. Durée de conservation">
                <ul className="list-disc pl-6 space-y-1">
                  <li>PostHog&nbsp;: 12&nbsp;mois glissants</li>
                  <li>Vercel Analytics&nbsp;: 30&nbsp;jours (valeur par défaut)</li>
                  <li>
                    GitHub&nbsp;: durée déterminée par la politique GitHub pour
                    les logs d&rsquo;accès aux artefacts publics
                  </li>
                </ul>
                <p className="mt-3">
                  Au-delà, les données sont supprimées ou agrégées de manière
                  anonyme.
                </p>
              </Section>

              <Section title="6. Droits des personnes concernées">
                <p>
                  Conformément au RGPD (articles&nbsp;15 à&nbsp;21), vous
                  disposez des droits suivants sur les données vous
                  concernant&nbsp;:
                </p>
                <ul className="mt-3 list-disc pl-6 space-y-1">
                  <li>
                    <strong className="text-text">Article&nbsp;15</strong> —
                    Droit d&rsquo;accès&nbsp;: obtenir la confirmation que des
                    données vous concernant sont traitées, et en recevoir une
                    copie.
                  </li>
                  <li>
                    <strong className="text-text">Article&nbsp;16</strong> —
                    Droit de rectification&nbsp;: faire corriger des données
                    inexactes ou incomplètes.
                  </li>
                  <li>
                    <strong className="text-text">Article&nbsp;17</strong> —
                    Droit à l&rsquo;effacement («&nbsp;droit à
                    l&rsquo;oubli&nbsp;»)&nbsp;: faire supprimer vos données
                    lorsque les conditions légales sont réunies.
                  </li>
                  <li>
                    <strong className="text-text">Article&nbsp;18</strong> —
                    Droit à la limitation du traitement&nbsp;: faire suspendre
                    temporairement le traitement.
                  </li>
                  <li>
                    <strong className="text-text">Article&nbsp;19</strong> —
                    Obligation de notification&nbsp;: être informé des
                    rectifications, effacements ou limitations communiqués aux
                    destinataires des données.
                  </li>
                  <li>
                    <strong className="text-text">Article&nbsp;20</strong> —
                    Droit à la portabilité&nbsp;: recevoir vos données dans un
                    format structuré, couramment utilisé et lisible par
                    machine, et les transmettre à un autre responsable.
                  </li>
                  <li>
                    <strong className="text-text">Article&nbsp;21</strong> —
                    Droit d&rsquo;opposition&nbsp;: vous opposer à tout moment
                    au traitement de vos données pour des raisons tenant à
                    votre situation particulière.
                  </li>
                </ul>
                <p className="mt-3">
                  Toute demande d&rsquo;exercice de ces droits est à adresser
                  par courriel à{" "}
                  <a
                    href="mailto:arthur.jean@strivex.fr"
                    className="text-text hover:text-accent-warm underline underline-offset-4"
                  >
                    arthur.jean@strivex.fr
                  </a>
                  . Une réponse vous sera apportée dans un délai maximal
                  d&rsquo;un mois. En cas de réponse insatisfaisante, vous
                  pouvez introduire une réclamation auprès de la{" "}
                  <a
                    href="https://www.cnil.fr/"
                    className="text-text hover:text-accent-warm underline underline-offset-4"
                  >
                    CNIL
                  </a>
                  .
                </p>
              </Section>

              <Section title="7. Contact">
                <p>
                  Pour toute question relative à la présente politique ou au
                  traitement de vos données&nbsp;:
                </p>
                <ul className="mt-3 list-disc pl-6 space-y-1">
                  <li>
                    E-mail&nbsp;:{" "}
                    <a
                      href="mailto:arthur.jean@strivex.fr"
                      className="text-text hover:text-accent-warm underline underline-offset-4"
                    >
                      arthur.jean@strivex.fr
                    </a>
                  </li>
                  <li>Éditeur&nbsp;: Strivex</li>
                </ul>
              </Section>
            </div>
          </div>
        </section>
      </main>
      <Footer />
    </>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section>
      <h2 className="text-xl sm:text-2xl font-semibold tracking-tight text-text mb-4">
        {title}
      </h2>
      <div className="space-y-3">{children}</div>
    </section>
  );
}

function SubProcessor({
  name,
  region,
  role,
  notes,
}: {
  name: string;
  region: string;
  role: string;
  notes: string;
}) {
  return (
    <div className="rounded-xl border border-surface-border bg-surface/30 p-4">
      <div className="text-text font-medium">{name}</div>
      <div className="mt-1 text-sm">
        <span className="text-text-subtle">Localisation&nbsp;:</span> {region}
      </div>
      <div className="mt-1 text-sm">
        <span className="text-text-subtle">Rôle&nbsp;:</span> {role}
      </div>
      <div className="mt-2 text-sm">{notes}</div>
    </div>
  );
}

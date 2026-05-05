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

const linkClass =
  "text-text hover:text-text-muted underline underline-offset-4 decoration-surface-border-hover";

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
        <section className="pt-32 sm:pt-40 pb-20 sm:pb-24">
          <div className="max-w-2xl mx-auto px-6">
            <header className="mb-10 sm:mb-12">
              <h1 className="text-2xl sm:text-3xl font-semibold tracking-tight">
                Politique de confidentialité
              </h1>
              <p className="mt-3 text-sm sm:text-base text-text-muted leading-relaxed">
                Dernière mise à jour&nbsp;: {LAST_UPDATED}
              </p>
            </header>

            <div className="space-y-10 text-sm sm:text-base text-text-muted leading-relaxed">
              <Section title="1. Responsable du traitement">
                <p>
                  Le responsable du traitement au sens de l&rsquo;article&nbsp;4
                  du RGPD est&nbsp;:
                </p>
                <ul className="mt-3 space-y-2">
                  <BulletItem>Strivex</BulletItem>
                  <BulletItem>
                    Contact&nbsp;:{" "}
                    <a
                      href="mailto:arthur.jean@strivex.fr"
                      className={linkClass}
                    >
                      arthur.jean@strivex.fr
                    </a>
                  </BulletItem>
                </ul>
              </Section>

              <Section title="2. Données collectées sur paneflow.dev">
                <p>
                  Le site{" "}
                  <span className="font-mono text-text">paneflow.dev</span>{" "}
                  utilise deux outils d&rsquo;analytique
                  complémentaires&nbsp;:
                </p>
                <ul className="mt-3 space-y-2.5">
                  <BulletItem>
                    <strong className="text-text font-semibold">
                      Vercel Analytics
                    </strong>
                    &nbsp;&middot; métriques d&rsquo;infrastructure (pages
                    vues, Web Vitals) fournies par l&rsquo;hébergeur.
                  </BulletItem>
                  <BulletItem>
                    <strong className="text-text font-semibold">
                      PostHog
                    </strong>
                    &nbsp;&middot; mode <em>cookieless</em>, aucun cookie
                    n&rsquo;est déposé, aucun identifiant n&rsquo;est persisté
                    dans le navigateur. Sont collectés&nbsp;: pages vues
                    anonymes, clics sur les boutons de téléchargement,
                    défilements jusqu&rsquo;aux sections clés, paramètres UTM
                    et{" "}
                    <span className="font-mono text-text">referer</span>{" "}
                    d&rsquo;arrivée. Les adresses IP ne sont{" "}
                    <strong className="text-text font-semibold">
                      jamais stockées
                    </strong>{" "}
                    (masquage activé côté serveur PostHog).
                  </BulletItem>
                </ul>
                <p className="mt-3">
                  Aucune donnée nominative (nom, e-mail, identifiant de compte)
                  n&rsquo;est collectée par le site. Aucun profilage
                  publicitaire n&rsquo;est effectué.
                </p>
              </Section>

              <Section title="3. Télémétrie de l'application desktop (opt-in)">
                <p>
                  L&rsquo;application Paneflow installée sur votre poste ne
                  transmet{" "}
                  <strong className="text-text font-semibold">
                    aucune donnée
                  </strong>{" "}
                  tant que vous n&rsquo;y avez pas explicitement consenti via
                  la fenêtre de consentement affichée au premier lancement.
                </p>
                <p className="mt-3">
                  Si vous acceptez, les événements suivants sont
                  transmis&nbsp;:
                </p>
                <ul className="mt-3 space-y-2">
                  <BulletItem>
                    <span className="font-mono text-text">app_started</span>
                    &nbsp;&middot; démarrage de l&rsquo;application
                  </BulletItem>
                  <BulletItem>
                    <span className="font-mono text-text">app_exited</span>
                    &nbsp;&middot; arrêt de l&rsquo;application
                  </BulletItem>
                  <BulletItem>
                    <span className="font-mono text-text">
                      update_installed
                    </span>
                    &nbsp;&middot; installation d&rsquo;une mise à jour
                  </BulletItem>
                </ul>
                <p className="mt-3">
                  Chaque événement est associé à un identifiant machine
                  anonyme (UUID&nbsp;v4 aléatoire, stocké localement et
                  réinitialisable par l&rsquo;utilisateur), au système
                  d&rsquo;exploitation et à la version de Paneflow.{" "}
                  <strong className="text-text font-semibold">
                    Aucun chemin de fichier, nom de projet, contenu de
                    terminal, commande shell ou information personnelle
                    n&rsquo;est transmis.
                  </strong>
                </p>
                <p className="mt-3">
                  La variable d&rsquo;environnement{" "}
                  <span className="font-mono text-text">
                    PANEFLOW_NO_TELEMETRY=1
                  </span>{" "}
                  désactive toute télémétrie de manière inconditionnelle,
                  avant même l&rsquo;affichage de la fenêtre de consentement.
                  La fenêtre de consentement peut être ré-ouverte à tout
                  moment depuis les préférences de l&rsquo;application pour
                  réviser votre choix.
                </p>
              </Section>

              <Section title="4. Sous-traitants">
                <p>
                  Conformément à l&rsquo;article&nbsp;28 du RGPD, Paneflow
                  recourt aux sous-traitants suivants&nbsp;:
                </p>
                <div className="mt-4 space-y-3">
                  <SubProcessor
                    name="PostHog Inc."
                    region="EU Cloud · AWS Frankfurt (Allemagne)"
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
                <ul className="space-y-2">
                  <BulletItem>PostHog&nbsp;: 12&nbsp;mois glissants</BulletItem>
                  <BulletItem>
                    Vercel Analytics&nbsp;: 30&nbsp;jours (valeur par défaut)
                  </BulletItem>
                  <BulletItem>
                    GitHub&nbsp;: durée déterminée par la politique GitHub
                    pour les logs d&rsquo;accès aux artefacts publics
                  </BulletItem>
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
                <ul className="mt-3 space-y-2.5">
                  <BulletItem>
                    <strong className="text-text font-semibold">
                      Article&nbsp;15
                    </strong>
                    &nbsp;&middot; Droit d&rsquo;accès&nbsp;: obtenir la
                    confirmation que des données vous concernant sont
                    traitées, et en recevoir une copie.
                  </BulletItem>
                  <BulletItem>
                    <strong className="text-text font-semibold">
                      Article&nbsp;16
                    </strong>
                    &nbsp;&middot; Droit de rectification&nbsp;: faire
                    corriger des données inexactes ou incomplètes.
                  </BulletItem>
                  <BulletItem>
                    <strong className="text-text font-semibold">
                      Article&nbsp;17
                    </strong>
                    &nbsp;&middot; Droit à l&rsquo;effacement («&nbsp;droit à
                    l&rsquo;oubli&nbsp;»)&nbsp;: faire supprimer vos données
                    lorsque les conditions légales sont réunies.
                  </BulletItem>
                  <BulletItem>
                    <strong className="text-text font-semibold">
                      Article&nbsp;18
                    </strong>
                    &nbsp;&middot; Droit à la limitation du
                    traitement&nbsp;: faire suspendre temporairement le
                    traitement.
                  </BulletItem>
                  <BulletItem>
                    <strong className="text-text font-semibold">
                      Article&nbsp;19
                    </strong>
                    &nbsp;&middot; Obligation de notification&nbsp;: être
                    informé des rectifications, effacements ou limitations
                    communiqués aux destinataires des données.
                  </BulletItem>
                  <BulletItem>
                    <strong className="text-text font-semibold">
                      Article&nbsp;20
                    </strong>
                    &nbsp;&middot; Droit à la portabilité&nbsp;: recevoir
                    vos données dans un format structuré, couramment utilisé
                    et lisible par machine, et les transmettre à un autre
                    responsable.
                  </BulletItem>
                  <BulletItem>
                    <strong className="text-text font-semibold">
                      Article&nbsp;21
                    </strong>
                    &nbsp;&middot; Droit d&rsquo;opposition&nbsp;: vous
                    opposer à tout moment au traitement de vos données pour
                    des raisons tenant à votre situation particulière.
                  </BulletItem>
                </ul>
                <p className="mt-3">
                  Toute demande d&rsquo;exercice de ces droits est à adresser
                  par courriel à{" "}
                  <a
                    href="mailto:arthur.jean@strivex.fr"
                    className={linkClass}
                  >
                    arthur.jean@strivex.fr
                  </a>
                  . Une réponse vous sera apportée dans un délai maximal
                  d&rsquo;un mois. En cas de réponse insatisfaisante, vous
                  pouvez introduire une réclamation auprès de la{" "}
                  <a href="https://www.cnil.fr/" className={linkClass}>
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
                <ul className="mt-3 space-y-2">
                  <BulletItem>
                    E-mail&nbsp;:{" "}
                    <a
                      href="mailto:arthur.jean@strivex.fr"
                      className={linkClass}
                    >
                      arthur.jean@strivex.fr
                    </a>
                  </BulletItem>
                  <BulletItem>Éditeur&nbsp;: Strivex</BulletItem>
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
      <h2 className="text-base sm:text-lg font-semibold tracking-tight text-text mb-3">
        {title}
      </h2>
      <div className="space-y-3">{children}</div>
    </section>
  );
}

function BulletItem({ children }: { children: React.ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="text-text-muted/60 select-none">-</span>
      <span>{children}</span>
    </li>
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
    <div className="rounded-lg border border-surface-border bg-bg-elevated p-4">
      <div className="text-text text-sm font-semibold">{name}</div>
      <div className="mt-1.5 text-sm">
        <span className="text-text-subtle">Localisation&nbsp;:</span> {region}
      </div>
      <div className="mt-1 text-sm">
        <span className="text-text-subtle">Rôle&nbsp;:</span> {role}
      </div>
      <div className="mt-2 text-sm">{notes}</div>
    </div>
  );
}

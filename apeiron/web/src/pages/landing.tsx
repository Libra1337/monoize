import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { motion } from "framer-motion";
import { ArrowRight, Coins, Layers, Play, Terminal } from "lucide-react";
import { api, type Template } from "@/lib/api";
import { easings } from "@/components/ui/motion";
import { PageWrapper, ScrollReveal, SectionKicker } from "@/components/ui/motion";
import { Button } from "@/components/ui/button";
import { ApeironLogo } from "@/components/logo";
import { GRID_TEXTURE } from "@/components/app-shell";
import { useAuth } from "@/auth";

function HeroTerminal() {
  const lines = [
    "$ POST /api/runs/oneclick",
    '{ "topic": "a 30s intro to our platform",',
    '  "material_mode": "stock",',
    '  "subtitle": true }',
    "→ script · storyboard · shots · voice · subs",
    "→ assemble: final.mp4",
  ];
  return (
    <motion.div
      initial={{ opacity: 0, y: 24 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.7, delay: 0.25, ease: easings.easeOutQuart }}
      className="relative rounded-lg border bg-card/90 p-4 shadow-sm"
    >
      <div className="mb-3 flex items-center gap-1.5">
        <span className="size-2.5 rounded-full bg-destructive/70" />
        <span className="size-2.5 rounded-full bg-warning/70" />
        <span className="size-2.5 rounded-full bg-success/70" />
      </div>
      <div className="space-y-1.5 font-mono text-xs leading-relaxed text-muted-foreground">
        {lines.map((line, index) => (
          <motion.div
            key={index}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ delay: 0.5 + index * 0.28, duration: 0.3 }}
          >
            <span className={index === 0 ? "text-foreground" : undefined}>{line}</span>
          </motion.div>
        ))}
        <span className="inline-block h-3.5 w-1.5 animate-[caret-blink_1.1s_steps(1)_infinite] bg-primary align-middle" />
      </div>
    </motion.div>
  );
}

export function LandingPage() {
  const { t } = useTranslation();
  const { me } = useAuth();
  const { data: templates } = useSWR("templates", () =>
    api.get<{ templates: Template[] }>("/templates"),
  );

  const features = [
    { icon: Play, title: t("landing.features.oneclickTitle"), body: t("landing.features.oneclickBody") },
    { icon: Layers, title: t("landing.features.canvasTitle"), body: t("landing.features.canvasBody") },
    { icon: Coins, title: t("landing.features.walletTitle"), body: t("landing.features.walletBody") },
    { icon: Terminal, title: t("landing.features.managerTitle"), body: t("landing.features.managerBody") },
  ];

  return (
    <div className="h-dvh overflow-y-auto bg-background">
      <header className="sticky top-0 z-40 border-b bg-background/92 px-4 backdrop-blur-xl sm:px-6">
        <div className="mx-auto flex h-14 max-w-7xl items-center justify-between">
          <div className="flex items-center gap-2.5">
            <span className="flex size-9 items-center justify-center rounded-lg border bg-card">
              <ApeironLogo className="h-5 w-5" />
            </span>
            <div className="leading-tight">
              <div className="font-display text-sm font-semibold tracking-tight">
                {t("brand.name")}
              </div>
              <div className="text-xs text-muted-foreground">{t("brand.tagline")}</div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {me?.platform_url ? (
              <Button variant="ghost" size="sm" onClick={() => window.open(me.platform_url, "_blank", "noreferrer")}>
                {t("nav.backToPlatform")}
              </Button>
            ) : null}
            <Link to="/create">
              <Button variant="primary" size="sm">
                {t("landing.ctaPrimary")}
                <ArrowRight />
              </Button>
            </Link>
          </div>
        </div>
      </header>

      <PageWrapper>
        {/* Hero */}
        <section className={GRID_TEXTURE}>
          <div className="mx-auto grid max-w-7xl items-center gap-10 px-4 py-20 sm:px-6 sm:py-28 lg:grid-cols-[1.08fr_0.92fr]">
            <div className="space-y-6">
              <div className="flex items-center gap-3 font-mono text-sm text-primary">
                <span className="h-px w-7 bg-primary/70" />
                <span>01 · {t("landing.kicker01")}</span>
              </div>
              <motion.h1
                initial={{ opacity: 0, y: 26, filter: "blur(10px)" }}
                animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
                transition={{ duration: 0.7, ease: easings.easeOutQuart }}
                className="font-display text-4xl font-semibold tracking-tight text-balance sm:text-6xl"
              >
                {t("landing.heroTitle")}
              </motion.h1>
              <p className="max-w-xl text-muted-foreground">{t("landing.heroDescription")}</p>
              <div className="flex flex-wrap gap-3">
                <Link to="/create">
                  <Button variant="primary" size="lg">
                    {t("landing.ctaPrimary")}
                    <ArrowRight />
                  </Button>
                </Link>
                <Link to="/projects">
                  <Button variant="outline" size="lg">
                    {t("landing.ctaCanvas")}
                  </Button>
                </Link>
              </div>
            </div>
            <HeroTerminal />
          </div>
        </section>

        {/* Templates band: hairline cell grid */}
        <section className="border-y bg-card">
          <div className="mx-auto max-w-7xl px-4 py-16 sm:px-6">
            <ScrollReveal className="mb-8 space-y-3">
              <SectionKicker index="02" label={t("landing.sectionTemplatesTitle")} />
              <h2 className="font-display text-2xl font-semibold tracking-tight sm:text-3xl">
                {t("landing.sectionTemplatesTitle")}
              </h2>
              <p className="text-sm text-muted-foreground">
                {t("landing.sectionTemplatesDescription")}
              </p>
            </ScrollReveal>
            <ScrollReveal delay={0.1}>
              <div className="grid grid-cols-1 border-l border-t sm:grid-cols-2 lg:grid-cols-4">
                {(templates?.templates ?? []).slice(0, 4).map((template) => (
                  <div key={template.id} className="border-b border-r p-5 transition-colors hover:bg-muted/30">
                    <div className="mb-3 flex h-8 w-8 items-center justify-center rounded-md bg-muted">
                      <Layers className="h-4 w-4 text-muted-foreground" />
                    </div>
                    <div className="text-sm font-medium">{template.name}</div>
                    <p className="mt-1 line-clamp-3 text-xs leading-relaxed text-muted-foreground">
                      {template.description}
                    </p>
                  </div>
                ))}
                {(templates?.templates ?? []).length === 0
                  ? [0, 1, 2, 3].map((index) => (
                      <div key={index} className="border-b border-r p-5">
                        <div className="h-8 w-8 animate-pulse rounded-md bg-muted" />
                        <div className="mt-3 h-4 w-24 animate-pulse rounded bg-muted" />
                        <div className="mt-2 h-3 w-full animate-pulse rounded bg-muted" />
                      </div>
                    ))
                  : null}
              </div>
            </ScrollReveal>
          </div>
        </section>

        {/* Features */}
        <section>
          <div className="mx-auto max-w-7xl px-4 py-16 sm:px-6">
            <ScrollReveal className="mb-8 space-y-3">
              <SectionKicker index="03" label={t("landing.sectionFeaturesTitle")} />
              <h2 className="font-display text-2xl font-semibold tracking-tight sm:text-3xl">
                {t("landing.sectionFeaturesTitle")}
              </h2>
            </ScrollReveal>
            <ScrollReveal delay={0.1}>
              <div className="grid grid-cols-1 border-l border-t sm:grid-cols-2 lg:grid-cols-4">
                {features.map((feature) => (
                  <div key={feature.title} className="border-b border-r p-5 transition-colors hover:bg-muted/30">
                    <feature.icon className="h-5 w-5 text-primary transition-transform group-hover:scale-110" />
                    <div className="mt-4 text-sm font-medium">{feature.title}</div>
                    <p className="mt-1.5 text-xs leading-relaxed text-muted-foreground">
                      {feature.body}
                    </p>
                  </div>
                ))}
              </div>
            </ScrollReveal>
          </div>
        </section>

        {/* CTA */}
        <section className="border-t bg-muted/35">
          <div className="mx-auto flex max-w-7xl flex-col items-center gap-4 px-4 py-16 text-center sm:px-6">
            <h2 className="font-display text-2xl font-semibold tracking-tight sm:text-3xl">
              {t("landing.sectionCtaTitle")}
            </h2>
            <p className="text-sm text-muted-foreground">{t("landing.sectionCtaBody")}</p>
            <Link to="/create">
              <Button variant="primary" size="lg">
                {t("landing.ctaPrimary")}
                <ArrowRight />
              </Button>
            </Link>
          </div>
        </section>

        <footer className="border-t">
          <div className="mx-auto flex max-w-7xl items-center justify-between px-4 py-6 text-xs text-muted-foreground sm:px-6">
            <span className="font-mono">Apeiron · powered by Monoize</span>
            {me?.platform_url ? (
              <a
                className="hover:text-foreground"
                href={me.platform_url}
                target="_blank"
                rel="noreferrer"
              >
                {t("nav.backToPlatform")}
              </a>
            ) : null}
          </div>
        </footer>
      </PageWrapper>
    </div>
  );
}

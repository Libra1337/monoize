import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { useAuth } from "@/auth";
import { ArrowRight, Coins, Play, Terminal } from "lucide-react";
import { ScrollReveal, SectionKicker } from "@/components/ui/motion";
import { Button } from "@/components/ui/button";

export function Features() {
  const { t } = useTranslation();
  const cells = [
    { icon: Play, title: t("landing.features.oneclickTitle"), body: t("landing.features.oneclickBody") },
    { icon: Terminal, title: t("landing.features.canvasTitle"), body: t("landing.features.canvasBody") },
    { icon: Coins, title: t("landing.features.walletTitle"), body: t("landing.features.walletBody") },
  ];
  return (
    <section>
      <div className="mx-auto max-w-5xl px-4 py-16 sm:px-6">
        <ScrollReveal className="mb-8 space-y-3">
          <SectionKicker index="04" label={t("landing.sectionFeaturesTitle")} />
          <h2 className="font-display text-2xl font-semibold tracking-tight text-balance sm:text-3xl">
            {t("landing.sectionFeaturesTitle")}
          </h2>
        </ScrollReveal>
        <ScrollReveal delay={0.1}>
          <div className="grid border-l border-t sm:grid-cols-3">
            {cells.map((cell) => (
              <div key={cell.title} className="border-b border-r p-5 transition-colors hover:bg-muted/20">
                <cell.icon className="h-5 w-5 text-primary" />
                <div className="mt-4 text-sm font-medium">{cell.title}</div>
                <p className="mt-1.5 text-xs leading-relaxed text-muted-foreground">{cell.body}</p>
              </div>
            ))}
          </div>
        </ScrollReveal>
      </div>
    </section>
  );
}

export function SiteFooter() {
  const { t } = useTranslation();
  const { me } = useAuth();
  return (
    <footer className="border-t border-border/60">
      <div className="mx-auto flex max-w-5xl flex-col items-center gap-4 px-4 py-14 text-center sm:px-6">
        <p className="font-display text-base font-medium text-muted-foreground text-balance">
          {t("landing.anaximander1")}
        </p>
        <Link to="/create">
          <Button variant="primary" size="lg">
            {t("landing.ctaPrimary")}
            <ArrowRight />
          </Button>
        </Link>
        <div className="mt-4 flex items-center gap-4 font-mono text-xs text-muted-foreground/70">
          <span>Apeiron · ἄπειρον</span>
          <span aria-hidden>·</span>
          <span>powered by Monoize</span>
          {me?.platform_url ? (
            <>
              <span aria-hidden>·</span>
              <a className="hover:text-foreground" href={me.platform_url} target="_blank" rel="noreferrer">
                {t("nav.backToPlatform")}
              </a>
            </>
          ) : null}
        </div>
      </div>
    </footer>
  );
}

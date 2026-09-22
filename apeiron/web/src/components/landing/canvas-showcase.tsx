import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { ArrowRight } from "lucide-react";
import { ScrollReveal, SectionKicker } from "@/components/ui/motion";
import { Button } from "@/components/ui/button";

/**
 * Real product capture: the workbench serving this very site, photographed
 * from a live session (apeiron/web/public/images/canvas-workbench.png).
 */
export function CanvasShowcase() {
  const { t } = useTranslation();
  return (
    <section>
      <div className="mx-auto max-w-5xl space-y-8 px-4 py-20 sm:px-6">
        <ScrollReveal className="space-y-4">
          <SectionKicker index="02" label={t("landing.canvasTitle")} />
          <h2 className="font-display text-2xl font-semibold tracking-tight text-balance sm:text-3xl">
            {t("landing.canvasTitle")}
          </h2>
          <p className="max-w-2xl leading-relaxed text-muted-foreground text-pretty">
            {t("landing.canvasBody")}
          </p>
        </ScrollReveal>
        <ScrollReveal delay={0.1}>
          <figure className="overflow-hidden rounded-lg border bg-card shadow-sm">
            <div className="flex items-center gap-1.5 border-b px-4 py-2.5">
              <span className="size-2.5 rounded-full bg-destructive/70" />
              <span className="size-2.5 rounded-full bg-warning/70" />
              <span className="size-2.5 rounded-full bg-success/70" />
              <span className="ml-3 font-mono text-xs text-muted-foreground">
                apeiron.lynshen.org/canvas
              </span>
            </div>
            <img
              src="/images/canvas-workbench.png"
              alt={t("landing.canvasAlt")}
              className="w-full"
              width={1440}
              height={900}
              loading="lazy"
            />
          </figure>
        </ScrollReveal>
        <ScrollReveal delay={0.15}>
          <Link to="/projects">
            <Button variant="outline">
              {t("landing.canvasCta")}
              <ArrowRight />
            </Button>
          </Link>
        </ScrollReveal>
      </div>
    </section>
  );
}

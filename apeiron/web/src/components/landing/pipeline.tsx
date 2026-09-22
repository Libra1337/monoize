import { useTranslation } from "react-i18next";
import { motion } from "framer-motion";
import { ScrollReveal, SectionKicker } from "@/components/ui/motion";

/** Six pipeline stages rendered as a hairline numbered strip. */
export function Pipeline() {
  const { t } = useTranslation();
  const stages = [
    "create.stages.script",
    "create.stages.storyboard",
    "create.stages.media",
    "create.stages.voice",
    "create.stages.subtitle",
    "create.stages.assemble",
  ];
  return (
    <section className="border-y border-border/60">
      <div className="mx-auto max-w-5xl px-4 py-16 sm:px-6">
        <ScrollReveal className="mb-8 space-y-3">
          <SectionKicker index="01" label={t("landing.pipelineTitle")} />
          <h2 className="font-display text-2xl font-semibold tracking-tight text-balance sm:text-3xl">
            {t("landing.pipelineTitle")}
          </h2>
        </ScrollReveal>
        <ScrollReveal delay={0.1}>
          <ol className="grid grid-cols-2 border-l border-t sm:grid-cols-3 lg:grid-cols-6">
            {stages.map((stage, index) => (
              <li
                key={stage}
                className="border-b border-r px-4 py-5 transition-colors hover:bg-muted/20"
              >
                <div className="font-mono text-xs text-primary">
                  {String(index + 1).padStart(2, "0")}
                </div>
                <div className="mt-2 text-sm font-medium leading-snug">{t(stage)}</div>
              </li>
            ))}
          </ol>
        </ScrollReveal>
        <ScrollReveal delay={0.15}>
          <motion.p
            initial={{ opacity: 0 }}
            whileInView={{ opacity: 1 }}
            viewport={{ once: true }}
            className="mt-6 font-mono text-xs leading-relaxed text-muted-foreground/80"
          >
            topic → script → storyboard → shots → media → voice → subtitles → assemble: final.mp4
          </motion.p>
        </ScrollReveal>
      </div>
    </section>
  );
}

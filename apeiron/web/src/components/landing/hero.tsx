import * as React from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { motion } from "framer-motion";
import { CornerDownLeft, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { easings } from "@/components/ui/motion";
import { useCreateTarget } from "@/lib/platform-entry";

/**
 * The house grid texture, drifting (hero backdrop). Line-based only: no
 * gradient blobs, per the design brief.
 */
const GRID_TEXTURE =
  "bg-[radial-gradient(circle_at_50%_0%,hsl(var(--primary)/0.10),transparent_42%),linear-gradient(to_right,hsl(var(--border)/0.5)_1px,transparent_1px),linear-gradient(to_bottom,hsl(var(--border)/0.5)_1px,transparent_1px)] bg-[size:auto,36px_36px,36px_36px] [mask-image:linear-gradient(to_bottom,black,transparent)]";

const rise = (delay: number) => ({
  initial: { opacity: 0, y: 22, filter: "blur(8px)" },
  animate: { opacity: 1, y: 0, filter: "blur(0px)" },
  transition: { duration: 0.8, ease: easings.easeOutQuart, delay },
});

export function Hero() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [topic, setTopic] = React.useState("");

  const inspirations = [t("landing.inspiration1"), t("landing.inspiration2"), t("landing.inspiration3")];

  const createTarget = useCreateTarget();

  function submit(value: string) {
    if (createTarget !== "/create") {
      window.location.href = createTarget;
      return;
    }
    const trimmed = value.trim();
    navigate(trimmed ? `/create?topic=${encodeURIComponent(trimmed)}` : "/create");
  }

  return (
    <section className={GRID_TEXTURE}>
      <div className="mx-auto flex max-w-4xl flex-col items-center gap-8 px-4 py-24 text-center sm:px-6 sm:py-32">
        <motion.div {...rise(0)} className="font-mono text-xs tracking-[0.3em] text-primary/90">
          {"ἄπειρον — THE BOUNDLESS"}
        </motion.div>

        <motion.p
          {...rise(0.15)}
          className="max-w-2xl font-display text-lg font-medium leading-relaxed text-muted-foreground text-balance sm:text-xl"
        >
          {t("landing.anaximander1")}
        </motion.p>

        <motion.h1
          {...rise(0.3)}
          className="max-w-3xl font-display text-4xl font-semibold leading-tight tracking-tight text-balance sm:text-6xl"
        >
          {t("landing.anaximander2")}
        </motion.h1>

        <motion.form
          {...rise(0.45)}
          className="flex w-full max-w-2xl items-center gap-2 rounded-lg border bg-card/90 p-2 pl-4 shadow-sm"
          onSubmit={(event) => {
            event.preventDefault();
            submit(topic);
          }}
        >
          <Sparkles className="h-4 w-4 shrink-0 text-muted-foreground" />
          <input
            value={topic}
            onChange={(event) => setTopic(event.target.value)}
            placeholder={t("landing.promptPlaceholder")}
            aria-label={t("landing.promptPlaceholder")}
            className="h-10 w-full bg-transparent text-sm placeholder:text-muted-foreground/70 focus-visible:outline-none"
          />
          <Button type="submit" variant="primary" className="shrink-0">
            <CornerDownLeft />
            <span className="hidden sm:inline">{t("landing.promptCta")}</span>
          </Button>
        </motion.form>

        <motion.div {...rise(0.6)} className="flex flex-wrap items-center justify-center gap-2">
          {inspirations.map((idea) => (
            <button
              key={idea}
              type="button"
              onClick={() => submit(idea)}
              className="rounded-md border px-3 py-1.5 text-xs text-foreground/75 transition-colors hover:border-primary/60 hover:text-foreground"
            >
              {idea}
            </button>
          ))}
        </motion.div>
      </div>
    </section>
  );
}

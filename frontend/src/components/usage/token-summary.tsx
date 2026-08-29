import { useEffect, useState } from "react";
import { animate, motion, useReducedMotion } from "framer-motion";
import { useTranslation } from "react-i18next";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  formatCacheHitRate,
  formatTokenCount,
  type ExactTokenTotals,
} from "@/lib/usage-analytics";

function AnimatedTokenValue({ value }: { value: bigint }) {
  const reduceMotion = useReducedMotion();
  const { i18n } = useTranslation();
  const [visible, setVisible] = useState(value);

  useEffect(() => {
    if (reduceMotion) {
      setVisible(value);
      return;
    }
    const controls = animate(0, 1, {
      duration: 0.24,
      ease: "easeOut",
      onUpdate: (progress) => {
        const step = BigInt(Math.round(progress * 1_000));
        setVisible((value * step) / 1_000n);
      },
    });
    return () => controls.stop();
  }, [reduceMotion, value]);

  return (
    <motion.span
      key={value.toString()}
      initial={reduceMotion ? false : { opacity: 0.45, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: reduceMotion ? 0 : 0.2 }}
      className="font-display tabular-nums"
    >
      {formatTokenCount(visible, i18n.language)}
    </motion.span>
  );
}

export function TokenSummary({
  totals,
  loading = false,
}: {
  totals?: ExactTokenTotals;
  loading?: boolean;
}) {
  const { t } = useTranslation();
  const cards = totals ? [
    { key: "input", label: t("usageAnalysis.metrics.input"), value: totals.input },
    { key: "cache", label: t("usageAnalysis.metrics.cacheRead"), value: totals.cacheRead },
    { key: "output", label: t("usageAnalysis.metrics.output"), value: totals.output },
    { key: "total", label: t("usageAnalysis.metrics.total"), value: totals.total },
  ] : [];

  if (loading && !totals) {
    return (
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4" aria-label={t("usageAnalysis.summary")}>
        {Array.from({ length: 4 }, (_, index) => (
          <Card key={index} className="rounded-lg">
            <CardContent className="p-5">
              <Skeleton className="h-4 w-24" />
              <Skeleton className="mt-4 h-8 w-32" />
            </CardContent>
          </Card>
        ))}
      </div>
    );
  }

  return (
    <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4" aria-label={t("usageAnalysis.summary")}>
      {cards.map((card, index) => (
        <motion.div
          key={card.key}
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.2, delay: index * 0.035 }}
        >
          <Card className="h-full rounded-lg">
            <CardContent className="p-5">
              <p className="text-sm text-muted-foreground">{card.label}</p>
              <p className="mt-2 min-w-0 break-words text-2xl font-semibold">
                <AnimatedTokenValue value={card.value} />
              </p>
              {card.key === "cache" && totals ? (
                <p className="mt-2 text-xs text-muted-foreground">
                  {t("usageAnalysis.cacheHitRate")}: {formatCacheHitRate(totals.input, totals.cacheRead)}
                </p>
              ) : null}
            </CardContent>
          </Card>
        </motion.div>
      ))}
    </div>
  );
}

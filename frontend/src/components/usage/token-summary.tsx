import { useEffect, useRef, useState } from "react";
import { animate, motion, useReducedMotion } from "framer-motion";
import { useTranslation } from "react-i18next";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  formatCacheHitRate,
  formatTokenCount,
  type ExactTokenTotals,
} from "@/lib/usage-analytics";

export function AnimatedTokenValue({ value, showDelta = false }: { value: bigint; showDelta?: boolean }) {
  const reduceMotion = useReducedMotion();
  const { i18n } = useTranslation();
  const [visible, setVisible] = useState(value);
  const visibleRef = useRef(value);
  const targetRef = useRef(value);
  const [delta, setDelta] = useState<bigint>(0n);

  useEffect(() => {
    const start = visibleRef.current;
    const previousTarget = targetRef.current;
    targetRef.current = value;
    setDelta(value - previousTarget);
    if (value === previousTarget) return;
    if (reduceMotion) {
      setVisible(value);
      visibleRef.current = value;
      return;
    }
    const controls = animate(0, 1, {
      duration: 1.35,
      ease: "easeOut",
      onUpdate: (progress) => {
        const step = BigInt(Math.round(progress * 1_000));
        const next = start + ((value - start) * step) / 1_000n;
        visibleRef.current = next;
        setVisible(next);
      },
    });
    return () => controls.stop();
  }, [reduceMotion, value]);

  return <span className="inline-flex flex-col">
    <motion.span
      initial={false}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: reduceMotion ? 0 : 0.25 }}
      className="font-display tabular-nums"
    >{formatTokenCount(visible, i18n.language)}</motion.span>
    {showDelta && delta !== 0n ? (
      <span className={`mt-1 text-xs font-medium tabular-nums ${delta > 0n ? "text-success" : "text-destructive"}`}>
        {delta > 0n ? "+" : "-"}{formatTokenCount(delta < 0n ? -delta : delta, i18n.language)}
      </span>
    ) : null}
  </span>;
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
      <Card className="overflow-hidden rounded-lg" aria-label={t("usageAnalysis.summary")}>
        <CardContent className="grid divide-y p-0 sm:grid-cols-4 sm:divide-x sm:divide-y-0">
          {Array.from({ length: 4 }, (_, index) => (
            <div key={index} className="min-w-0 p-5">
              <Skeleton className="h-4 w-24" />
              <Skeleton className="mt-4 h-8 w-32" />
            </div>
          ))}
        </CardContent>
      </Card>
    );
  }

  return (
    <Card className="overflow-hidden rounded-lg" aria-label={t("usageAnalysis.summary")}>
      <CardContent className="grid divide-y p-0 sm:grid-cols-4 sm:divide-x sm:divide-y-0">
        {cards.map((card, index) => (
          <motion.div
            key={card.key}
            className="min-w-0 p-5"
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.7, delay: index * 0.08 }}
          >
            <p className="text-sm text-muted-foreground">{card.label}</p>
            <p className="mt-2 min-w-0 break-words text-2xl font-semibold">
              <AnimatedTokenValue value={card.value} showDelta />
            </p>
            {card.key === "cache" && totals ? (
              <p className="mt-2 text-xs text-muted-foreground">
                {t("usageAnalysis.cacheHitRate")}: {formatCacheHitRate(totals.input, totals.cacheRead)}
              </p>
            ) : null}
          </motion.div>
        ))}
      </CardContent>
    </Card>
  );
}

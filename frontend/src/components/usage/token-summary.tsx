import { useEffect, useRef, useState } from "react";
import { AnimatePresence, animate, motion, useReducedMotion } from "framer-motion";
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
  const cycleStartRef = useRef(value);
  const cycleActiveRef = useRef(false);
  const [delta, setDelta] = useState<bigint>(0n);
  const [deltaVisible, setDeltaVisible] = useState(false);

  useEffect(() => {
    const previousTarget = targetRef.current;
    if (value === previousTarget) return;
    targetRef.current = value;
    if (reduceMotion) {
      setVisible(value);
      visibleRef.current = value;
      cycleStartRef.current = value;
      cycleActiveRef.current = false;
      setDelta(0n);
      setDeltaVisible(false);
      return;
    }
    if (!cycleActiveRef.current) {
      cycleStartRef.current = visibleRef.current;
      cycleActiveRef.current = true;
      setDelta(0n);
      setDeltaVisible(false);
    }
    const start = visibleRef.current;
    const controls = animate(0, 1, {
      duration: 7.2,
      ease: "easeInOut",
      onUpdate: (progress) => {
        const step = BigInt(Math.round(progress * 1_000));
        const next = start + ((value - start) * step) / 1_000n;
        visibleRef.current = next;
        setVisible(next);
        setDelta(next - cycleStartRef.current);
        setDeltaVisible(next !== cycleStartRef.current);
      },
      onComplete: () => {
        if (targetRef.current !== value) return;
        visibleRef.current = value;
        setVisible(value);
        cycleStartRef.current = value;
        cycleActiveRef.current = false;
        setDeltaVisible(false);
      },
    });
    return () => controls.stop();
  }, [reduceMotion, value]);

  return <span className="inline-flex items-baseline gap-1.5 whitespace-nowrap">
    <motion.span
      initial={false}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: reduceMotion ? 0 : 0.25 }}
      className="font-display tabular-nums"
    >{formatTokenCount(visible, i18n.language)}</motion.span>
    <AnimatePresence initial={false}>
      {showDelta && deltaVisible && delta !== 0n ? (
        <motion.span
          key="delta"
          initial={{ opacity: 0, y: 2 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -2 }}
          transition={{ duration: reduceMotion ? 0 : 0.35 }}
          className={`text-xs font-medium tabular-nums ${delta > 0n ? "text-success" : "text-destructive"}`}
        >
          {delta > 0n ? "+" : "-"}{formatTokenCount(delta < 0n ? -delta : delta, i18n.language)}
        </motion.span>
      ) : null}
    </AnimatePresence>
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

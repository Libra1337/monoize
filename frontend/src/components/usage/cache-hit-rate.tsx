import { useMemo } from "react";
import { useReducedMotion } from "framer-motion";
import { useTranslation } from "react-i18next";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useSelectionDataset } from "@/hooks/use-selection-dataset";
import {
  formatCacheHitRate,
  formatTokenCount,
  rankModelCacheHitRates,
  type CacheHitGrade,
  type TokenAnalyticsBucket,
} from "@/lib/usage-analytics";
import { cn } from "@/lib/utils";

const GRADE_TEXT: Record<CacheHitGrade, string> = {
  insufficient: "text-muted-foreground",
  low: "text-destructive",
  partial: "text-warning",
  high: "text-success",
};

const GRADE_BAR: Record<CacheHitGrade, string> = {
  insufficient: "bg-muted-foreground/40",
  low: "bg-destructive",
  partial: "bg-warning",
  high: "bg-success",
};

export function CacheHitRateByModel({
  buckets,
  selectionKey,
  loading = false,
}: {
  buckets?: TokenAnalyticsBucket[];
  selectionKey: string;
  loading?: boolean;
}) {
  const reduceMotion = useReducedMotion();
  const { t, i18n } = useTranslation();
  const nextRows = useMemo(() => rankModelCacheHitRates(buckets ?? []), [buckets]);
  const { dataset: rows, animate } = useSelectionDataset({
    selectionKey,
    loading,
    dataset: nextRows,
    animationDurationMs: 1000,
    enabled: !reduceMotion,
  });

  if (loading && !buckets) {
    return (
      <Card className="rounded-lg">
        <CardHeader className="gap-2">
          <Skeleton className="h-5 w-52" />
          <Skeleton className="h-4 w-80" />
        </CardHeader>
        <CardContent className="grid gap-4">
          {Array.from({ length: 3 }, (_, index) => (
            <div key={index} className="grid gap-2">
              <Skeleton className="h-4 w-64" />
              <Skeleton className="h-1.5 w-full" />
            </div>
          ))}
        </CardContent>
      </Card>
    );
  }

  return (
    <Card className="rounded-lg">
      <CardHeader>
        <CardTitle className="text-base">{t("usageAnalysis.cacheByModel.title")}</CardTitle>
        <CardDescription>{t("usageAnalysis.cacheByModel.description")}</CardDescription>
      </CardHeader>
      <CardContent>
        {rows.length === 0 ? (
          <div className="grid h-24 place-items-center">
            <p className="text-sm text-muted-foreground">{t("usageAnalysis.empty")}</p>
          </div>
        ) : (
          <div className="flex min-w-0 flex-col gap-4">
            {rows.map((row) => (
              <div key={row.model} className="min-w-0">
                <div className="flex min-w-0 flex-wrap items-baseline justify-between gap-x-3 gap-y-1 text-sm">
                  <span className="min-w-0 font-medium leading-5 [overflow-wrap:anywhere]">{row.model}</span>
                  <span className={cn("font-mono text-sm tabular-nums", GRADE_TEXT[row.grade])}>
                    {formatCacheHitRate(row.input, row.cacheRead)}
                    <span className="ml-2 text-xs font-sans">
                      {t(`usageAnalysis.cacheByModel.grades.${row.grade}`)}
                    </span>
                  </span>
                </div>
                <div className="mt-1 flex min-w-0 flex-wrap gap-x-1 font-mono text-xs tabular-nums text-muted-foreground">
                  <span>{t("usageAnalysis.metrics.cacheRead")}</span>
                  <span className="[overflow-wrap:anywhere]">{formatTokenCount(row.cacheRead, i18n.language)}</span>
                  <span aria-hidden="true">·</span>
                  <span>{t("usageAnalysis.metrics.input")}</span>
                  <span className="[overflow-wrap:anywhere]">{formatTokenCount(row.input, i18n.language)}</span>
                </div>
                <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted">
                  <div
                    className={cn(
                      "h-full rounded-full",
                      GRADE_BAR[row.grade],
                      animate && "transition-[width] duration-[1000ms] ease-in-out",
                    )}
                    style={{ width: `${Number(row.basisPoints) / 100}%` }}
                  />
                </div>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

import { useMemo } from "react";
import { useReducedMotion } from "framer-motion";
import { useTranslation } from "react-i18next";
import { Cell, Pie, PieChart } from "recharts";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ChartContainer, type ChartConfig } from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";
import {
  formatTokenCount,
  rankModelsByTokens,
  type TokenAnalyticsBucket,
  type TokenMetric,
} from "@/lib/usage-analytics";

const COLORS = Array.from({ length: 5 }, (_, index) => `hsl(var(--chart-${index + 1}))`);

export function ModelDistribution({
  buckets,
  metric,
  loading = false,
}: {
  buckets?: TokenAnalyticsBucket[];
  metric: TokenMetric;
  loading?: boolean;
}) {
  const reduceMotion = useReducedMotion();
  const { t, i18n } = useTranslation();
  const ranked = useMemo(() => rankModelsByTokens(buckets ?? [], metric), [buckets, metric]);
  const total = ranked.reduce((sum, row) => sum + row.value, 0n);
  const chartRows = ranked.slice(0, 5).map((row, index) => ({
    ...row,
    display: total === 0n ? 0 : Number((row.value * 10_000n) / total) / 100,
    fill: COLORS[index % COLORS.length],
  }));
  const config = Object.fromEntries(chartRows.map((row) => [
    row.model,
    { label: row.model, color: row.fill },
  ])) satisfies ChartConfig;

  if (loading && !buckets) {
    return (
      <Card className="rounded-lg">
        <CardHeader><Skeleton className="h-5 w-44" /></CardHeader>
        <CardContent className="grid gap-6 md:grid-cols-[220px_1fr]">
          <Skeleton className="mx-auto size-48 rounded-full" />
          <Skeleton className="h-48 w-full" />
        </CardContent>
      </Card>
    );
  }

  return (
    <Card className="rounded-lg">
      <CardHeader>
        <CardTitle className="text-base">{t("usageAnalysis.modelDistribution")}</CardTitle>
      </CardHeader>
      <CardContent>
        {ranked.length === 0 ? (
          <div className="grid h-52 place-items-center">
            <p className="text-sm text-muted-foreground">{t("usageAnalysis.empty")}</p>
          </div>
        ) : (
          <div className="grid items-center gap-6 md:grid-cols-[220px_minmax(0,1fr)]">
            <ChartContainer config={config} className="mx-auto size-52 !aspect-square">
              <PieChart>
                <Pie
                  data={chartRows}
                  dataKey="display"
                  nameKey="model"
                  innerRadius={58}
                  outerRadius={88}
                  strokeWidth={3}
                  isAnimationActive={!reduceMotion}
                  animationDuration={900}
                >
                  {chartRows.map((row) => <Cell key={row.model} fill={row.fill} />)}
                </Pie>
              </PieChart>
            </ChartContainer>
            <div className="flex min-w-0 flex-col gap-3">
              {ranked.map((row, index) => {
                const basisPoints = total === 0n ? 0n : (row.value * 10_000n + total / 2n) / total;
                const percentage = `${basisPoints / 100n}.${(basisPoints % 100n).toString().padStart(2, "0")}%`;
                return (
                  <div key={row.model} className="min-w-0">
                    <div className="flex min-w-0 items-center justify-between gap-4 text-sm">
                      <div className="flex min-w-0 items-center gap-2">
                        <span className="size-2.5 shrink-0 rounded-sm" style={{ backgroundColor: COLORS[index % COLORS.length] }} />
                        <span className="truncate font-medium">{row.model}</span>
                      </div>
                      <span className="shrink-0 font-mono text-xs tabular-nums text-muted-foreground">
                        {formatTokenCount(row.value, i18n.language)} · {percentage}
                      </span>
                    </div>
                    <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted">
                      <div
                        className="h-full rounded-full bg-primary transition-[width] duration-700"
                        style={{ width: `${Number(basisPoints) / 100}%` }}
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

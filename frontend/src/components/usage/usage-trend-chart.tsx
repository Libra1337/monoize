import { useMemo } from "react";
import { useReducedMotion } from "framer-motion";
import { useTranslation } from "react-i18next";
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";
import {
  formatTokenCount,
  tokenMetricForBucket,
  type TokenAnalyticsBucket,
  type TokenMetric,
} from "@/lib/usage-analytics";

const chartConfig = {
  usage: { label: "Tokens", color: "hsl(var(--primary))" },
} satisfies ChartConfig;

export function UsageTrendChart({
  buckets,
  metric,
  loading = false,
  compact = false,
}: {
  buckets?: TokenAnalyticsBucket[];
  metric: TokenMetric;
  loading?: boolean;
  compact?: boolean;
}) {
  const reduceMotion = useReducedMotion();
  const { t, i18n } = useTranslation();
  const rows = useMemo(() => {
    const exactRows = (buckets ?? []).map((bucket) => ({
      label: bucket.label,
      exact: tokenMetricForBucket(bucket, metric),
    }));
    const maximum = exactRows.reduce((max, row) => row.exact > max ? row.exact : max, 0n);
    return exactRows.map((row) => ({
      label: row.label,
      exact: row.exact.toString(),
      usage: maximum === 0n ? 0 : Number((row.exact * 10_000n) / maximum) / 100,
    }));
  }, [buckets, metric]);

  if (loading && !buckets) {
    return (
      <Card className="rounded-lg">
        <CardHeader><Skeleton className="h-5 w-40" /></CardHeader>
        <CardContent><Skeleton className={compact ? "h-48 w-full" : "h-72 w-full"} /></CardContent>
      </Card>
    );
  }

  return (
    <Card className="rounded-lg">
      <CardHeader className="flex-row items-center justify-between gap-4">
        <CardTitle className="text-base">{t("usageAnalysis.trend")}</CardTitle>
        <p className="text-xs text-muted-foreground">{t(`usageAnalysis.metricOptions.${metric}`)}</p>
      </CardHeader>
      <CardContent>
        {rows.some((row) => row.exact !== "0") ? (
          <ChartContainer
            config={chartConfig}
            className={compact ? "h-48 w-full !aspect-auto" : "h-72 w-full !aspect-auto"}
            aria-label={t("usageAnalysis.trend")}
          >
            <LineChart data={rows} margin={{ top: 8, right: 12, left: -20, bottom: 0 }}>
              <CartesianGrid vertical={false} />
              <XAxis dataKey="label" tickLine={false} axisLine={false} minTickGap={24} />
              <YAxis tickLine={false} axisLine={false} domain={[0, 100]} tickFormatter={(value) => `${value}%`} />
              <ChartTooltip
                content={(
                  <ChartTooltipContent
                    formatter={(_value, _name, item) => (
                      <div className="flex min-w-40 items-center justify-between gap-4">
                        <span className="text-muted-foreground">{t("usageAnalysis.tokens")}</span>
                        <span className="font-mono tabular-nums">
                          {formatTokenCount(BigInt(String(item.payload.exact)), i18n.language)}
                        </span>
                      </div>
                    )}
                  />
                )}
              />
              <Line
                type="monotone"
                dataKey="usage"
                stroke="var(--color-usage)"
                strokeWidth={2}
                dot={false}
                activeDot={{ r: 4 }}
                isAnimationActive={!reduceMotion}
                animationDuration={240}
              />
            </LineChart>
          </ChartContainer>
        ) : (
          <div className={compact ? "grid h-48 place-items-center" : "grid h-72 place-items-center"}>
            <p className="text-sm text-muted-foreground">{t("usageAnalysis.empty")}</p>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

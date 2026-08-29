import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper, SharedTabIndicator } from "@/components/ui/motion";
import { ModelDistribution } from "@/components/usage/model-distribution";
import { TokenSummary } from "@/components/usage/token-summary";
import { UsageTrendChart } from "@/components/usage/usage-trend-chart";
import { useDashboardAnalytics } from "@/lib/swr";
import { aggregateTokenTotals, type TokenMetric } from "@/lib/usage-analytics";
import { cn } from "@/lib/utils";

type UsageRange = "24h" | "7d" | "30d";

const USAGE_RANGES: Record<UsageRange, { hours: number; buckets: number }> = {
  "24h": { hours: 24, buckets: 24 },
  "7d": { hours: 168, buckets: 28 },
  "30d": { hours: 720, buckets: 30 },
};

const METRICS: TokenMetric[] = ["total", "input", "cache_read", "output"];

function SegmentedControl<T extends string>({
  values,
  value,
  label,
  layoutId,
  renderLabel,
  onChange,
}: {
  values: T[];
  value: T;
  label: string;
  layoutId: string;
  renderLabel: (value: T) => string;
  onChange: (value: T) => void;
}) {
  return (
    <div className="flex max-w-full overflow-x-auto rounded-lg bg-muted p-1" role="group" aria-label={label}>
      {values.map((option) => (
        <button
          key={option}
          type="button"
          aria-pressed={value === option}
          className={cn(
            "relative min-h-9 shrink-0 rounded-md px-3 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            value === option ? "text-foreground" : "text-muted-foreground hover:text-foreground",
          )}
          onClick={() => onChange(option)}
        >
          {value === option ? <SharedTabIndicator layoutId={layoutId} className="absolute inset-0 rounded-md bg-background shadow-sm" /> : null}
          <span className="relative z-10">{renderLabel(option)}</span>
        </button>
      ))}
    </div>
  );
}

export function UsageAnalysisPage() {
  const { t } = useTranslation();
  const [range, setRange] = useState<UsageRange>("7d");
  const [metric, setMetric] = useState<TokenMetric>("total");
  const config = USAGE_RANGES[range];
  const analytics = useDashboardAnalytics(config.buckets, config.hours, "self", {
    keepPreviousData: true,
    refreshInterval: 2000,
  });
  const totals = useMemo(
    () => analytics.data ? aggregateTokenTotals(analytics.data.buckets) : undefined,
    [analytics.data],
  );

  return (
    <PageWrapper className="flex min-w-0 flex-col gap-6 pb-6">
      <PageHeader
        title={t("usageAnalysis.title")}
        description={t("usageAnalysis.description")}
        actions={(
          <SegmentedControl
            values={Object.keys(USAGE_RANGES) as UsageRange[]}
            value={range}
            label={t("usageAnalysis.rangeLabel")}
            layoutId="usage-analysis-range"
            renderLabel={(option) => t(`usageAnalysis.ranges.${option}`)}
            onChange={setRange}
          />
        )}
      />

      {analytics.error && !analytics.data ? (
        <EmptyState
          title={t("usageAnalysis.loadFailed")}
          description={t("usageAnalysis.loadFailedDescription")}
          action={<Button variant="outline" onClick={() => void analytics.mutate()}>{t("common.retry")}</Button>}
        />
      ) : (
        <>
          <TokenSummary totals={totals} loading={analytics.isLoading} />
          <div className="flex justify-end">
            <SegmentedControl
              values={METRICS}
              value={metric}
              label={t("usageAnalysis.metricLabel")}
              layoutId="usage-analysis-metric"
              renderLabel={(option) => t(`usageAnalysis.metricOptions.${option}`)}
              onChange={setMetric}
            />
          </div>
          <div className="grid min-w-0 gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.65fr)]">
            <UsageTrendChart
              buckets={analytics.data?.buckets}
              metric={metric}
              loading={analytics.isLoading}
            />
            <ModelDistribution
              buckets={analytics.data?.buckets}
              metric={metric}
              loading={analytics.isLoading}
            />
          </div>
        </>
      )}
    </PageWrapper>
  );
}

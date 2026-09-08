import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import { ArrowRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CoinAmount } from "@/components/coin-amount";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { PageWrapper, SharedTabIndicator, motion } from "@/components/ui/motion";
import { TokenSummary } from "@/components/usage/token-summary";
import { UsageTrendChart } from "@/components/usage/usage-trend-chart";
import { useAuth } from "@/hooks/use-auth";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useDashboardAnalytics } from "@/lib/swr";
import { formatCoinFromNanoUsdForCurrency } from "@/lib/store-money";
import { aggregateTokenTotals, formatCacheHitRate } from "@/lib/usage-analytics";
import { cn } from "@/lib/utils";

type DashboardRange = "24h" | "week" | "month";

const DASHBOARD_RANGES: Record<DashboardRange, { hours: number; buckets: number }> = {
  "24h": { hours: 24, buckets: 24 },
  week: { hours: 168, buckets: 28 },
  month: { hours: 720, buckets: 30 },
};

function OverviewCard({
  title,
  rows,
  index,
}: {
  title: string;
  rows: Array<{ label: string; value: ReactNode }>;
  index: number;
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, delay: index * 0.04 }}
      whileHover={{ y: -1 }}
    >
      <Card className="h-full rounded-lg">
        <CardHeader className="p-4 pb-2">
          <CardTitle className="text-base">{title}</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-2.5 p-4 pt-0">
          {rows.map((row) => (
            <div key={row.label} className="rounded-md border bg-muted/25 px-3 py-2">
              <p className="text-xs text-muted-foreground">{row.label}</p>
              <p className="mt-1 min-w-0 break-words font-display text-base font-semibold tabular-nums">{row.value}</p>
            </div>
          ))}
        </CardContent>
      </Card>
    </motion.div>
  );
}

function RangeControl({
  value,
  onChange,
}: {
  value: DashboardRange;
  onChange: (value: DashboardRange) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex rounded-lg bg-muted p-1" role="group" aria-label={t("usageAnalysis.rangeLabel")}>
      {(Object.keys(DASHBOARD_RANGES) as DashboardRange[]).map((range) => (
        <button
          key={range}
          type="button"
          aria-pressed={value === range}
          className={cn(
            "relative min-h-9 rounded-md px-3 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            value === range ? "text-foreground" : "text-muted-foreground hover:text-foreground",
          )}
          onClick={() => onChange(range)}
        >
          {value === range ? <SharedTabIndicator layoutId="dashboard-token-range" className="absolute inset-0 rounded-md bg-background shadow-sm" /> : null}
          <span className="relative z-10">{t(`usageAnalysis.ranges.${range}`)}</span>
        </button>
      ))}
    </div>
  );
}

export function DashboardPage() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const exchangeRate = useStoreExchangeRate(true);
  const { currency } = useStoreCurrency();
  const [range, setRange] = useState<DashboardRange>("24h");
  const rangeConfig = DASHBOARD_RANGES[range];
  const summary = useDashboardAnalytics(8, 720, "self", {
    keepPreviousData: true,
    refreshInterval: 2000,
  });
  const usage = useDashboardAnalytics(rangeConfig.buckets, rangeConfig.hours, "self", {
    keepPreviousData: true,
    refreshInterval: 2000,
  });
  const usageSelectionLoading = usage.isLoading
    || usage.data?.buckets.length !== rangeConfig.buckets;
  const totals = useMemo(
    () => usage.data ? aggregateTokenTotals(usage.data.buckets) : undefined,
    [usage.data],
  );

  const plan = user?.billing_plan;
  const cnyPerUsd = exchangeRate.data?.cny_per_usd;
  const moneyLoading = !cnyPerUsd && exchangeRate.isLoading;
  const displayMoney = (nanoUsd: string | null | undefined) => {
    return cnyPerUsd ? formatCoinFromNanoUsdForCurrency(nanoUsd ?? "0", currency, cnyPerUsd) : "—";
  };
  const displayCoin = (nanoUsd: string | null | undefined) => {
    const value = displayMoney(nanoUsd);
    return value === "—" ? value : <CoinAmount value={value} />;
  };
  const summaryTotals = summary.data ? aggregateTokenTotals(summary.data.buckets) : undefined;
  const overview = [
    {
      title: t("dashboard.cards.accountData"),
      rows: [
        {
          label: t("dashboard.cards.currentBalance"),
          value: user?.balance_unlimited ? t("users.unlimited") : displayCoin(user?.balance_nano_usd),
        },
        {
          label: t("dashboard.cards.subscription"),
          value: plan ? <><span>{plan.name} · </span>{displayCoin(plan.grant_amount_nano_usd)}<span>/{plan.schedule}</span></> : t("dashboard.cards.noPlan"),
        },
      ],
    },
    {
      title: t("dashboard.cards.requestOverview"),
      rows: [
        { label: t("dashboard.cards.totalRequests"), value: (summary.data?.total_calls ?? 0).toLocaleString() },
        { label: t("dashboard.cards.todayRequests"), value: (summary.data?.today_calls ?? 0).toLocaleString() },
      ],
    },
    {
      title: t("dashboard.cards.costOverview"),
      rows: [
        { label: t("dashboard.cards.totalSpend"), value: displayCoin(summary.data?.total_cost_nano_usd) },
        { label: t("dashboard.cards.todaySpend"), value: displayCoin(summary.data?.today_cost_nano_usd) },
      ],
    },
    {
      title: t("dashboard.cards.tokenOverview"),
      rows: [
        { label: t("usageAnalysis.metrics.total"), value: (summaryTotals?.total ?? 0n).toLocaleString() },
        {
          label: t("usageAnalysis.cacheHitRate"),
          value: summaryTotals ? formatCacheHitRate(summaryTotals.input, summaryTotals.cacheRead) : "—",
        },
      ],
    },
  ];

  return (
    <PageWrapper className="flex min-w-0 flex-col gap-6 pb-6">
      <PageHeader
        title={t("dashboard.greeting", { username: user?.username ?? t("roles.user") })}
        description={t("dashboard.subtitle")}
      />

      {(summary.isLoading && !summary.data) || moneyLoading ? (
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          {Array.from({ length: 4 }, (_, index) => <Skeleton key={index} className="h-40 rounded-lg" />)}
        </div>
      ) : (
        <section className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          {overview.map((card, index) => <OverviewCard key={card.title} {...card} index={index} />)}
        </section>
      )}

      <section className="flex min-w-0 flex-col gap-4" aria-labelledby="dashboard-token-usage-title">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <h2 id="dashboard-token-usage-title" className="font-display text-lg font-semibold">{t("dashboard.tokenUsage")}</h2>
            <p className="mt-1 text-sm text-muted-foreground">{t("dashboard.tokenUsageDescription")}</p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <RangeControl value={range} onChange={setRange} />
            <Button asChild variant="outline" className="rounded-lg">
              <Link to="/dashboard/usage">{t("dashboard.openUsageAnalysis")}<ArrowRight className="size-4" /></Link>
            </Button>
          </div>
        </div>

        {usage.error && !usage.data ? (
          <EmptyState
            title={t("usageAnalysis.loadFailed")}
            description={t("usageAnalysis.loadFailedDescription")}
            action={<Button variant="outline" onClick={() => void usage.mutate()}>{t("common.retry")}</Button>}
          />
        ) : (
          <>
            <TokenSummary totals={totals} loading={usage.isLoading} />
            <UsageTrendChart
              buckets={usage.data?.buckets}
              metric="total"
              selectionKey={range}
              loading={usageSelectionLoading}
              compact
            />
          </>
        )}
      </section>
    </PageWrapper>
  );
}

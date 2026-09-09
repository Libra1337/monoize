import { useMemo, useState } from "react";
import { Activity, BarChart3, LoaderCircle, MousePointerClick } from "lucide-react";
import { useTranslation } from "react-i18next";

import { CoinAmount } from "@/components/coin-amount";
import { AnimatedTokenValue } from "@/components/usage/token-summary";
import { UsageTrendChart } from "@/components/usage/usage-trend-chart";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import type { ApiKey, ApiKeyAnalyticsRange } from "@/lib/api";
import { useApiKeyAnalytics } from "@/lib/swr";
import { formatCoinFromNanoUsdForCurrency } from "@/lib/store-money";
import { formatTokenCount, type TokenAnalyticsBucket } from "@/lib/usage-analytics";

function exact(value: string | null | undefined): bigint {
  return /^(?:0|[1-9]\d*)$/.test(value ?? "") ? BigInt(value!) : 0n;
}

function AnalyticsSkeleton() {
  return (
    <div className="grid min-h-[32rem] gap-4">
      <div className="grid gap-3 sm:grid-cols-3">
        {Array.from({ length: 3 }, (_, index) => (
          <Skeleton key={index} className="h-24 rounded-2xl" />
        ))}
      </div>
      <Skeleton className="h-56 rounded-2xl" />
      <Skeleton className="h-40 rounded-2xl" />
    </div>
  );
}

export function ApiKeyAnalyticsDialog({
  apiKey,
  onOpenChange,
}: {
  apiKey: ApiKey | null;
  onOpenChange: (open: boolean) => void;
}) {
  const { t, i18n } = useTranslation();
  const [range, setRange] = useState<ApiKeyAnalyticsRange>("24h");
  const { data, error, isLoading, isValidating } = useApiKeyAnalytics(apiKey?.id ?? null, range);
  const exchangeRate = useStoreExchangeRate(Boolean(apiKey));
  const { currency } = useStoreCurrency();
  const cnyPerUsd = exchangeRate.data?.cny_per_usd;

  const trendBuckets = useMemo<TokenAnalyticsBucket[]>(() => (data?.trend ?? []).map((point) => ({
    label: point.label,
    input_tokens_by_model: { usage: point.input_tokens },
    cache_read_tokens_by_model: { usage: point.cache_read_tokens },
    output_tokens_by_model: { usage: point.output_tokens },
  })), [data?.trend]);

  const formatCoin = (value: string | null | undefined) => (
    cnyPerUsd
      ? <CoinAmount value={formatCoinFromNanoUsdForCurrency(value ?? "0", currency, cnyPerUsd)} />
      : <span aria-label={t("common.loading")}>—</span>
  );

  return (
    <Dialog open={Boolean(apiKey)} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-hidden rounded-2xl p-0 sm:max-w-5xl">
        <div className="flex min-h-[38rem] flex-col p-5 sm:p-6">
          <DialogHeader className="shrink-0 pr-10">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0">
                <DialogTitle className="flex items-center gap-2 text-balance">
                  <BarChart3 className="size-5 text-primary" />
                  {t("apiKeys.analyticsTitle")} · {apiKey?.name}
                </DialogTitle>
                <DialogDescription className="mt-2 text-pretty">
                  {t("apiKeys.analyticsDescription")}
                </DialogDescription>
              </div>
              <div className="flex items-center gap-2">
                {isValidating && data ? (
                  <LoaderCircle className="size-4 animate-spin text-muted-foreground motion-reduce:animate-none" aria-label={t("common.loading")} />
                ) : null}
                <Tabs value={range} onValueChange={(value) => setRange(value as ApiKeyAnalyticsRange)}>
                  <TabsList className="rounded-xl" aria-label={t("apiKeys.analyticsRange")}>
                    <TabsTrigger value="24h">24h</TabsTrigger>
                    <TabsTrigger value="7d">7d</TabsTrigger>
                    <TabsTrigger value="30d">30d</TabsTrigger>
                    <TabsTrigger value="all">{t("apiKeys.analyticsAll")}</TabsTrigger>
                  </TabsList>
                </Tabs>
              </div>
            </div>
          </DialogHeader>

          <div className="mt-5 min-h-0 flex-1 overflow-y-auto pr-1">
            {isLoading && !data ? <AnalyticsSkeleton /> : error && !data ? (
              <div className="grid min-h-[32rem] place-items-center rounded-2xl border">
                <p role="alert" className="text-sm text-destructive">
                  {error instanceof Error ? error.message : t("apiKeys.analyticsLoadFailed")}
                </p>
              </div>
            ) : data ? (
              <div className="grid gap-4">
                <div className="grid overflow-hidden rounded-2xl border bg-card sm:grid-cols-3 sm:divide-x">
                  <div className="p-4">
                    <div className="flex items-center gap-2 text-sm text-muted-foreground"><Activity className="size-4" />{t("apiKeys.analyticsTokens")}</div>
                    <p className="mt-2 text-2xl font-semibold"><AnimatedTokenValue value={exact(data.total_tokens)} /></p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {t("apiKeys.analyticsInput")} {formatTokenCount(exact(data.total_input_tokens), i18n.language)} · {t("apiKeys.analyticsCache")} {formatTokenCount(exact(data.total_cache_read_tokens), i18n.language)} · {t("apiKeys.analyticsOutput")} {formatTokenCount(exact(data.total_output_tokens), i18n.language)}
                    </p>
                  </div>
                  <div className="border-t p-4 sm:border-t-0">
                    <div className="flex items-center gap-2 text-sm text-muted-foreground"><MousePointerClick className="size-4" />{t("apiKeys.analyticsRequests")}</div>
                    <p className="mt-2 text-2xl font-semibold tabular-nums">{data.request_count.toLocaleString(i18n.language)}</p>
                  </div>
                  <div className="border-t p-4 sm:border-t-0">
                    <p className="text-sm text-muted-foreground">{t("apiKeys.analyticsCost")}</p>
                    <p className="mt-2 text-2xl font-semibold">{formatCoin(data.consumed_coin_nano)}</p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {data.balance_mode === "independent"
                        ? <>{t("apiKeys.analyticsIndependentBalance")}: {formatCoin(data.independent_balance_nano)}</>
                        : t("apiKeys.analyticsWalletBalance")}
                    </p>
                  </div>
                </div>

                <UsageTrendChart
                  buckets={trendBuckets}
                  metric="total"
                  selectionKey={`api-key:${data.key_id}:${range}`}
                  loading={isValidating}
                  compact
                />

                <Card className="overflow-hidden rounded-2xl">
                  <CardContent className="p-0">
                    <div className="flex items-center justify-between gap-3 border-b px-4 py-3">
                      <h3 className="font-medium">{t("apiKeys.analyticsModels")}</h3>
                      <Badge variant="secondary">{data.models.length}</Badge>
                    </div>
                    {data.models.length === 0 ? (
                      <p className="p-6 text-center text-sm text-muted-foreground">{t("apiKeys.analyticsEmpty")}</p>
                    ) : (
                      <div className="divide-y">
                        {data.models.map((model) => (
                          <div key={model.model} className="grid gap-2 px-4 py-3 text-sm sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-center sm:gap-5">
                            <div className="min-w-0">
                              <p className="truncate font-mono font-medium" title={model.model}>{model.model}</p>
                              <p className="mt-1 text-xs text-muted-foreground">
                                {t("apiKeys.analyticsInput")} {formatTokenCount(exact(model.input_tokens), i18n.language)} · {t("apiKeys.analyticsCache")} {formatTokenCount(exact(model.cache_read_tokens), i18n.language)} · {t("apiKeys.analyticsOutput")} {formatTokenCount(exact(model.output_tokens), i18n.language)}
                              </p>
                            </div>
                            <div className="text-left sm:text-right">
                              <p className="font-mono font-medium tabular-nums"><AnimatedTokenValue value={exact(model.total_tokens)} /></p>
                              <p className="text-xs text-muted-foreground">{model.request_count.toLocaleString(i18n.language)} {t("apiKeys.analyticsRequestsUnit")}</p>
                            </div>
                            <div className="font-medium sm:text-right">{formatCoin(model.consumed_coin_nano)}</div>
                          </div>
                        ))}
                      </div>
                    )}
                  </CardContent>
                </Card>
              </div>
            ) : null}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

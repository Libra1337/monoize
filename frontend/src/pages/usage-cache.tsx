import { useDeferredValue, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { DataTableShell, TableToolbarSearch } from "@/components/ui/data-table-shell";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper, SharedTabIndicator } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useAuth } from "@/hooks/use-auth";
import { useProviders } from "@/lib/swr";
import { api } from "@/lib/api";
import { useUsageAnalytics, type WorkspaceAnalyticsScope } from "@/lib/org-analytics";
import {
  aggregateTokenTotals,
  cacheHitRateForTotals,
  cacheHitRateTable,
  formatCacheHitRate,
  formatTokenCount,
  type CacheHitGrade,
} from "@/lib/usage-analytics";
import { cn } from "@/lib/utils";

type CacheRange = "24h" | "7d" | "30d";

const CACHE_RANGES: Record<CacheRange, { hours: number; buckets: number }> = {
  "24h": { hours: 24, buckets: 24 },
  "7d": { hours: 168, buckets: 28 },
  "30d": { hours: 720, buckets: 30 },
};

const GRADE_TEXT: Record<CacheHitGrade, string> = {
  no_traffic: "text-muted-foreground",
  insufficient: "text-muted-foreground",
  low: "text-destructive",
  partial: "text-warning",
  high: "text-success",
};

const GRADE_BAR: Record<CacheHitGrade, string> = {
  no_traffic: "bg-transparent",
  insufficient: "bg-muted-foreground/40",
  low: "bg-destructive",
  partial: "bg-warning",
  high: "bg-success",
};

export function UsageCachePage({ orgId }: { orgId?: string } = {}) {
  const { t, i18n } = useTranslation();
  const { user } = useAuth();
  const isAdmin = user?.role === "super_admin" || user?.role === "admin";
  const [range, setRange] = useState<CacheRange>("7d");
  const [search, setSearch] = useState("");
  const [trafficOnly, setTrafficOnly] = useState(false);
  const deferredSearch = useDeferredValue(search.trim().toLowerCase());
  const config = CACHE_RANGES[range];

  const isSuperAdmin = !orgId && user?.role === "super_admin";
  // UA-25: a super admin aggregates every user, an admin its group, a member itself.
  // In org mode the source is the org's shared keys instead and the scope is ignored.
  const scope: WorkspaceAnalyticsScope = isSuperAdmin
    ? "all"
    : !orgId && user?.role === "admin"
      ? "group"
      : "self";
  const analytics = useUsageAnalytics(orgId, config.buckets, config.hours, undefined, scope);
  // UA-41: the per-user table is super_admin-only.
  const cacheUsers = useSWR(
    isSuperAdmin ? `/api/dashboard/usage/cache/users?range_hours=${config.hours}` : null,
    () => api.getCacheHitRateUsers(config.hours),
    { keepPreviousData: true, refreshInterval: 2000 },
  );
  // UA-36: the routable catalog is admin-only, so a member sees the models it used.
  // Org space is always member-level: the table lists the models the org actually used.
  const providers = useProviders({ keepPreviousData: true }, !orgId && isAdmin);

  const catalog = useMemo(
    () => (providers.data ?? []).flatMap((provider) => Object.keys(provider.channel.models)),
    [providers.data],
  );
  const rows = useMemo(
    () => analytics.data ? cacheHitRateTable(analytics.data.buckets, catalog) : [],
    [analytics.data, catalog],
  );
  const totals = useMemo(
    () => analytics.data ? aggregateTokenTotals(analytics.data.buckets) : undefined,
    [analytics.data],
  );
  const visible = useMemo(
    () => rows.filter((row) => (
      (!trafficOnly || row.input > 0n)
      && (!deferredSearch || row.model.toLowerCase().includes(deferredSearch))
    )),
    [rows, trafficOnly, deferredSearch],
  );
  const trackedCount = rows.filter((row) => row.input > 0n).length;
  const userRows = useMemo(
    () => (cacheUsers.data?.users ?? []).map((row) => {
      const input = BigInt(row.input_tokens);
      const cacheRead = BigInt(row.cache_read_tokens);
      return {
        user_id: row.user_id,
        username: row.username,
        input,
        cacheRead,
        ...cacheHitRateForTotals(input, cacheRead),
      };
    }),
    [cacheUsers.data],
  );
  const visibleUsers = useMemo(
    () => userRows.filter((row) => (
      (!trafficOnly || row.input > 0n)
      && (!deferredSearch || row.username.toLowerCase().includes(deferredSearch))
    )),
    [userRows, trafficOnly, deferredSearch],
  );

  if (analytics.error && !analytics.data) {
    return (
      <PageWrapper className="flex min-w-0 flex-col gap-6 pb-6">
        <PageHeader title={t("usageCache.title")} description={t("usageCache.description")} />
        <EmptyState
          title={t("usageAnalysis.loadFailed")}
          description={t("usageAnalysis.loadFailedDescription")}
          action={<Button variant="outline" onClick={() => void analytics.mutate()}>{t("common.retry")}</Button>}
        />
      </PageWrapper>
    );
  }

  return (
    <PageWrapper className="flex min-w-0 flex-col gap-6 pb-6">
      <PageHeader
        title={t("usageCache.title")}
        description={t("usageCache.description")}
        actions={(
          <div className="flex max-w-full overflow-x-auto rounded-lg bg-muted p-1" role="group" aria-label={t("usageAnalysis.rangeLabel")}>
            {(Object.keys(CACHE_RANGES) as CacheRange[]).map((option) => (
              <button
                key={option}
                type="button"
                aria-pressed={range === option}
                className={cn(
                  "relative min-h-9 shrink-0 rounded-md px-3 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  range === option ? "text-foreground" : "text-muted-foreground hover:text-foreground",
                )}
                onClick={() => setRange(option)}
              >
                {range === option ? <SharedTabIndicator layoutId="usage-cache-range" className="absolute inset-0 rounded-md bg-background shadow-sm" /> : null}
                <span className="relative z-10">{t(`usageAnalysis.ranges.${option}`)}</span>
              </button>
            ))}
          </div>
        )}
      />

      <Card className="overflow-hidden rounded-lg">
        <CardContent className="grid divide-y p-0 sm:grid-cols-4 sm:divide-x sm:divide-y-0">
          {totals ? [
            { key: "input", label: t("usageAnalysis.metrics.input"), value: formatTokenCount(totals.input, i18n.language) },
            { key: "cache", label: t("usageAnalysis.metrics.cacheRead"), value: formatTokenCount(totals.cacheRead, i18n.language) },
            { key: "rate", label: t("usageAnalysis.cacheHitRate"), value: formatCacheHitRate(totals.input, totals.cacheRead) },
            { key: "models", label: t("usageCache.modelsTracked"), value: `${trackedCount} / ${rows.length}` },
          ].map((card) => (
            <div key={card.key} className="min-w-0 p-5">
              <p className="text-sm text-muted-foreground">{card.label}</p>
              <p className="mt-2 min-w-0 break-words font-display text-2xl font-semibold tabular-nums">{card.value}</p>
            </div>
          )) : Array.from({ length: 4 }, (_, index) => (
            <div key={index} className="min-w-0 p-5">
              <Skeleton className="h-4 w-24" />
              <Skeleton className="mt-4 h-8 w-32" />
            </div>
          ))}
        </CardContent>
      </Card>

      <p className="max-w-3xl text-sm text-muted-foreground">
        {t("usageAnalysis.cacheByModel.description")}
      </p>

      <DataTableShell
        toolbar={(
          <>
            <TableToolbarSearch
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder={t("usageCache.searchPlaceholder")}
              aria-label={t("usageCache.searchPlaceholder")}
            />
            <label className="flex items-center gap-2 text-sm text-muted-foreground">
              <Switch checked={trafficOnly} onCheckedChange={setTrafficOnly} aria-label={t("usageCache.trafficOnly")} />
              {t("usageCache.trafficOnly")}
            </label>
          </>
        )}
        isEmpty={!analytics.isLoading && visible.length === 0}
        emptyState={<EmptyState title={t("usageAnalysis.empty")} description={t("usageCache.emptyDescription")} />}
      >
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("usageCache.columns.model")}</TableHead>
              <TableHead className="text-right">{t("usageAnalysis.metrics.input")}</TableHead>
              <TableHead className="text-right">{t("usageAnalysis.metrics.cacheRead")}</TableHead>
              <TableHead className="text-right">{t("usageAnalysis.cacheHitRate")}</TableHead>
              <TableHead>{t("usageCache.columns.assessment")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {analytics.isLoading && !analytics.data ? Array.from({ length: 8 }, (_, index) => (
              <TableRow key={index}>
                {Array.from({ length: 5 }, (_, cell) => (
                  <TableCell key={cell}><Skeleton className="h-4 w-full" /></TableCell>
                ))}
              </TableRow>
            )) : visible.map((row) => (
              <TableRow key={row.model}>
                <TableCell className="max-w-[22rem] font-medium [overflow-wrap:anywhere]">{row.model}</TableCell>
                <TableCell className="text-right font-mono text-xs tabular-nums">
                  {row.input === 0n ? "—" : formatTokenCount(row.input, i18n.language)}
                </TableCell>
                <TableCell className="text-right font-mono text-xs tabular-nums">
                  {row.input === 0n ? "—" : formatTokenCount(row.cacheRead, i18n.language)}
                </TableCell>
                <TableCell className={cn("text-right font-mono text-sm tabular-nums", GRADE_TEXT[row.grade])}>
                  {formatCacheHitRate(row.input, row.cacheRead)}
                </TableCell>
                <TableCell className="min-w-[9rem]">
                  <div className="flex items-center gap-2">
                    <span className={cn("text-xs", GRADE_TEXT[row.grade])}>
                      {t(`usageAnalysis.cacheByModel.grades.${row.grade}`)}
                    </span>
                    {row.input > 0n ? (
                      <span className="h-1.5 w-16 overflow-hidden rounded-full bg-muted">
                        <span
                          className={cn("block h-full rounded-full", GRADE_BAR[row.grade])}
                          style={{ width: `${Number(row.basisPoints) / 100}%` }}
                        />
                      </span>
                    ) : null}
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </DataTableShell>

      {isSuperAdmin ? (
        <section className="flex min-w-0 flex-col gap-3" aria-labelledby="usage-cache-users-title">
          <h2 id="usage-cache-users-title" className="text-base font-semibold">
            {t("usageCache.usersTitle")}
          </h2>
          <p className="max-w-3xl text-sm text-muted-foreground">
            {t("usageCache.usersDescription")}
          </p>
          <DataTableShell
            isEmpty={!cacheUsers.isLoading && visibleUsers.length === 0}
            emptyState={<EmptyState title={t("usageAnalysis.empty")} description={t("usageCache.usersEmptyDescription")} />}
          >
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("usageCache.columns.user")}</TableHead>
                  <TableHead className="text-right">{t("usageAnalysis.metrics.input")}</TableHead>
                  <TableHead className="text-right">{t("usageAnalysis.metrics.cacheRead")}</TableHead>
                  <TableHead className="text-right">{t("usageAnalysis.cacheHitRate")}</TableHead>
                  <TableHead>{t("usageCache.columns.assessment")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {cacheUsers.isLoading && !cacheUsers.data ? Array.from({ length: 5 }, (_, index) => (
                  <TableRow key={index}>
                    {Array.from({ length: 5 }, (_, cell) => (
                      <TableCell key={cell}><Skeleton className="h-4 w-full" /></TableCell>
                    ))}
                  </TableRow>
                )) : visibleUsers.map((row) => (
                  <TableRow key={row.user_id}>
                    <TableCell className="max-w-[22rem] font-medium [overflow-wrap:anywhere]">{row.username}</TableCell>
                    <TableCell className="text-right font-mono text-xs tabular-nums">
                      {formatTokenCount(row.input, i18n.language)}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs tabular-nums">
                      {formatTokenCount(row.cacheRead, i18n.language)}
                    </TableCell>
                    <TableCell className={cn("text-right font-mono text-sm tabular-nums", GRADE_TEXT[row.grade])}>
                      {formatCacheHitRate(row.input, row.cacheRead)}
                    </TableCell>
                    <TableCell className="min-w-[9rem]">
                      <div className="flex items-center gap-2">
                        <span className={cn("text-xs", GRADE_TEXT[row.grade])}>
                          {t(`usageAnalysis.cacheByModel.grades.${row.grade}`)}
                        </span>
                        {row.input > 0n ? (
                          <span className="h-1.5 w-16 overflow-hidden rounded-full bg-muted">
                            <span
                              className={cn("block h-full rounded-full", GRADE_BAR[row.grade])}
                              style={{ width: `${Number(row.basisPoints) / 100}%` }}
                            />
                          </span>
                        ) : null}
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </DataTableShell>
        </section>
      ) : null}
    </PageWrapper>
  );
}

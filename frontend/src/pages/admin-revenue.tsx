import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { mutate } from "swr";
import {
  AlertTriangle,
  ChevronDown,
  Coins,
  Download,
  FileSpreadsheet,
  MousePointerClick,
  RefreshCw,
  UserMinus,
  X,
} from "lucide-react";

import { CoinAmount } from "@/components/coin-amount";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/skeleton";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import type { RevenueDayRow, User } from "@/lib/api";
import { api } from "@/lib/api";
import {
  SWR_KEYS,
  useAdminRevenueDaily,
  useAdminRevenueExclusions,
  useUsers,
} from "@/lib/swr";
import { formatCoinFromNanoUsdForCurrency } from "@/lib/store-money";
import { cn } from "@/lib/utils";

/** AR-10/AR-19: day ids are `YYYY-MM-DD` in Asia/Shanghai local time. */
function beijingTodayId(): string {
  const beijingNow = new Date(Date.now() + 8 * 3600 * 1000);
  return beijingNow.toISOString().slice(0, 10);
}

function shiftDay(day: string, offsetDays: number): string {
  const date = new Date(`${day}T00:00:00Z`);
  date.setUTCDate(date.getUTCDate() + offsetDays);
  return date.toISOString().slice(0, 10);
}

function formatInteger(value: number): string {
  return value.toLocaleString("en-US");
}

function RevenueSkeleton() {
  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-2">
          <Skeleton className="h-8 w-44" />
          <Skeleton className="h-4 w-72" />
        </div>
        <div className="flex gap-2">
          <Skeleton className="h-9 w-32" />
          <Skeleton className="h-9 w-32" />
          <Skeleton className="size-9 rounded-lg" />
        </div>
      </div>
      <Skeleton className="h-96 w-full rounded-xl" />
    </div>
  );
}

export function AdminRevenuePage() {
  const { t } = useTranslation();
  const { currency } = useStoreCurrency();
  const exchangeRate = useStoreExchangeRate(currency === "CNY");
  const cnyPerUsd = exchangeRate.data?.cny_per_usd;

  const today = useMemo(beijingTodayId, []);
  const [from, setFrom] = useState(() => shiftDay(today, -29));
  const [to, setTo] = useState(today);
  const [expandedDay, setExpandedDay] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [userQuery, setUserQuery] = useState("");
  const [exclusionError, setExclusionError] = useState<string | null>(null);

  const daily = useAdminRevenueDaily(from, to);
  const exclusions = useAdminRevenueExclusions();
  const { data: users = [] } = useUsers();

  const formatCost = (nanoUsd: string) => {
    if (currency === "CNY" && !cnyPerUsd) return "—";
    return formatCoinFromNanoUsdForCurrency(nanoUsd, currency, cnyPerUsd ?? "0");
  };

  const excludedIds = new Set(
    (exclusions.data?.exclusions ?? []).map((row) => row.user_id)
  );
  const userMatches = useMemo(() => {
    const query = userQuery.trim().toLowerCase();
    if (!query) return [];
    return users
      .filter(
        (user: User) =>
          !excludedIds.has(user.id) &&
          (user.username.toLowerCase().includes(query) ||
            user.id.toLowerCase().includes(query))
      )
      .slice(0, 8);
  }, [userQuery, users, excludedIds]);

  const addExclusion = async (user: User) => {
    setExclusionError(null);
    setUserQuery("");
    // AR-17: optimistic list update; the server recomputes history before
    // returning, so the revalidation that follows carries settled numbers.
    const current = exclusions.data?.exclusions ?? [];
    mutate(
      SWR_KEYS.ADMIN_REVENUE_EXCLUSIONS,
      {
        exclusions: [
          ...current,
          { user_id: user.id, username: user.username, created_at: "" },
        ],
      },
      false
    );
    try {
      await api.addAdminRevenueExclusion(user.id);
      await Promise.all([
        exclusions.mutate(),
        daily.mutate(),
      ]);
    } catch (error) {
      await exclusions.mutate();
      setExclusionError(
        error instanceof Error ? error.message : t("common.error")
      );
    }
  };

  const removeExclusion = async (userId: string) => {
    setExclusionError(null);
    const current = exclusions.data?.exclusions ?? [];
    mutate(
      SWR_KEYS.ADMIN_REVENUE_EXCLUSIONS,
      { exclusions: current.filter((row) => row.user_id !== userId) },
      false
    );
    try {
      await api.removeAdminRevenueExclusion(userId);
      await Promise.all([exclusions.mutate(), daily.mutate()]);
    } catch (error) {
      await exclusions.mutate();
      setExclusionError(
        error instanceof Error ? error.message : t("common.error")
      );
    }
  };

  const exportExcel = async () => {
    setExporting(true);
    try {
      const blob = await api.exportAdminRevenueDaily(from, to);
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `monoize-revenue-${from}-${to}.xlsx`;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      URL.revokeObjectURL(url);
    } catch (error) {
      setExclusionError(
        error instanceof Error ? error.message : t("common.error")
      );
    } finally {
      setExporting(false);
    }
  };

  if (daily.isLoading && !daily.data) {
    return (
      <PageWrapper>
        <RevenueSkeleton />
      </PageWrapper>
    );
  }
  if (daily.error && !daily.data) {
    return (
      <PageWrapper className="space-y-4">
        <EmptyState
          variant="card"
          icon={<AlertTriangle className="size-8 text-destructive" />}
          title={t("adminRevenue.loadFailed")}
          description={
            daily.error instanceof Error
              ? daily.error.message
              : t("common.error")
          }
        />
        <div className="flex justify-center">
          <Button variant="outline" onClick={() => void daily.mutate()}>
            <RefreshCw data-icon />
            {t("common.retry")}
          </Button>
        </div>
      </PageWrapper>
    );
  }

  const days = daily.data?.days ?? [];
  const totalRevenue = days.reduce(
    (sum, day) => sum + BigInt(day.total_charge_nano_usd),
    0n
  );

  return (
    <PageWrapper className="space-y-5 pb-6">
      <PageHeader
        title={t("adminRevenue.title")}
        description={t("adminRevenue.description")}
        actions={
          <div className="flex items-center gap-2">
            <Input
              type="date"
              aria-label={t("adminRevenue.from")}
              value={from}
              max={to}
              onChange={(event) => setFrom(event.target.value)}
              className="h-9 w-36 font-mono text-xs"
            />
            <span className="text-xs text-muted-foreground">–</span>
            <Input
              type="date"
              aria-label={t("adminRevenue.to")}
              value={to}
              min={from}
              max={today}
              onChange={(event) => setTo(event.target.value)}
              className="h-9 w-36 font-mono text-xs"
            />
            <Button
              size="sm"
              variant="outline"
              disabled={exporting}
              onClick={() => void exportExcel()}
            >
              {exporting ? (
                <RefreshCw data-icon className="animate-spin" />
              ) : (
                <FileSpreadsheet data-icon />
              )}
              {exporting ? t("adminRevenue.exporting") : t("adminRevenue.export")}
            </Button>
          </div>
        }
      />

      <div className="grid gap-3 sm:grid-cols-3">
        <div className="flex items-center gap-3 rounded-xl border bg-card p-4">
          <Coins className="size-5 text-warning" />
          <div>
            <p className="text-xs text-muted-foreground">
              {t("adminRevenue.totalRevenue")}
            </p>
            <p className="font-mono text-lg font-semibold">
              <CoinAmount value={formatCost(totalRevenue.toString())} />
            </p>
          </div>
        </div>
        <div className="flex items-center gap-3 rounded-xl border bg-card p-4">
          <MousePointerClick className="size-5 text-primary" />
          <div>
            <p className="text-xs text-muted-foreground">
              {t("adminRevenue.totalCalls")}
            </p>
            <p className="font-mono text-lg font-semibold">
              {formatInteger(
                days.reduce((sum, day) => sum + day.total_calls, 0)
              )}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-3 rounded-xl border bg-card p-4">
          <Download className="size-5 text-success" />
          <div>
            <p className="text-xs text-muted-foreground">
              {t("adminRevenue.daysShown")}
            </p>
            <p className="font-mono text-lg font-semibold">
              {formatInteger(days.length)}
            </p>
          </div>
        </div>
      </div>

      <Card className="overflow-hidden rounded-xl">
        <CardContent className="p-0">
          <div className="border-b px-5 py-4">
            <h2 className="font-display text-base font-semibold">
              {t("adminRevenue.tableTitle")}
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">
              {t("adminRevenue.tableHint")}
            </p>
          </div>
          {days.length === 0 ? (
            <EmptyState
              title={t("adminRevenue.empty")}
              description={t("adminRevenue.emptyHint")}
              className="py-14"
            />
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full min-w-[760px] text-sm">
                <thead>
                  <tr className="border-b bg-muted/35 text-left text-xs text-muted-foreground">
                    <th className="px-5 py-3 font-medium">
                      {t("adminRevenue.day")}
                    </th>
                    <th className="px-3 py-3 text-right font-medium">
                      {t("adminRevenue.revenue")}
                    </th>
                    <th className="px-3 py-3 text-right font-medium">
                      {t("adminRevenue.calls")}
                    </th>
                    <th className="px-3 py-3 text-right font-medium">
                      {t("adminRevenue.tokens")}
                    </th>
                    <th className="px-5 py-3 font-medium">
                      {t("adminRevenue.topModel")}
                    </th>
                    <th className="w-10 px-2 py-3" />
                  </tr>
                </thead>
                <tbody>
                  {days.map((day) => (
                    <DayRow
                      key={day.day}
                      day={day}
                      expanded={expandedDay === day.day}
                      formatCost={formatCost}
                      onToggle={() =>
                        setExpandedDay(expandedDay === day.day ? null : day.day)
                      }
                    />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      <Card className="overflow-hidden rounded-xl">
        <CardContent className="p-0">
          <div className="border-b px-5 py-4">
            <h2 className="font-display text-base font-semibold">
              {t("adminRevenue.exclusionsTitle")}
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">
              {t("adminRevenue.exclusionsHint")}
            </p>
          </div>
          <div className="relative border-b px-5 py-4">
            <Input
              value={userQuery}
              onChange={(event) => setUserQuery(event.target.value)}
              placeholder={t("adminRevenue.searchUser")}
              className="w-full max-w-sm"
              aria-label={t("adminRevenue.searchUser")}
            />
            {userQuery.trim() && userMatches.length > 0 && (
              <div className="absolute inset-x-5 top-full z-10 mt-1 max-w-sm rounded-lg border bg-popover p-1 shadow-md">
                {userMatches.map((user) => (
                  <button
                    key={user.id}
                    type="button"
                    onClick={() => void addExclusion(user)}
                    className="flex w-full items-center justify-between gap-2 rounded-md px-3 py-2 text-left text-sm hover:bg-accent"
                  >
                    <span className="truncate font-medium">{user.username}</span>
                    <span className="truncate font-mono text-xs text-muted-foreground">
                      {user.id}
                    </span>
                  </button>
                ))}
              </div>
            )}
            {exclusionError && (
              <p className="mt-2 text-sm text-destructive">{exclusionError}</p>
            )}
          </div>
          {(exclusions.data?.exclusions ?? []).length === 0 ? (
            <EmptyState
              icon={<UserMinus className="size-8 text-muted-foreground" />}
              title={t("adminRevenue.noExclusions")}
              className="py-10"
            />
          ) : (
            <ul className="divide-y">
              {exclusions.data?.exclusions.map((row) => (
                <li
                  key={row.user_id}
                  className="flex items-center justify-between gap-3 px-5 py-3"
                >
                  <div className="min-w-0">
                    <p className="truncate font-medium">{row.username}</p>
                    <p className="truncate font-mono text-xs text-muted-foreground">
                      {row.user_id}
                    </p>
                  </div>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => void removeExclusion(row.user_id)}
                    aria-label={t("adminRevenue.removeExclusion")}
                  >
                    <X data-icon />
                    {t("common.remove")}
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </PageWrapper>
  );
}

function DayRow({
  day,
  expanded,
  formatCost,
  onToggle,
}: {
  day: RevenueDayRow;
  expanded: boolean;
  formatCost: (nanoUsd: string) => string;
  onToggle: () => void;
}) {
  const { t } = useTranslation();
  const topModel = day.models[0];
  const tokens = day.total_input_tokens + day.total_output_tokens;

  return (
    <>
      <tr
        className={cn(
          "border-b transition-colors duration-200 last:border-b-0 hover:bg-accent/45",
          expanded && "bg-accent/30"
        )}
      >
        <td className="px-5 py-3 font-mono">{day.day}</td>
        <td className="px-3 py-3 text-right font-mono tabular-nums">
          <CoinAmount value={formatCost(day.total_charge_nano_usd)} />
        </td>
        <td className="px-3 py-3 text-right font-mono tabular-nums">
          {formatInteger(day.total_calls)}
        </td>
        <td className="px-3 py-3 text-right font-mono tabular-nums">
          {formatInteger(tokens)}
        </td>
        <td className="px-5 py-3">
          {topModel ? (
            <div className="flex min-w-0 items-center gap-2">
              <span className="truncate font-medium">{topModel.model}</span>
              <span className="shrink-0 font-mono text-xs text-muted-foreground">
                (<CoinAmount value={formatCost(topModel.charge_nano_usd)} />)
              </span>
            </div>
          ) : (
            <span className="text-muted-foreground">—</span>
          )}
        </td>
        <td className="px-2 py-3">
          <button
            type="button"
            disabled={day.models.length === 0}
            onClick={onToggle}
            aria-expanded={expanded}
            aria-label={t("adminRevenue.modelDetails")}
            className="flex size-7 items-center justify-center rounded-md hover:bg-accent disabled:opacity-30"
          >
            <ChevronDown
              className={cn(
                "size-4 transition-transform duration-200",
                expanded && "rotate-180"
              )}
            />
          </button>
        </td>
      </tr>
      {expanded && (
        <tr className="bg-muted/25">
          <td colSpan={6} className="px-5 py-3">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs text-muted-foreground">
                  <th className="py-1.5 pr-3 font-medium">
                    {t("adminRevenue.model")}
                  </th>
                  <th className="py-1.5 px-3 text-right font-medium">
                    {t("adminRevenue.revenue")}
                  </th>
                  <th className="py-1.5 px-3 text-right font-medium">
                    {t("adminRevenue.calls")}
                  </th>
                  <th className="py-1.5 px-3 text-right font-medium">
                    {t("adminRevenue.inputTokens")}
                  </th>
                  <th className="py-1.5 pl-3 text-right font-medium">
                    {t("adminRevenue.outputTokens")}
                  </th>
                </tr>
              </thead>
              <tbody>
                {day.models.map((model) => (
                  <tr key={model.model} className="border-t border-border/60">
                    <td className="py-2 pr-3 font-medium">{model.model}</td>
                    <td className="py-2 px-3 text-right font-mono tabular-nums">
                      <CoinAmount value={formatCost(model.charge_nano_usd)} />
                    </td>
                    <td className="py-2 px-3 text-right font-mono tabular-nums">
                      {formatInteger(model.calls)}
                    </td>
                    <td className="py-2 px-3 text-right font-mono tabular-nums">
                      {formatInteger(model.input_tokens)}
                    </td>
                    <td className="py-2 pl-3 text-right font-mono tabular-nums">
                      {formatInteger(model.output_tokens)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </td>
        </tr>
      )}
    </>
  );
}

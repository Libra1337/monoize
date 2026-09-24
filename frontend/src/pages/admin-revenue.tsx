import type { ReactNode } from "react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { mutate } from "swr";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  ChevronDown,
  Coins,
  FileSpreadsheet,
  MousePointerClick,
  RefreshCw,
  UserMinus,
  Users,
  X,
} from "lucide-react";

import { CoinAmount } from "@/components/coin-amount";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/skeleton";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import type { RevenueModelRow, User } from "@/lib/api";
import { api } from "@/lib/api";
import { SWR_KEYS, useAdminRevenueDaily, useAdminRevenueExclusions, useUsers } from "@/lib/swr";
import { formatCoinFromNanoUsdForCurrency } from "@/lib/store-money";
import { cn } from "@/lib/utils";

/** AR-10/AR-19: day ids are `YYYY-MM-DD` in Asia/Shanghai local time. */
function beijingTodayId(): string {
  const beijingNow = new Date(Date.now() + 8 * 3600 * 1000);
  return beijingNow.toISOString().slice(0, 10);
}

function formatInteger(value: number): string {
  return value.toLocaleString("en-US");
}

/** Current-year day ids render as `MM-DD`; other years keep the full form. */
function formatDayShort(day: string): string {
  const currentYear = new Date().getFullYear().toString();
  return day.startsWith(`${currentYear}-`) ? day.slice(5) : day;
}

type SortKey = "day" | "revenue" | "calls" | "tokens";
type SortDirection = "asc" | "desc";

/** One flattened (day, user) row of the spreadsheet-style table. */
interface FlatRow {
  day: string;
  userId: string;
  username: string | null;
  chargeNanoUsd: bigint;
  calls: number;
  inputTokens: number;
  outputTokens: number;
  models: RevenueModelRow[];
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
      <div className="grid gap-3 sm:grid-cols-3">
        <Skeleton className="h-20 rounded-xl" />
        <Skeleton className="h-20 rounded-xl" />
        <Skeleton className="h-20 rounded-xl" />
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

  // AR-16: both bounds default to the current Beijing day.
  const today = useMemo(beijingTodayId, []);
  const [from, setFrom] = useState(today);
  const [to, setTo] = useState(today);
  const [sortKey, setSortKey] = useState<SortKey>("day");
  const [sortDirection, setSortDirection] = useState<SortDirection>("desc");
  const [expandedRow, setExpandedRow] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [exclusionsOpen, setExclusionsOpen] = useState(false);
  const [userQuery, setUserQuery] = useState("");
  const [exclusionError, setExclusionError] = useState<string | null>(null);

  const daily = useAdminRevenueDaily(from, to);
  const exclusions = useAdminRevenueExclusions();
  const { data: users = [] } = useUsers();

  const formatCost = (nanoUsd: string) => {
    if (currency === "CNY" && !cnyPerUsd) return "—";
    return formatCoinFromNanoUsdForCurrency(nanoUsd, currency, cnyPerUsd ?? "0");
  };

  // AR-16: flatten every day's per-user detail into one spreadsheet-style list.
  const flatRows = useMemo<FlatRow[]>(() => {
    const rows: FlatRow[] = [];
    for (const day of daily.data?.days ?? []) {
      for (const user of day.users) {
        rows.push({
          day: day.day,
          userId: user.user_id,
          username: user.username,
          chargeNanoUsd: BigInt(user.charge_nano_usd),
          calls: user.calls,
          inputTokens: user.input_tokens,
          outputTokens: user.output_tokens,
          models: user.models ?? [],
        });
      }
    }
    return rows;
  }, [daily.data]);

  const sortedRows = useMemo(() => {
    return [...flatRows].sort((left, right) => {
      let comparison: number;
      switch (sortKey) {
        case "revenue":
          comparison =
            sortDirection === "asc"
              ? Number(left.chargeNanoUsd - right.chargeNanoUsd)
              : Number(right.chargeNanoUsd - left.chargeNanoUsd);
          break;
        case "calls":
          comparison = sortDirection === "asc" ? left.calls - right.calls : right.calls - left.calls;
          break;
        case "tokens":
          comparison =
            sortDirection === "asc"
              ? left.inputTokens + left.outputTokens - (right.inputTokens + right.outputTokens)
              : right.inputTokens + right.outputTokens - (left.inputTokens + left.outputTokens);
          break;
        case "day":
        default:
          comparison =
            sortDirection === "asc"
              ? left.day.localeCompare(right.day)
              : right.day.localeCompare(left.day);
          break;
      }
      // Stable tie-breaks: day desc, then revenue desc, then user id.
      if (comparison === 0) {
        if (left.day !== right.day) return right.day.localeCompare(left.day);
        if (left.chargeNanoUsd !== right.chargeNanoUsd) {
          return right.chargeNanoUsd > left.chargeNanoUsd ? 1 : -1;
        }
        return left.userId.localeCompare(right.userId);
      }
      return comparison;
    });
  }, [flatRows, sortKey, sortDirection]);

  const summary = useMemo(() => {
    let revenue = 0n;
    let calls = 0;
    const userIds = new Set<string>();
    for (const row of flatRows) {
      revenue += row.chargeNanoUsd;
      calls += row.calls;
      userIds.add(row.userId);
    }
    return { revenue, calls, users: userIds.size };
  }, [flatRows]);

  // Group the sorted rows per calendar day: one header row per day (with the
  // day's totals), user rows below without repeating the date. Groups keep the
  // day sort direction; inner rows follow the active sort key.
  const dayGroups = useMemo(() => {
    const groups = new Map<string, FlatRow[]>();
    for (const row of sortedRows) {
      const bucket = groups.get(row.day);
      if (bucket) bucket.push(row);
      else groups.set(row.day, [row]);
    }
    let days = [...groups.entries()].map(([day, rows]) => {
      let revenue = 0n;
      const userIds = new Set<string>();
      for (const row of rows) {
        revenue += row.chargeNanoUsd;
        userIds.add(row.userId);
      }
      return { day, rows, revenue, users: userIds.size };
    });
    // sortKey "day" controls group order; other keys keep days newest-first.
    const dayAsc = sortKey === "day" && sortDirection === "asc";
    days = days.sort((left, right) =>
      dayAsc ? left.day.localeCompare(right.day) : right.day.localeCompare(left.day),
    );
    return days;
  }, [sortedRows, sortKey, sortDirection]);

  const toggleSort = (key: SortKey) => {
    if (sortKey === key) {
      setSortDirection((direction) => (direction === "asc" ? "desc" : "asc"));
    } else {
      setSortKey(key);
      setSortDirection("desc");
    }
  };

  const userMatches = useMemo(() => {
    const excludedIds = new Set((exclusions.data?.exclusions ?? []).map((row) => row.user_id));
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
  }, [userQuery, users, exclusions.data]);

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
      await Promise.all([exclusions.mutate(), daily.mutate()]);
    } catch (error) {
      await exclusions.mutate();
      setExclusionError(error instanceof Error ? error.message : t("common.error"));
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
      setExclusionError(error instanceof Error ? error.message : t("common.error"));
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
      setExclusionError(error instanceof Error ? error.message : t("common.error"));
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
          description={daily.error instanceof Error ? daily.error.message : t("common.error")}
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

  const sortIcon = (key: SortKey) => {
    if (sortKey !== key) return <ArrowUpDown className="size-3 opacity-40" />;
    return sortDirection === "asc" ? (
      <ArrowUp className="size-3" />
    ) : (
      <ArrowDown className="size-3" />
    );
  };
  const sortButton = (key: SortKey, label: string, align: "left" | "right" = "right") => (
    <button
      type="button"
      onClick={() => toggleSort(key)}
      className={cn(
        "flex items-center gap-1 hover:text-foreground",
        align === "right" && "ml-auto flex-row-reverse"
      )}
      aria-label={label}
    >
      {label}
      {sortIcon(key)}
    </button>
  );

  const exclusionCount = (exclusions.data?.exclusions ?? []).length;

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
            <Button size="sm" variant="outline" onClick={() => setExclusionsOpen(true)}>
              <UserMinus data-icon />
              {t("adminRevenue.exclusionsButton", { count: exclusionCount })}
            </Button>
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
            <p className="text-xs text-muted-foreground">{t("adminRevenue.totalRevenue")}</p>
            <p className="font-mono text-lg font-semibold">
              <CoinAmount value={formatCost(summary.revenue.toString())} />
            </p>
          </div>
        </div>
        <div className="flex items-center gap-3 rounded-xl border bg-card p-4">
          <MousePointerClick className="size-5 text-primary" />
          <div>
            <p className="text-xs text-muted-foreground">{t("adminRevenue.totalCalls")}</p>
            <p className="font-mono text-lg font-semibold">{formatInteger(summary.calls)}</p>
          </div>
        </div>
        <div className="flex items-center gap-3 rounded-xl border bg-card p-4">
          <Users className="size-5 text-success" />
          <div>
            <p className="text-xs text-muted-foreground">{t("adminRevenue.activeUsers")}</p>
            <p className="font-mono text-lg font-semibold">{formatInteger(summary.users)}</p>
          </div>
        </div>
      </div>

      <Card className="overflow-hidden rounded-xl">
        <CardContent className="p-0">
          <div className="border-b px-5 py-4">
            <h2 className="font-display text-base font-semibold">
              {t("adminRevenue.userTableTitle")}
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">
              {t("adminRevenue.userTableHint")}
            </p>
          </div>
          {sortedRows.length === 0 ? (
            <EmptyState
              title={t("adminRevenue.empty")}
              description={t("adminRevenue.emptyHint")}
              className="py-14"
            />
          ) : (
            <div className="max-h-[70vh] overflow-auto">
              <table className="w-full min-w-[840px] text-sm">
                <thead className="sticky top-0 z-10">
                  <tr className="border-b bg-muted/60 text-xs text-muted-foreground backdrop-blur">
                    <th className="px-5 py-3 text-left font-medium">{t("adminRevenue.user")}</th>
                    <th className="px-3 py-3 text-right font-medium">
                      {sortButton("revenue", t("adminRevenue.revenue"))}
                    </th>
                    <th className="px-3 py-3 text-right font-medium">
                      {sortButton("calls", t("adminRevenue.calls"))}
                    </th>
                    <th className="px-3 py-3 text-right font-medium">
                      {sortButton("tokens", t("adminRevenue.tokens"))}
                    </th>
                    <th className="px-3 py-3 text-right font-medium">
                      {t("adminRevenue.inputTokens")}
                    </th>
                    <th className="px-3 py-3 text-right font-medium">
                      {t("adminRevenue.outputTokens")}
                    </th>
                    <th className="px-5 py-3 text-right font-medium">
                      {t("adminRevenue.modelCount")}
                    </th>
                    <th className="w-10 px-2 py-3" />
                  </tr>
                </thead>
                <tbody>
                  {dayGroups.map((group) => (
                    <DayGroup
                      key={group.day}
                      day={group.day}
                      rows={group.rows}
                      revenue={group.revenue}
                      users={group.users}
                      expandedRow={expandedRow}
                      formatCost={formatCost}
                      daySortIcon={sortKey === "day" ? sortIcon("day") : null}
                      onToggleDaySort={() => toggleSort("day")}
                      onToggleRow={(rowKey) =>
                        setExpandedRow(expandedRow === rowKey ? null : rowKey)
                      }
                    />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      <Dialog open={exclusionsOpen} onOpenChange={setExclusionsOpen}>
        <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-hidden rounded-2xl p-0 sm:max-w-lg">
          <div className="flex max-h-[calc(100dvh-2rem)] flex-col p-5 sm:p-6">
            <DialogHeader className="shrink-0 pr-10">
              <DialogTitle className="flex items-center gap-2">
                <UserMinus className="size-5 text-primary" />
                {t("adminRevenue.exclusionsTitle")}
              </DialogTitle>
              <DialogDescription className="mt-2 text-pretty">
                {t("adminRevenue.exclusionsHint")}
              </DialogDescription>
            </DialogHeader>
            <div className="relative mt-4 shrink-0">
              <Input
                value={userQuery}
                onChange={(event) => setUserQuery(event.target.value)}
                placeholder={t("adminRevenue.searchUser")}
                className="w-full"
                aria-label={t("adminRevenue.searchUser")}
              />
              {userQuery.trim() && userMatches.length > 0 && (
                <div className="absolute inset-x-0 top-full z-10 mt-1 rounded-lg border bg-popover p-1 shadow-md">
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
            <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
              {exclusionCount === 0 ? (
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
                      className="flex items-center justify-between gap-3 py-3"
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
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </PageWrapper>
  );
}

/** One day's block: a summary header row plus that day's user rows. */
function DayGroup({
  day,
  rows,
  revenue,
  users,
  expandedRow,
  formatCost,
  daySortIcon,
  onToggleDaySort,
  onToggleRow,
}: {
  day: string;
  rows: FlatRow[];
  revenue: bigint;
  users: number;
  expandedRow: string | null;
  formatCost: (nanoUsd: string) => string;
  daySortIcon: ReactNode;
  onToggleDaySort: () => void;
  onToggleRow: (rowKey: string) => void;
}) {
  const { t } = useTranslation();

  return (
    <>
      <tr className="border-b bg-muted/40">
        <td colSpan={8} className="px-5 py-2">
          <div className="flex items-center gap-3 text-xs">
            <button
              type="button"
              onClick={onToggleDaySort}
              className="flex items-center gap-1 font-mono font-semibold hover:text-foreground"
              aria-label={t("adminRevenue.day")}
            >
              {formatDayShort(day)}
              {daySortIcon}
            </button>
            <span className="text-muted-foreground">
              {t("adminRevenue.dayGroupSummary", {
                revenue: formatCost(revenue.toString()),
                count: users,
              })}
            </span>
          </div>
        </td>
      </tr>
      {rows.map((row) => {
        const rowKey = `${row.day}-${row.userId}`;
        return (
          <UserRow
            key={rowKey}
            row={row}
            expanded={expandedRow === rowKey}
            formatCost={formatCost}
            onToggle={() => onToggleRow(rowKey)}
          />
        );
      })}
    </>
  );
}

function UserRow({
  row,
  expanded,
  formatCost,
  onToggle,
}: {
  row: FlatRow;
  expanded: boolean;
  formatCost: (nanoUsd: string) => string;
  onToggle: () => void;
}) {
  const { t } = useTranslation();

  return (
    <>
      <tr
        className={cn(
          "border-b border-border/50 transition-colors duration-200 last:border-b-0 hover:bg-accent/45",
          expanded && "bg-accent/30"
        )}
      >
        <td className="max-w-56 px-5 py-2.5">
          <span className="block truncate font-medium">{row.username || row.userId}</span>
          {row.username && (
            <span className="block truncate font-mono text-xs text-muted-foreground">
              {row.userId}
            </span>
          )}
        </td>
        <td className="whitespace-nowrap px-3 py-2.5 text-right font-mono tabular-nums">
          <CoinAmount value={formatCost(row.chargeNanoUsd.toString())} />
        </td>
        <td className="whitespace-nowrap px-3 py-2.5 text-right font-mono tabular-nums">
          {formatInteger(row.calls)}
        </td>
        <td className="whitespace-nowrap px-3 py-2.5 text-right font-mono tabular-nums">
          {formatInteger(row.inputTokens + row.outputTokens)}
        </td>
        <td className="whitespace-nowrap px-3 py-2.5 text-right font-mono tabular-nums text-muted-foreground">
          {formatInteger(row.inputTokens)}
        </td>
        <td className="whitespace-nowrap px-3 py-2.5 text-right font-mono tabular-nums text-muted-foreground">
          {formatInteger(row.outputTokens)}
        </td>
        <td className="whitespace-nowrap px-5 py-2.5 text-right font-mono tabular-nums text-muted-foreground">
          {formatInteger(row.models.length)}
        </td>
        <td className="px-2 py-2.5">
          <button
            type="button"
            disabled={row.models.length === 0}
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
          <td colSpan={8} className="px-5 py-3">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs text-muted-foreground">
                  <th className="py-1.5 pr-3 font-medium">{t("adminRevenue.model")}</th>
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
                {row.models.map((model) => (
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

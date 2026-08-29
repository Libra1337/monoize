import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ArrowUpRight, Coins, MousePointerClick, RefreshCw } from "lucide-react";

import { RefreshStatusLogo } from "@/components/admin/refresh-status-logo";
import { Badge } from "@/components/ui/badge";
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
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper, motion, transitions } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/skeleton";
import { formatNanoUsd } from "@/lib/exact-decimal";
import type { AdminUsageUserRow } from "@/lib/api";
import { useAdminUsageRanking } from "@/lib/swr";
import { cn } from "@/lib/utils";

function integer(value: string): bigint {
  return /^(?:0|[1-9]\d*)$/.test(value) ? BigInt(value) : 0n;
}

function formatInteger(value: string | bigint): string {
  return (typeof value === "bigint" ? value : integer(value)).toLocaleString("en-US");
}

function totalTokens(row: Pick<AdminUsageUserRow, "input_tokens" | "cache_read_tokens" | "output_tokens">): bigint {
  return integer(row.input_tokens) + integer(row.cache_read_tokens) + integer(row.output_tokens);
}

function percent(value: bigint, total: bigint): number {
  if (value <= 0n || total <= 0n) return 0;
  return Number((value * 10_000n) / total) / 100;
}

function UsageSkeleton() {
  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-2"><Skeleton className="h-8 w-44" /><Skeleton className="h-4 w-72" /></div>
        <Skeleton className="size-9 rounded-lg" />
      </div>
      <Skeleton className="h-36 w-full rounded-xl" />
      <Skeleton className="h-80 w-full rounded-xl" />
    </div>
  );
}

export function AdminUsagePage() {
  const { t } = useTranslation();
  const { data, error, isLoading, isValidating, mutate } = useAdminUsageRanking();
  const [selected, setSelected] = useState<AdminUsageUserRow | null>(null);

  const totals = useMemo(() => {
    if (!data) return { all: 0n, input: 0n, cache: 0n, output: 0n };
    return {
      all: integer(data.total_tokens),
      input: integer(data.total_input_tokens),
      cache: integer(data.total_cache_read_tokens),
      output: integer(data.total_output_tokens),
    };
  }, [data]);

  if (isLoading && !data) return <PageWrapper><UsageSkeleton /></PageWrapper>;
  if (error && !data) {
    return (
      <PageWrapper className="space-y-4">
        <EmptyState
          variant="card"
          icon={<AlertTriangle className="size-8 text-destructive" />}
          title={t("adminUsage.loadFailed")}
          description={error instanceof Error ? error.message : t("common.error")}
        />
        <div className="flex justify-center">
          <Button variant="outline" onClick={() => void mutate()}><RefreshCw data-icon />{t("common.retry")}</Button>
        </div>
      </PageWrapper>
    );
  }
  if (!data) return null;

  const segments = [
    { key: "input", label: t("adminUsage.inputTokens"), value: totals.input, className: "bg-primary" },
    { key: "cache", label: t("adminUsage.cacheTokens"), value: totals.cache, className: "bg-warning" },
    { key: "output", label: t("adminUsage.outputTokens"), value: totals.output, className: "bg-success" },
  ];

  return (
    <PageWrapper className="space-y-5 pb-6">
      <PageHeader
        title={t("adminUsage.title")}
        description={t("adminUsage.description")}
        actions={<RefreshStatusLogo refreshing={isValidating} label={isValidating ? t("adminUsage.refreshing") : t("adminUsage.current")} />}
      />

      <motion.section
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={transitions.normal}
        className="rounded-xl border bg-card p-5 shadow-sm"
      >
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <p className="text-sm font-medium text-muted-foreground">{t("adminUsage.totalTokens")}</p>
            <p className="mt-1 font-mono text-3xl font-semibold tabular-nums">{formatInteger(totals.all)}</p>
          </div>
          <Badge variant="secondary" className="font-mono">{t("adminUsage.window24h")}</Badge>
        </div>
        <div className="mt-5 flex h-3 w-full overflow-hidden rounded-full bg-muted" role="progressbar" aria-valuenow={Number(totals.all > BigInt(Number.MAX_SAFE_INTEGER) ? BigInt(Number.MAX_SAFE_INTEGER) : totals.all)}>
          {totals.all > 0n && segments.map((segment) => (
            <div
              key={segment.key}
              className={cn("h-full border-r border-background/80 transition-[width] duration-700 ease-out last:border-r-0", segment.className)}
              style={{ width: `${percent(segment.value, totals.all)}%` }}
            />
          ))}
        </div>
        <div className="mt-5 grid divide-y border-t sm:grid-cols-3 sm:divide-x sm:divide-y-0">
          {segments.map((segment) => (
            <div key={segment.key} className="flex items-center justify-between gap-3 py-3 sm:px-4 sm:first:pl-0 sm:last:pr-0">
              <span className="flex items-center gap-2 text-sm text-muted-foreground"><span className={cn("size-2 rounded-full", segment.className)} />{segment.label}</span>
              <span className="font-mono text-sm font-medium tabular-nums">{formatInteger(segment.value)}</span>
            </div>
          ))}
        </div>
      </motion.section>

      <div className="grid rounded-xl border bg-card sm:grid-cols-3 sm:divide-x">
        <div className="flex items-center gap-3 p-4"><MousePointerClick className="size-5 text-primary" /><div><p className="text-xs text-muted-foreground">{t("adminUsage.calls")}</p><p className="font-mono text-lg font-semibold">{data.total_calls.toLocaleString("en-US")}</p></div></div>
        <div className="flex items-center gap-3 border-t p-4 sm:border-t-0"><Coins className="size-5 text-warning" /><div><p className="text-xs text-muted-foreground">{t("adminUsage.cost")}</p><p className="font-mono text-lg font-semibold">{formatNanoUsd(data.total_cost_nano_usd, 4)}</p></div></div>
        <div className="flex items-center gap-3 border-t p-4 sm:border-t-0"><ArrowUpRight className="size-5 text-success" /><div><p className="text-xs text-muted-foreground">{t("adminUsage.activeUsers")}</p><p className="font-mono text-lg font-semibold">{data.users.length}</p></div></div>
      </div>

      <Card className="overflow-hidden rounded-xl">
        <CardContent className="p-0">
          <div className="border-b px-5 py-4"><h2 className="font-display text-base font-semibold">{t("adminUsage.ranking")}</h2><p className="mt-1 text-sm text-muted-foreground">{t("adminUsage.rankingHint")}</p></div>
          {data.users.length === 0 ? <EmptyState title={t("adminUsage.empty")} className="py-14" /> : (
            <div className="overflow-x-auto">
              <table className="w-full min-w-[680px] text-sm">
                <thead><tr className="border-b bg-muted/35 text-left text-xs text-muted-foreground"><th className="px-5 py-3 font-medium">#</th><th className="px-3 py-3 font-medium">{t("adminUsage.user")}</th><th className="px-3 py-3 text-right font-medium">Tokens</th><th className="px-3 py-3 text-right font-medium">{t("adminUsage.calls")}</th><th className="px-5 py-3 text-right font-medium">{t("adminUsage.cost")}</th></tr></thead>
                <tbody>{data.users.map((row, index) => (
                  <tr key={row.user_id} className="cursor-pointer border-b transition-colors duration-200 last:border-b-0 hover:bg-accent/45 focus-within:bg-accent/45" onClick={() => setSelected(row)}>
                    <td className="px-5 py-3 font-mono text-muted-foreground">{String(index + 1).padStart(2, "0")}</td>
                    <td className="px-3 py-3"><button type="button" className="w-full text-left focus-visible:outline-none"><span className="block font-medium">{row.username || row.user_id}</span>{row.username && <span className="block max-w-64 truncate font-mono text-xs text-muted-foreground">{row.user_id}</span>}</button></td>
                    <td className="px-3 py-3 text-right font-mono tabular-nums">{formatInteger(totalTokens(row))}</td>
                    <td className="px-3 py-3 text-right font-mono tabular-nums">{row.call_count.toLocaleString("en-US")}</td>
                    <td className="px-5 py-3 text-right font-mono tabular-nums">{formatNanoUsd(row.cost_nano_usd, 4)}</td>
                  </tr>
                ))}</tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      <Dialog open={selected != null} onOpenChange={(open) => !open && setSelected(null)}>
        <DialogContent className="h-[min(42rem,calc(100dvh-2rem))] max-w-3xl rounded-xl" closeLabel={t("common.close")}>
          <DialogHeader className="pr-10"><DialogTitle>{selected?.username || selected?.user_id}</DialogTitle><DialogDescription>{t("adminUsage.modelDetails")}</DialogDescription></DialogHeader>
          <div className="min-h-0 flex-1 overflow-auto rounded-lg border">
            <table className="w-full min-w-[620px] text-sm">
              <thead className="sticky top-0 bg-background"><tr className="border-b text-left text-xs text-muted-foreground"><th className="px-4 py-3 font-medium">{t("adminUsage.model")}</th><th className="px-3 py-3 text-right font-medium">Tokens</th><th className="px-3 py-3 text-right font-medium">{t("adminUsage.calls")}</th><th className="px-4 py-3 text-right font-medium">{t("adminUsage.cost")}</th></tr></thead>
              <tbody>{selected?.models.map((model) => (
                <tr key={model.model} className="border-b last:border-b-0"><td className="max-w-72 truncate px-4 py-3 font-mono">{model.model}</td><td className="px-3 py-3 text-right font-mono">{formatInteger(totalTokens(model))}</td><td className="px-3 py-3 text-right font-mono">{model.call_count.toLocaleString("en-US")}</td><td className="px-4 py-3 text-right font-mono">{formatNanoUsd(model.cost_nano_usd, 4)}</td></tr>
              ))}</tbody>
            </table>
          </div>
        </DialogContent>
      </Dialog>
    </PageWrapper>
  );
}

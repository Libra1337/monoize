import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { UsersRound } from "lucide-react";
import { api, type OrgMemberUsageResponse } from "@/lib/api";
import { formatCost } from "../request-logs/utils";
import { useMyOrgs } from "./shared";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";

const RANGES = [
  { hours: 1, labelKey: "orgUsage.range1h" },
  { hours: 24, labelKey: "orgUsage.range24h" },
  { hours: 168, labelKey: "orgUsage.range7d" },
  { hours: 720, labelKey: "orgUsage.range30d" },
];

function formatTokens(n: number): string {
  return n.toLocaleString("en-US");
}

/** One member's spend series as a proportional bar strip. */
function SeriesStrip({
  series,
}: {
  series: { charge_nano_usd: string; calls: number }[];
}) {
  const charges = series.map((s) => Math.max(0, Number(s.charge_nano_usd)));
  const max = Math.max(1, ...charges);
  return (
    <div className="flex h-8 w-48 items-end gap-px" title={t_calls(series)}>
      {charges.map((c, i) => (
        <div
          key={i}
          className="flex-1 rounded-[1px] bg-primary/70"
          style={{ height: `${Math.max(2, (c / max) * 100)}%` }}
        />
      ))}
    </div>
  );
}

function t_calls(series: { charge_nano_usd: string; calls: number }[]): string {
  return series.map((s) => `${s.calls} calls`).join(", ");
}

export function OrgMemberUsagePage() {
  const { orgId } = useParams();
  const { t } = useTranslation();
  const { data: overview } = useMyOrgs();
  const [rangeHours, setRangeHours] = useState(24);
  const [data, setData] = useState<OrgMemberUsageResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);

  const role = overview?.orgs?.find((o) => o.id === orgId)?.role;
  const isOwner = role === "owner";

  useEffect(() => {
    if (!isOwner) return;
    let cancelled = false;
    setLoading(true);
    const buckets = Math.min(48, Math.max(6, Math.round(rangeHours / 2)));
    api
      .getOrgMemberUsage(orgId ?? "", rangeHours, buckets)
      .then((res) => {
        if (cancelled) return;
        setData(res);
        setError(null);
      })
      .catch((e) => !cancelled && setError(String(e?.message ?? e)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [orgId, rangeHours, isOwner]);

  if (!isOwner) {
    return (
      <div className="p-4 text-sm text-muted-foreground sm:p-6">{t("orgLimits.ownerOnly")}</div>
    );
  }

  const members = [...(data?.members ?? []), ...(data?.removed_members ?? [])].sort(
    (a, b) => Number(b.total_charge_nano_usd) - Number(a.total_charge_nano_usd),
  );

  return (
    <div className="space-y-4 p-4 sm:space-y-6 sm:p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <UsersRound className="h-5 w-5 text-muted-foreground" />
          <h1 className="text-lg font-semibold tracking-tight">{t("orgUsage.title")}</h1>
        </div>
        <div className="flex items-center rounded-md bg-muted p-0.5" role="group">
          {RANGES.map((r) => (
            <button
              key={r.hours}
              type="button"
              onClick={() => setRangeHours(r.hours)}
              className={cn(
                "rounded-[5px] px-2.5 py-1 text-xs font-medium transition-colors",
                rangeHours === r.hours
                  ? "bg-background shadow-sm text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {t(r.labelKey)}
            </button>
          ))}
        </div>
      </div>

      {loading && (
        <div className="space-y-3">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-40 w-full" />
        </div>
      )}
      {error && (
        <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}
      {!loading && !error && members.length === 0 && (
        <div className="rounded-lg border p-6 text-center text-sm text-muted-foreground">
          {t("orgUsage.empty")}
        </div>
      )}

      {!loading && !error && members.length > 0 && (
        <div className="rounded-lg border">
          <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("orgUsage.member")}</TableHead>
                <TableHead className="text-right">{t("orgUsage.charge")}</TableHead>
                <TableHead className="text-right">{t("orgUsage.calls")}</TableHead>
                <TableHead className="text-right">{t("orgUsage.inputTokens")}</TableHead>
                <TableHead className="text-right">{t("orgUsage.outputTokens")}</TableHead>
                <TableHead className="text-right">{t("orgUsage.cacheRead")}</TableHead>
                <TableHead>{t("orgUsage.trend")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {members.map((m) => {
                const isRemoved = (data?.removed_members ?? []).some(
                  (r) => r.user_id === m.user_id,
                );
                const isOpen = expanded === m.user_id;
                return (
                  <>
                    <TableRow
                      key={m.user_id}
                      className="cursor-pointer"
                      onClick={() => setExpanded(isOpen ? null : m.user_id)}
                    >
                      <TableCell className="font-medium">
                        {m.username || m.user_id}
                        {isRemoved && (
                          <span className="ml-2 rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                            {t("orgUsage.removed")}
                          </span>
                        )}
                      </TableCell>
                      <TableCell className="text-right font-mono text-xs">
                        {formatCost(m.total_charge_nano_usd)}
                      </TableCell>
                      <TableCell className="text-right">{formatTokens(m.calls)}</TableCell>
                      <TableCell className="text-right">{formatTokens(m.input_tokens)}</TableCell>
                      <TableCell className="text-right">{formatTokens(m.output_tokens)}</TableCell>
                      <TableCell className="text-right">
                        {formatTokens(m.cache_read_tokens)}
                      </TableCell>
                      <TableCell>
                        <SeriesStrip series={m.series} />
                      </TableCell>
                    </TableRow>
                    {isOpen && (
                      <TableRow key={`${m.user_id}-detail`}>
                        <TableCell colSpan={7} className="bg-muted/25 px-4 py-3">
                          <div className="text-xs font-medium text-muted-foreground">
                            {t("orgUsage.byModel")}
                          </div>
                          <div className="mt-2 space-y-1">
                            {m.by_model
                              .sort(
                                (a, b) => Number(b.charge_nano_usd) - Number(a.charge_nano_usd),
                              )
                              .map((row) => (
                                <div
                                  key={row.model}
                                  className="flex items-center justify-between font-mono text-xs"
                                >
                                  <span>{row.model}</span>
                                  <span>
                                    {formatCost(row.charge_nano_usd)} · {row.calls}{" "}
                                    {t("orgUsage.callsUnit")}
                                  </span>
                                </div>
                              ))}
                            {m.by_model.length === 0 && (
                              <div className="text-xs text-muted-foreground">
                                {t("orgUsage.noModels")}
                              </div>
                            )}
                          </div>
                        </TableCell>
                      </TableRow>
                    )}
                  </>
                );
              })}
            </TableBody>
          </Table>
          </div>
        </div>
      )}
    </div>
  );
}

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Ban, CalendarDays, ShieldAlert, Users } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { EmptyState } from "@/components/ui/empty-state";
import { AnimatedButton, PageWrapper, motion, transitions } from "@/components/ui/motion";
import { PageHeader } from "@/components/ui/page-header";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart";
import { Bar, BarChart, CartesianGrid, XAxis, YAxis } from "recharts";
import { useFirewallEvents, useFirewallStats } from "@/lib/swr";
import type { FirewallEvent } from "@/lib/api";

const PAGE_SIZE = 20;

const RANGE_OPTIONS = [
  { value: "24h", hours: 24 },
  { value: "7d", hours: 24 * 7 },
  { value: "30d", hours: 24 * 30 },
  { value: "all", hours: 0 },
] as const;

const ACTION_OPTIONS = ["all", "blocked", "marked"] as const;

type RangeValue = (typeof RANGE_OPTIONS)[number]["value"];

const endpointLabels: Record<string, string> = {
  chat_completions: "chat/completions",
  responses: "responses",
  messages: "messages",
  responses_compact: "responses/compact",
  embeddings: "embeddings",
  images_generations: "images/generations",
  images_edits: "images/edits",
};

function formatDateTime(unixMs: number): string {
  const date = new Date(unixMs);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours()
  )}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

function StatTile({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof Ban;
  label: string;
  value: number | string;
}) {
  return (
    <div className="flex items-center gap-3 p-4">
      <Icon className="size-5 text-primary" />
      <div>
        <p className="text-xs text-muted-foreground">{label}</p>
        <p className="mt-1 font-mono text-lg font-semibold">{value}</p>
      </div>
    </div>
  );
}

export function FirewallPage() {
  const { t } = useTranslation();
  const [filterTime, setFilterTime] = useState(() => Date.now());
  const [range, setRange] = useState<RangeValue>("7d");
  const [actionFilter, setActionFilter] = useState<(typeof ACTION_OPTIONS)[number]>("all");
  const [termFilter, setTermFilter] = useState("");
  const [termInput, setTermInput] = useState("");
  const [pageOffset, setPageOffset] = useState(0);
  const [selected, setSelected] = useState<FirewallEvent | null>(null);

  const { data: stats, isLoading: statsLoading } = useFirewallStats({ refreshInterval: 15000 });

  const filters = useMemo(() => {
    const option = RANGE_OPTIONS.find((item) => item.value === range);
    return {
      term: termFilter || undefined,
      action: actionFilter === "all" ? undefined : actionFilter,
      since_ms: option?.hours ? filterTime - option.hours * 60 * 60 * 1000 : undefined,
    } as const;
  }, [range, termFilter, actionFilter, filterTime]);

  const { data: events, isLoading: eventsLoading, mutate: mutateEvents } = useFirewallEvents(
    PAGE_SIZE,
    pageOffset,
    filters
  );

  const total = events?.total ?? 0;
  const rows = events?.data ?? [];
  const hasMore = pageOffset + PAGE_SIZE < total;

  const chartConfig = {
    count: { label: t("firewall.blockedRequests"), color: "hsl(var(--primary))" },
  } satisfies ChartConfig;

  const applyTermFilter = () => {
    setFilterTime(Date.now());
    setTermFilter(termInput.trim());
    setPageOffset(0);
    void mutateEvents();
  };

  return (
    <PageWrapper className="flex min-w-0 flex-col gap-6">
      <motion.div initial={{ opacity: 0, y: -10 }} animate={{ opacity: 1, y: 0 }} transition={transitions.normal}>
        <PageHeader
          title={t("firewall.title")}
          description={t("firewall.description")}
        />
      </motion.div>

      {/* CF-26: the firewall blocks nothing while the judge is inert. */}
      {stats && !stats.judge_active ? (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-sm text-amber-600 dark:text-amber-400">
          {t("firewall.judgeInactive")}
        </div>
      ) : null}

      {/* Stat tiles (CF-26) */}
      <motion.div
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={transitions.normal}
        className="grid overflow-hidden rounded-xl border bg-card sm:grid-cols-2 xl:grid-cols-4 sm:divide-x sm:divide-y divide-y sm:divide-y-0"
      >
        {statsLoading || !stats ? (
          Array.from({ length: 4 }).map((_, index) => (
            <div key={index} className="flex items-center gap-3 p-4">
              <Skeleton className="size-5" />
              <div className="flex-1 space-y-2">
                <Skeleton className="h-3 w-20" />
                <Skeleton className="h-5 w-12" />
              </div>
            </div>
          ))
        ) : (
          <>
            <StatTile icon={ShieldAlert} label={t("firewall.last24h")} value={stats.last_24h} />
            <StatTile icon={CalendarDays} label={t("firewall.last7d")} value={stats.last_7d} />
            <StatTile icon={Ban} label={t("firewall.totalBlocked")} value={stats.total} />
            <StatTile icon={Users} label={t("firewall.affectedUsers")} value={stats.distinct_users} />
          </>
        )}
      </motion.div>

      <div className="grid gap-6 xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        {/* 14-day trend (CF-26) */}
        <Card className="p-4">
          <h3 className="mb-3 text-sm font-medium">{t("firewall.trendTitle")}</h3>
          {statsLoading || !stats ? (
            <Skeleton className="h-64 w-full" />
          ) : stats.daily.every((day) => day.count === 0) ? (
            <div className="flex h-64 items-center justify-center text-sm text-muted-foreground">
              {t("firewall.noBlocks")}
            </div>
          ) : (
            <ChartContainer config={chartConfig} className="h-64 w-full !aspect-auto">
              <BarChart data={stats.daily} margin={{ top: 8, right: 12, left: -20, bottom: 0 }}>
                <CartesianGrid vertical={false} />
                <XAxis
                  dataKey="date"
                  tickLine={false}
                  axisLine={false}
                  minTickGap={24}
                  tickFormatter={(value: string) => value.slice(5)}
                />
                <YAxis tickLine={false} axisLine={false} allowDecimals={false} />
                <ChartTooltip content={<ChartTooltipContent />} />
                <Bar dataKey="count" fill="var(--color-count)" radius={[3, 3, 0, 0]} />
              </BarChart>
            </ChartContainer>
          )}
        </Card>

        {/* Top terms (CF-26) */}
        <Card className="p-4">
          <h3 className="mb-3 text-sm font-medium">{t("firewall.topTermsTitle")}</h3>
          {statsLoading || !stats ? (
            <div className="space-y-2">
              {Array.from({ length: 5 }).map((_, index) => (
                <Skeleton key={index} className="h-8 w-full" />
              ))}
            </div>
          ) : stats.top_terms.length === 0 ? (
            <div className="flex h-40 items-center justify-center text-sm text-muted-foreground">
              {t("firewall.noBlocks")}
            </div>
          ) : (
            <ul className="flex flex-col gap-2">
              {stats.top_terms.map((item) => (
                <li key={item.term} className="flex items-center justify-between gap-3">
                  <button
                    type="button"
                    className="min-w-0 truncate font-mono text-sm hover:underline"
                    onClick={() => {
                      setTermInput(item.term);
                      setFilterTime(Date.now());
                      setTermFilter(item.term);
                      setPageOffset(0);
                    }}
                    title={item.term}
                  >
                    {item.term}
                  </button>
                  <Badge variant="secondary" className="font-mono">
                    {item.count}
                  </Badge>
                </li>
              ))}
            </ul>
          )}
        </Card>
      </div>

      {/* Events table (CF-25, CF-26) */}
      <div className="rounded-lg border bg-card">
        <div className="flex flex-col gap-3 border-b p-4 sm:flex-row sm:items-center">
          <div className="flex flex-1 items-center gap-2">
            <Input
              value={termInput}
              placeholder={t("firewall.searchTermPlaceholder")}
              className="max-w-xs"
              onChange={(e) => setTermInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") applyTermFilter();
              }}
            />
            <Button variant="secondary" size="sm" onClick={applyTermFilter}>
              {t("firewall.search")}
            </Button>
          </div>
          <Select
            value={actionFilter}
            onValueChange={(value) => {
              setFilterTime(Date.now());
              setActionFilter(value as (typeof ACTION_OPTIONS)[number]);
              setPageOffset(0);
            }}
          >
            <SelectTrigger className="w-32">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {ACTION_OPTIONS.map((option) => (
                <SelectItem key={option} value={option}>
                  {t(`firewall.action.${option}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={range}
            onValueChange={(value) => {
              setFilterTime(Date.now());
              setRange(value as RangeValue);
              setPageOffset(0);
            }}
          >
            <SelectTrigger className="w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {RANGE_OPTIONS.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {t(`firewall.range.${option.value}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {eventsLoading ? (
          <div className="flex flex-col gap-2 p-4">
            {Array.from({ length: 8 }).map((_, index) => (
              <Skeleton key={index} className="h-9 w-full" />
            ))}
          </div>
        ) : rows.length === 0 ? (
          <EmptyState
            icon={<ShieldAlert className="h-12 w-12" />}
            title={t("firewall.noEvents")}
            description={t("firewall.noEventsDescription")}
          />
        ) : (
          <>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("firewall.column.time")}</TableHead>
                  <TableHead>{t("firewall.column.user")}</TableHead>
                  <TableHead>{t("firewall.column.endpoint")}</TableHead>
                  <TableHead>{t("firewall.column.model")}</TableHead>
                  <TableHead>{t("firewall.column.term")}</TableHead>
                  <TableHead>{t("firewall.column.content")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((event) => (
                  <TableRow
                    key={event.id}
                    className="cursor-pointer"
                    onClick={() => setSelected(event)}
                  >
                    <TableCell className="whitespace-nowrap font-mono text-xs text-muted-foreground">
                      {formatDateTime(event.created_at_unix_ms)}
                    </TableCell>
                    <TableCell className="max-w-40 truncate">
                      {event.username ?? event.user_id ?? "—"}
                    </TableCell>
                    <TableCell className="whitespace-nowrap font-mono text-xs">
                      {endpointLabels[event.endpoint] ?? event.endpoint}
                    </TableCell>
                    <TableCell className="max-w-40 truncate font-mono text-xs">{event.model}</TableCell>
                    <TableCell>
                      <Badge
                        variant={event.action === "marked" ? "secondary" : "destructive"}
                        className="max-w-32 truncate font-mono"
                      >
                        {event.term}
                      </Badge>
                    </TableCell>
                    <TableCell className="max-w-64 truncate text-muted-foreground">
                      {event.content}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            <div className="flex items-center justify-between gap-3 border-t p-3 text-sm text-muted-foreground">
              <span>
                {t("firewall.showing", {
                  from: pageOffset + 1,
                  to: Math.min(pageOffset + rows.length, total),
                  total,
                })}
              </span>
              <div className="flex gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={pageOffset === 0}
                  onClick={() => setPageOffset(Math.max(0, pageOffset - PAGE_SIZE))}
                >
                  {t("firewall.prevPage")}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!hasMore}
                  onClick={() => setPageOffset(pageOffset + PAGE_SIZE)}
                >
                  {t("firewall.nextPage")}
                </Button>
              </div>
            </div>
          </>
        )}
      </div>

      {/* Click-open full-content dialog (CF-21) */}
      <Dialog open={selected !== null} onOpenChange={(open) => !open && setSelected(null)}>
        <DialogContent className="max-h-[80dvh] overflow-auto sm:max-w-2xl">
          {selected ? (
            <>
              <DialogHeader>
                <DialogTitle>{t("firewall.eventDetailTitle")}</DialogTitle>
                <DialogDescription>
                  {formatDateTime(selected.created_at_unix_ms)}
                </DialogDescription>
              </DialogHeader>
              <dl className="grid grid-cols-[auto_minmax(0,1fr)] items-start gap-x-4 gap-y-2 text-sm">
                <dt className="text-muted-foreground">{t("firewall.column.user")}</dt>
                <dd className="break-all">{selected.username ?? selected.user_id ?? "—"}</dd>
                <dt className="text-muted-foreground">{t("firewall.column.endpoint")}</dt>
                <dd className="font-mono text-xs">{endpointLabels[selected.endpoint] ?? selected.endpoint}</dd>
                <dt className="text-muted-foreground">{t("firewall.column.model")}</dt>
                <dd className="break-all font-mono text-xs">{selected.model}</dd>
                <dt className="text-muted-foreground">{t("firewall.column.term")}</dt>
                <dd>
                  <Badge variant="destructive" className="font-mono">
                    {selected.term}
                  </Badge>
                </dd>
                <dt className="text-muted-foreground">{t("firewall.column.apiKey")}</dt>
                <dd className="break-all">{selected.api_key_name ?? selected.api_key_id ?? "—"}</dd>
                {selected.reason ? (
                  <>
                    <dt className="text-muted-foreground">{t("firewall.judgeReason")}</dt>
                    <dd className="break-all text-muted-foreground">{selected.reason}</dd>
                  </>
                ) : null}
              </dl>
              <div>
                <p className="mb-2 text-sm font-medium">{t("firewall.blockedContent")}</p>
                <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-all rounded-md border bg-muted/40 p-3 font-mono text-xs">
                  {selected.content}
                </pre>
              </div>
              <div className="flex justify-end">
                <AnimatedButton>
                  <Button variant="secondary" onClick={() => setSelected(null)}>
                    {t("firewall.close")}
                  </Button>
                </AnimatedButton>
              </div>
            </>
          ) : null}
        </DialogContent>
      </Dialog>
    </PageWrapper>
  );
}

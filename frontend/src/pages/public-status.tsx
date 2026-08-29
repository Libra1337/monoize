import useSWR from "swr";
import { useTranslation } from "react-i18next";
import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  ChevronRight,
  CircleHelp,
  Clock3,
  XCircle,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { RefreshStatusLogo } from "@/components/admin/refresh-status-logo";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { StaggerItem, StaggerList } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

type PublicState =
  | "operational"
  | "minor_degradation"
  | "major_degradation"
  | "unavailable"
  | "insufficient_data";

interface PublicStatusProvider {
  public_name: string;
  state: PublicState;
  success_rate_24h_basis_points: number | null;
}

interface PublicStatusModel {
  name: string;
  state: PublicState;
  success_rate_24h_basis_points: number | null;
}

interface PublicStatusGroup {
  public_name: string;
  state: PublicState;
  insufficient_provider_count: number;
  success_rate_24h_basis_points: number | null;
  last_observed_at: string | null;
  timeline: Array<{ started_at: string; state: PublicState }>;
  models: PublicStatusModel[];
  providers: PublicStatusProvider[];
}

interface PublicStatusResponse {
  generated_at: string;
  data_through: string;
  data_complete: boolean;
  groups: PublicStatusGroup[];
}

async function fetchPublicStatus(): Promise<PublicStatusResponse> {
  const response = await fetch("/api/public/status", { credentials: "omit" });
  const data = await response.json();
  if (!response.ok) {
    throw new Error(data.error?.message || data.error?.code || "Request failed");
  }
  return data;
}

function StateIcon({ state, className }: { state: PublicState; className?: string }) {
  if (state === "operational") return <CheckCircle2 className={className} />;
  if (state === "insufficient_data") return <CircleHelp className={className} />;
  if (state === "unavailable") return <XCircle className={className} />;
  return <AlertTriangle className={className} />;
}

function stateTextClass(state: PublicState) {
  if (state === "operational") return "text-success";
  if (state === "unavailable") return "text-destructive";
  if (state === "insufficient_data") return "text-muted-foreground";
  return "text-warning-foreground";
}

function stateBadgeClass(state: PublicState) {
  if (state === "operational") return "border-success-border bg-success-soft text-success-foreground";
  if (state === "unavailable") return "border-destructive/30 bg-destructive/10 text-destructive";
  if (state === "insufficient_data") return "border-border bg-muted text-muted-foreground";
  return "border-warning-border bg-warning-soft text-warning-foreground";
}

function stateBucketClass(state: PublicState) {
  if (state === "operational") return "bg-success";
  if (state === "minor_degradation") return "bg-warning/55";
  if (state === "major_degradation") return "bg-warning";
  if (state === "unavailable") return "bg-destructive";
  return "bg-muted-foreground/25";
}

function percent(value: number | null) {
  return value == null ? "--" : `${(value / 100).toFixed(2)}%`;
}

function worstState(groups: PublicStatusGroup[]): PublicState {
  const states = groups.map((group) => group.state);
  if (states.includes("unavailable")) return "unavailable";
  if (states.includes("major_degradation")) return "major_degradation";
  if (states.includes("minor_degradation")) return "minor_degradation";
  if (states.includes("operational")) return "operational";
  return "insufficient_data";
}

export function PublicStatusPage({ refreshInterval = 30_000, dashboard = false }: { refreshInterval?: number; dashboard?: boolean } = {}) {
  const { t, i18n } = useTranslation();
  const { data, error, isLoading, isValidating } = useSWR<PublicStatusResponse>(
    "/api/public/status",
    fetchPublicStatus,
    { refreshInterval, keepPreviousData: true },
  );
  const locale = i18n.resolvedLanguage || i18n.language;

  return (
    <main className="mx-auto max-w-6xl px-4 py-10 sm:px-6 sm:py-14 lg:px-8">
      <header className={dashboard ? "flex items-start justify-between gap-4" : undefined}>
        <div className="min-w-0 max-w-3xl">
        <p className="font-mono text-sm text-primary">SERVICE STATUS</p>
        <h1 className="mt-3 font-display text-4xl font-semibold tracking-tight text-balance sm:text-5xl">
          {t("publicSite.status.title")}
        </h1>
        <p className="mt-4 text-base leading-7 text-muted-foreground sm:text-lg sm:leading-8">
          {t("publicSite.status.description")}
        </p>
        </div>
        {dashboard ? <RefreshStatusLogo refreshing={isValidating} label={isValidating ? t("adminRuntime.refreshing") : t("adminRuntime.current")} /> : null}
      </header>

      {isLoading && !data ? (
        <StatusSkeleton />
      ) : error ? (
        <Card className="mt-10 flex gap-3 rounded-xl border-warning-border bg-warning-soft p-5 text-warning-foreground">
          <AlertTriangle className="shrink-0" />
          <div>
            <h2 className="font-semibold">{t("publicSite.status.unavailableTitle")}</h2>
            <p className="mt-1">{t("publicSite.status.unavailableDescription")}</p>
          </div>
        </Card>
      ) : data ? (
        <StatusContent data={data} locale={locale} />
      ) : null}
    </main>
  );
}

function StatusContent({ data, locale }: { data: PublicStatusResponse; locale: string }) {
  const { t } = useTranslation();
  const overallState = worstState(data.groups);
  const incidentCount = data.groups.filter(
    (group) => group.state !== "operational" && group.state !== "insufficient_data",
  ).length;

  return (
    <div className="mt-10 space-y-8">
      {!data.data_complete ? (
        <Card className="flex gap-3 rounded-xl border-warning-border bg-warning-soft p-5 text-warning-foreground">
          <AlertTriangle className="shrink-0" />
          <p>{t("publicSite.status.incomplete")}</p>
        </Card>
      ) : null}

      <Card className="overflow-hidden rounded-xl">
        <div className="flex flex-col gap-5 p-5 sm:flex-row sm:items-center sm:justify-between sm:p-6">
          <div className="flex min-w-0 items-start gap-4">
            <div className={cn("flex size-11 shrink-0 items-center justify-center rounded-full border", stateBadgeClass(overallState))}>
              <StateIcon state={overallState} className="size-5" />
            </div>
            <div className="min-w-0">
              <p className="text-sm text-muted-foreground">{t("publicSite.status.overallLabel")}</p>
              <h2 className="mt-1 font-display text-xl font-semibold sm:text-2xl">
                {overallState === "insufficient_data"
                  ? t("publicSite.status.overallInsufficient")
                  : incidentCount === 0
                  ? t("publicSite.status.overallOperational")
                  : t("publicSite.status.overallIncident", { count: incidentCount })}
              </h2>
            </div>
          </div>
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Clock3 className="size-4" />
            <span>{t("publicSite.status.updatedAt", { time: new Date(data.generated_at).toLocaleString(locale) })}</span>
          </div>
        </div>
      </Card>

      <section aria-labelledby="status-legend-title">
        <div className="flex flex-col gap-1 sm:flex-row sm:items-end sm:justify-between">
          <div>
            <h2 id="status-legend-title" className="font-display text-lg font-semibold">
              {t("publicSite.status.legendTitle")}
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">{t("publicSite.status.legendDescription")}</p>
          </div>
          <p className="mt-2 text-sm text-muted-foreground sm:mt-0">
            {t("publicSite.status.dataThrough", { time: new Date(data.data_through).toLocaleString(locale) })}
          </p>
        </div>
        <ul className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
          {(["operational", "minor_degradation", "major_degradation", "unavailable", "insufficient_data"] as PublicState[]).map((state) => (
            <li key={state} className="flex min-h-12 items-center gap-3 rounded-lg border px-3 py-2">
              <span className={cn("size-2.5 shrink-0 rounded-sm", stateBucketClass(state))} />
              <span className="min-w-0">
                <span className="block text-sm font-medium">{t(`publicSite.status.states.${state}`)}</span>
                <span className="block text-xs text-muted-foreground">{t(`publicSite.status.thresholds.${state}`)}</span>
              </span>
            </li>
          ))}
        </ul>
      </section>

      <StaggerList className="space-y-5">
        {data.groups.map((group) => (
          <StaggerItem key={group.public_name}>
            <GroupStatus group={group} locale={locale} />
          </StaggerItem>
        ))}
      </StaggerList>
    </div>
  );
}

function GroupStatus({ group, locale }: { group: PublicStatusGroup; locale: string }) {
  const { t } = useTranslation();

  return (
    <section className="overflow-hidden rounded-xl border bg-card" aria-labelledby={`status-${group.public_name}`}>
      <div className="flex flex-col gap-5 border-b p-5 sm:flex-row sm:items-start sm:justify-between sm:p-6">
        <div className="flex min-w-0 items-start gap-3">
          <StateIcon state={group.state} className={cn("mt-0.5 size-6 shrink-0", stateTextClass(group.state))} />
          <div className="min-w-0">
            <h2 id={`status-${group.public_name}`} className="break-words font-display text-xl font-semibold">
              {group.public_name}
            </h2>
            <span className={cn("mt-2 inline-flex rounded-full border px-2.5 py-1 text-xs font-medium", stateBadgeClass(group.state))}>
              {t(`publicSite.status.states.${group.state}`)}
            </span>
          </div>
        </div>
        <div className="sm:text-right">
          <p className="font-display text-3xl font-semibold tabular-nums">
            {percent(group.success_rate_24h_basis_points)}
          </p>
          <p className="mt-1 text-sm text-muted-foreground">{t("publicSite.status.successRate")}</p>
        </div>
      </div>

      <div className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <p className="text-sm font-medium">{t("publicSite.status.windowLabel")}</p>
          <p className="text-xs text-muted-foreground">{t("publicSite.status.bucketDuration")}</p>
        </div>
        <div
          className="mt-3 grid h-10 w-full gap-1"
          style={{ gridTemplateColumns: "repeat(48, minmax(3px, 1fr))" }}
          aria-label={t("publicSite.status.timelineLabel", { group: group.public_name })}
        >
          {group.timeline.map((bucket) => (
            <span
              key={bucket.started_at}
              className={cn("h-full min-w-0 rounded-sm transition-opacity duration-200 hover:opacity-70", stateBucketClass(bucket.state))}
              title={`${new Date(bucket.started_at).toLocaleString(locale)} - ${t(`publicSite.status.states.${bucket.state}`)}`}
              aria-label={`${new Date(bucket.started_at).toLocaleString(locale)} - ${t(`publicSite.status.states.${bucket.state}`)}`}
              role="img"
            />
          ))}
        </div>

        <div className="mt-4 flex flex-col gap-3 border-t pt-4 sm:flex-row sm:items-center sm:justify-between">
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            <Activity className="size-4" />
            {group.last_observed_at
              ? t("publicSite.status.lastObserved", { time: new Date(group.last_observed_at).toLocaleString(locale) })
              : t("publicSite.status.noObservation")}
          </p>
          <ModelStatusDialog group={group} />
        </div>
      </div>

      <div className="border-t bg-muted/25 px-5 py-4 sm:px-6">
        <p className="text-xs font-medium uppercase text-muted-foreground">{t("publicSite.status.providersTitle")}</p>
        <ul className="mt-3 grid gap-2 sm:grid-cols-2">
          {group.providers.map((provider) => (
              <li key={provider.public_name} className="flex min-h-11 items-center gap-3 rounded-lg border bg-background px-3 py-2">
                <StateIcon state={provider.state} className={cn("size-4 shrink-0", stateTextClass(provider.state))} />
                <span className="min-w-0 flex-1 truncate text-sm font-medium">{provider.public_name}</span>
                <span className="text-xs tabular-nums text-muted-foreground">{percent(provider.success_rate_24h_basis_points)}</span>
              </li>
          ))}
        </ul>
      </div>
    </section>
  );
}

function ModelStatusDialog({ group }: { group: PublicStatusGroup }) {
  const { t } = useTranslation();

  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button type="button" variant="outline" className="min-h-11 rounded-lg">
          {t("publicSite.status.modelsAction", { count: group.models.length })}
          <ChevronRight className="size-4" />
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[min(42rem,calc(100vh-2rem))] max-w-2xl rounded-xl" closeLabel={t("common.close")}>
        <DialogHeader className="pr-8">
          <DialogTitle>{t("publicSite.status.modelDialogTitle", { group: group.public_name })}</DialogTitle>
          <DialogDescription>{t("publicSite.status.modelDialogDescription")}</DialogDescription>
        </DialogHeader>
        {group.models.length === 0 ? (
          <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">
            {t("publicSite.status.noModels")}
          </div>
        ) : (
          <ul className="divide-y overflow-hidden rounded-lg border">
            {group.models.map((model) => (
                <li key={model.name} className="grid min-h-14 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 px-4 py-3">
                  <StateIcon state={model.state} className={cn("size-4", stateTextClass(model.state))} />
                  <div className="min-w-0">
                    <p className="truncate font-mono text-sm font-medium">{model.name}</p>
                    <p className="mt-1 text-xs text-muted-foreground">{t(`publicSite.status.states.${model.state}`)}</p>
                  </div>
                  <span className="text-sm font-medium tabular-nums">{percent(model.success_rate_24h_basis_points)}</span>
                </li>
            ))}
          </ul>
        )}
      </DialogContent>
    </Dialog>
  );
}

function StatusSkeleton() {
  return (
    <div className="mt-10 space-y-8" aria-hidden="true">
      <Skeleton className="h-28 rounded-xl" />
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
        {Array.from({ length: 5 }).map((_, index) => <Skeleton key={index} className="h-14 rounded-lg" />)}
      </div>
      <Skeleton className="h-80 rounded-xl" />
      <Skeleton className="h-80 rounded-xl" />
    </div>
  );
}

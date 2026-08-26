import useSWR from "swr";
import { useTranslation } from "react-i18next";
import { AlertTriangle, CheckCircle2, CircleHelp, XCircle } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

type PublicState = "operational" | "minor_degradation" | "major_degradation" | "unavailable" | "insufficient_data";
interface PublicStatusResponse { generated_at: string; data_through: string; data_complete: boolean; groups: Array<{ public_name: string; state: PublicState; insufficient_provider_count: number; providers: Array<{ public_name: string; state: PublicState; success_rate_24h_basis_points: number | null }> }> }

async function fetchPublicStatus(): Promise<PublicStatusResponse> {
  const response = await fetch("/api/public/status", { credentials: "omit" });
  const data = await response.json();
  if (!response.ok) throw new Error(data.error?.message || data.error?.code || "Request failed");
  return data;
}

function stateIcon(state: PublicState) {
  if (state === "operational") return CheckCircle2;
  if (state === "insufficient_data") return CircleHelp;
  return state === "unavailable" ? XCircle : AlertTriangle;
}

export function PublicStatusPage() {
  const { t } = useTranslation();
  const { data, error, isLoading } = useSWR<PublicStatusResponse>("/api/public/status", fetchPublicStatus, { refreshInterval: 30_000 });
  return <div className="mx-auto max-w-5xl px-4 py-12 sm:px-6 lg:px-8"><header><p className="font-mono text-sm text-primary">SERVICE STATUS</p><h1 className="mt-3 font-display text-4xl font-semibold">{t("publicSite.status.title")}</h1><p className="mt-4 max-w-2xl text-lg leading-8 text-muted-foreground">{t("publicSite.status.description")}</p></header>{isLoading ? <div className="mt-10 space-y-4"><Skeleton className="h-24" /><Skeleton className="h-56" /></div> : error ? <Card className="mt-10 flex gap-3 border-warning/40 bg-warning-soft p-5 text-warning-foreground"><AlertTriangle className="shrink-0" /><div><h2 className="font-semibold">{t("publicSite.status.unavailableTitle")}</h2><p className="mt-1">{t("publicSite.status.unavailableDescription")}</p></div></Card> : data ? <div className="mt-10 space-y-5">{!data.data_complete && <Card className="flex gap-3 border-warning/40 bg-warning-soft p-5 text-warning-foreground"><AlertTriangle className="shrink-0" /><p>{t("publicSite.status.incomplete")}</p></Card>}<Card className="p-5"><p className="text-sm text-muted-foreground">{t("publicSite.status.dataThrough", { time: new Date(data.data_through).toLocaleString() })}</p></Card>{data.groups.map((group) => { const GroupIcon = stateIcon(group.state); return <section key={group.public_name} className="overflow-hidden rounded-lg border bg-card"><div className="flex items-center justify-between gap-4 border-b p-5"><div><h2 className="text-xl font-semibold">{group.public_name}</h2><p className="mt-1 text-sm text-muted-foreground">{t(`publicSite.status.states.${group.state}`)}</p></div><GroupIcon className={cn("size-6", group.state === "operational" ? "text-success" : group.state === "insufficient_data" ? "text-muted-foreground" : "text-warning")} /></div><ul className="divide-y">{group.providers.map((provider) => { const Icon = stateIcon(provider.state); return <li key={provider.public_name} className="flex min-h-16 items-center gap-3 px-5 py-3"><Icon className={cn("size-5 shrink-0", provider.state === "operational" ? "text-success" : provider.state === "insufficient_data" ? "text-muted-foreground" : "text-warning")} /><span className="min-w-0 flex-1 truncate font-medium">{provider.public_name}</span><span className="text-right text-sm text-muted-foreground">{provider.success_rate_24h_basis_points == null ? t(`publicSite.status.states.${provider.state}`) : `${(provider.success_rate_24h_basis_points / 100).toFixed(2)}%`}</span></li>})}</ul></section>})}</div> : null}</div>;
}

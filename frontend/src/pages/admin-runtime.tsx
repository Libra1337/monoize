import { useTranslation } from "react-i18next";
import { Activity, AlertTriangle, Database, HardDrive, Network, RefreshCw, Server } from "lucide-react";

import { RefreshStatusLogo } from "@/components/admin/refresh-status-logo";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper, motion, transitions } from "@/components/ui/motion";
import { CardsPageSkeleton } from "@/components/ui/page-skeleton";
import { useAdminOverview } from "@/lib/swr";

function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return days > 0 ? `${days}d ${hours}h` : hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`;
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index += 1; }
  return `${value.toFixed(1)} ${units[index]}`;
}

export function AdminRuntimePage() {
  const { t } = useTranslation();
  const { data, error, isLoading, isValidating, mutate } = useAdminOverview({ refreshInterval: 2000 });
  if (isLoading && !data) return <PageWrapper><CardsPageSkeleton count={4} /></PageWrapper>;
  if (error && !data) {
    return (
      <PageWrapper className="space-y-4">
        <EmptyState variant="card" icon={<AlertTriangle className="size-8 text-destructive" />} title={t("adminRuntime.loadFailed")} description={error instanceof Error ? error.message : t("common.error")} />
        <div className="flex justify-center"><Button variant="outline" onClick={() => void mutate()}><RefreshCw data-icon />{t("common.retry")}</Button></div>
      </PageWrapper>
    );
  }
  if (!data) return null;

  return (
    <PageWrapper className="space-y-5 pb-6">
      <PageHeader title={t("adminRuntime.title")} description={t("adminRuntime.description")} actions={<RefreshStatusLogo refreshing={isValidating} label={isValidating ? t("adminRuntime.refreshing") : t("adminRuntime.current")} />} />

      <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={transitions.normal} className="grid overflow-hidden rounded-xl border bg-card sm:grid-cols-2 xl:grid-cols-4 sm:[&>*:nth-child(even)]:border-l xl:[&>*]:border-l xl:[&>*:first-child]:border-l-0">
        {[
          [Server, t("adminRuntime.nodeRole"), data.node.role],
          [Activity, t("adminRuntime.uptime"), formatUptime(data.node.uptime_seconds)],
          [Network, t("adminRuntime.sseConnections"), data.system.sse_connections.toLocaleString("en-US")],
          [HardDrive, t("adminRuntime.pendingLogs"), data.system.pending_request_logs.toLocaleString("en-US")],
        ].map(([Icon, label, value], index) => {
          const MetricIcon = Icon as typeof Server;
          return <div key={String(label)} className={`flex items-center gap-3 p-4 ${index >= 2 ? "border-t xl:border-t-0" : ""}`}><MetricIcon className="size-5 text-primary" /><div><p className="text-xs text-muted-foreground">{String(label)}</p><p className="mt-1 font-mono text-lg font-semibold">{String(value)}</p></div></div>;
        })}
      </motion.div>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card className="rounded-xl"><CardHeader className="pb-3"><CardTitle className="flex items-center gap-2 text-base"><Database className="size-4 text-primary" />{t("adminRuntime.node")}</CardTitle></CardHeader><CardContent className="grid gap-3 text-sm">
          {[
            [t("adminRuntime.version"), data.node.version], [t("adminRuntime.startedAt"), new Date(data.node.started_at).toLocaleString()], [t("adminRuntime.listen"), data.node.listen], [t("adminRuntime.database"), `${data.node.database_backend} · ${data.node.database_dsn_redacted}`], [t("adminRuntime.routingRevision"), data.system.routing_config_revision],
          ].map(([label, value]) => <div key={label} className="flex items-start justify-between gap-4 border-b pb-2 last:border-b-0 last:pb-0"><span className="text-muted-foreground">{label}</span><span className="max-w-[65%] break-all text-right font-mono text-xs">{value}</span></div>)}
        </CardContent></Card>
        <Card className="rounded-xl"><CardHeader className="pb-3"><CardTitle className="flex items-center gap-2 text-base"><Network className="size-4 text-primary" />{t("adminRuntime.replicas")}</CardTitle></CardHeader><CardContent>
          <div className="mb-3 flex items-center justify-between text-sm"><span className="text-muted-foreground">{t("adminRuntime.spool")}</span><span className="font-mono">{data.replica.spool_pending_count} · {formatBytes(data.replica.spool_pending_bytes)}</span></div>
          {data.replica.replicas.length === 0 ? <EmptyState title={t("adminRuntime.noReplicas")} className="py-8" /> : <div className="grid gap-2">{data.replica.replicas.map((replica) => <div key={replica.id} className="flex items-center justify-between rounded-lg border p-3"><div className="min-w-0"><p className="truncate font-medium">{replica.hostname}</p><p className="truncate font-mono text-xs text-muted-foreground">{replica.listen}</p></div><Badge variant="secondary" className={replica.stale ? "text-destructive" : "text-success"}>{replica.stale ? t("adminRuntime.stale") : t("adminRuntime.live")}</Badge></div>)}</div>}
        </CardContent></Card>
      </div>

      <Card className="overflow-hidden rounded-xl"><CardHeader className="border-b pb-4"><CardTitle className="flex items-center justify-between gap-3 text-base"><span>{t("adminRuntime.channels")}</span><span className="text-xs font-normal text-muted-foreground">{data.channel_health.length} {t("adminRuntime.entries")}</span></CardTitle></CardHeader><CardContent className="p-0">
        {data.channel_health.length === 0 ? <EmptyState title={t("adminRuntime.noChannels")} className="py-12" /> : <div className="overflow-x-auto"><table className="w-full min-w-[720px] text-sm"><thead><tr className="border-b bg-muted/35 text-left text-xs text-muted-foreground"><th className="px-5 py-3 font-medium">Provider / Channel</th><th className="px-3 py-3 font-medium">{t("adminRuntime.status")}</th><th className="px-3 py-3 text-right font-medium">{t("adminRuntime.todayCalls")}</th><th className="px-5 py-3 text-right font-medium">{t("adminRuntime.unhealthyModels")}</th></tr></thead><tbody>{data.channel_health.map((channel) => <tr key={channel.channel_id} className="border-b last:border-b-0"><td className="px-5 py-3"><p className="font-medium">{channel.provider_name}</p><p className="text-xs text-muted-foreground">{channel.channel_name}</p></td><td className="px-3 py-3"><Badge variant="secondary" className={!channel.enabled || !channel.healthy ? "text-destructive" : "text-success"}>{!channel.enabled ? t("adminRuntime.disabled") : channel.cooldown_active ? t("adminRuntime.cooldown") : channel.healthy ? t("adminRuntime.healthy") : t("adminRuntime.unhealthy")}</Badge></td><td className="px-3 py-3 text-right font-mono">{channel.today_calls.toLocaleString("en-US")}</td><td className="max-w-72 px-5 py-3 text-right font-mono text-xs text-muted-foreground"><span className="block truncate">{channel.unhealthy_models.length ? channel.unhealthy_models.join(", ") : "-"}</span></td></tr>)}</tbody></table></div>}
      </CardContent></Card>
    </PageWrapper>
  );
}

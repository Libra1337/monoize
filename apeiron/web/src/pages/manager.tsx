import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { Boxes, HardDrive, Server } from "lucide-react";
import { api, type Provider } from "@/lib/api";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { BalanceChip, TopUpButton, WorkHeader } from "@/components/app-shell";
import { Badge, Skeleton } from "@/components/ui/badge";
import { formatTime } from "@/lib/format";

interface NodePack {
  id: string;
  name: string;
  version: string;
  description: string;
  nodes: string[];
  enabled: boolean;
  source: string;
}

export function ManagerPage() {
  const { t } = useTranslation();
  const { data: packs, isLoading } = useSWR("node-packs", () =>
    api.get<{ packs: NodePack[] }>("/node-packs"),
  );
  const { data: providers } = useSWR("providers-admin", () =>
    api.get<{ providers: Provider[] }>("/admin/providers"),
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <WorkHeader
        title={t("manager.title")}
        description={t("manager.description")}
        actions={
          <>
            <BalanceChip />
            <TopUpButton />
          </>
        }
      />
      <div className="flex-1 overflow-y-auto p-6 pb-24 md:pb-6">

      <section className="space-y-3">
        <h2 className="flex items-center gap-2 font-display text-lg font-semibold tracking-tight">
          <Boxes className="h-4 w-4 text-primary" />
          {t("manager.packsTitle")}
        </h2>
        {isLoading ? (
          <div className="grid gap-3 sm:grid-cols-3">
            {[0, 1, 2].map((index) => (
              <Skeleton key={index} className="h-28" />
            ))}
          </div>
        ) : (
          <div className="grid gap-3 sm:grid-cols-3">
            {(packs?.packs ?? []).map((pack) => (
              <Card key={pack.id}>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle>{pack.name}</CardTitle>
                    <Badge className="font-mono text-[10px]">
                      v{pack.version}
                    </Badge>
                  </div>
                  <CardDescription>{pack.description}</CardDescription>
                </CardHeader>
                <CardContent className="space-y-2">
                  <div className="text-xs text-muted-foreground">
                    {t("manager.contains")}:{" "}
                    <span className="font-mono">{pack.nodes.join(", ")}</span>
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {pack.enabled ? t("manager.enabled") : t("manager.disabled")}
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        )}
      </section>

      <section className="space-y-3">
        <h2 className="flex items-center gap-2 font-display text-lg font-semibold tracking-tight">
          <Server className="h-4 w-4 text-primary" />
          {t("manager.providersTitle")}
        </h2>
        {(providers?.providers ?? []).length === 0 ? (
          <p className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
            {t("manager.noProviders")}
          </p>
        ) : (
          <div className="space-y-2">
            {(providers?.providers ?? []).map((provider) => (
              <div
                key={provider.id}
                className="flex flex-wrap items-center gap-3 rounded-lg border bg-card px-4 py-3 text-sm"
              >
                <span className="font-medium">{provider.name}</span>
                <Badge className="font-mono text-[10px]">
                  {provider.kind} · {provider.upstream_kind}
                </Badge>
                <span className="truncate font-mono text-xs text-muted-foreground">
                  {provider.base_url}
                </span>
                <span className="ml-auto text-xs text-muted-foreground">
                  {provider.enabled ? t("manager.enabled") : t("manager.disabled")}
                </span>
              </div>
            ))}
          </div>
        )}
      </section>

      <section className="space-y-3">
        <h2 className="flex items-center gap-2 font-display text-lg font-semibold tracking-tight">
          <HardDrive className="h-4 w-4 text-primary" />
          {t("manager.workersTitle")}
        </h2>
        <Card>
          <CardContent className="p-4 text-sm text-muted-foreground">
            {t("manager.workersBody")}
            <div className="mt-2 font-mono text-xs">
              server · engine tick 1 s · updated {formatTime(new Date().toISOString())}
            </div>
          </CardContent>
        </Card>
      </section>
    </div>
  </div>
  );
}

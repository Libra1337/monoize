import { useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { api, type Provider, type Run } from "@/lib/api";
import { StatusDot } from "@/components/ui/page";
import { BalanceChip, TopUpButton, WorkHeader } from "@/components/app-shell";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input, Select } from "@/components/ui/input";
import { Label, Switch } from "@/components/ui/controls";
import { Skeleton } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { formatTime } from "@/lib/format";

const PROVIDER_KINDS = ["llm", "image", "video", "tts", "material"];
const UPSTREAM_KINDS = ["openai", "openai_video", "fal_video", "replicate", "openai_tts", "pexels"];
const PRICE_KEYS = [
  "price_image_nano",
  "price_video_nano",
  "price_tts_nano",
  "price_assemble_nano",
  "price_material_nano",
  "price_subtitle_nano",
];

interface ProviderDraft {
  id?: string;
  kind: string;
  upstream_kind: string;
  name: string;
  base_url: string;
  api_key: string;
  model: string;
  params: string;
  enabled: boolean;
  weight: number;
}

const emptyDraft: ProviderDraft = {
  kind: "llm",
  upstream_kind: "openai",
  name: "",
  base_url: "",
  api_key: "",
  model: "",
  params: "{}",
  enabled: true,
  weight: 0,
};

type Tab = "providers" | "prices" | "templates" | "runs";

export function AdminPage() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("providers");
  const { data: providers, mutate: reloadProviders } = useSWR("admin-providers", () =>
    api.get<{ providers: Provider[] }>("/admin/providers"),
  );
  const { data: settings, mutate: reloadSettings } = useSWR("admin-settings", () =>
    api.get<{ prices: Record<string, string> }>("/admin/settings"),
  );
  const { data: runs, mutate: reloadRuns } = useSWR("admin-runs", () =>
    api.get<{ runs: Run[] }>("/admin/runs"),
  );

  const [draft, setDraft] = useState<ProviderDraft | null>(null);
  const [prices, setPrices] = useState<Record<string, string> | null>(null);

  const tabs: { key: Tab; label: string }[] = [
    { key: "providers", label: t("admin.tabs.providers") },
    { key: "prices", label: t("admin.tabs.prices") },
    { key: "templates", label: t("admin.tabs.templates") },
    { key: "runs", label: t("admin.tabs.runs") },
  ];

  const effectivePrices = prices ?? settings?.prices ?? null;

  async function saveProvider() {
    if (!draft) return;
    let params: unknown;
    try {
      params = JSON.parse(draft.params || "{}");
    } catch {
      toast.error("params JSON invalid");
      return;
    }
    const body = {
      kind: draft.kind,
      upstream_kind: draft.upstream_kind,
      name: draft.name,
      base_url: draft.base_url,
      model: draft.model || null,
      params,
      enabled: draft.enabled,
      weight: draft.weight,
      ...(draft.api_key ? { api_key: draft.api_key } : {}),
    };
    try {
      if (draft.id) {
        await api.put(`/admin/providers/${draft.id}`, body);
      } else {
        await api.post("/admin/providers", body);
      }
      setDraft(null);
      void reloadProviders();
    } catch (error) {
      toast.error(String(error));
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <WorkHeader
        title={t("admin.title")}
        description={t("admin.description")}
        actions={
          <>
            <BalanceChip />
            <TopUpButton />
          </>
        }
      />
      <div className="flex-1 overflow-y-auto p-6 pb-24 md:pb-6">
      <div className="flex flex-wrap gap-1">
        {tabs.map((item) => (
          <button
            key={item.key}
            onClick={() => setTab(item.key)}
            className={cn(
              "rounded-md px-2.5 py-1.5 text-sm font-medium transition-colors",
              tab === item.key
                ? "bg-accent text-accent-foreground"
                : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
            )}
          >
            {item.label}
          </button>
        ))}
      </div>

      {tab === "providers" ? (
        <div className="space-y-3">
          <Button variant="primary" size="sm" onClick={() => setDraft({ ...emptyDraft })}>
            <Plus />
            {t("admin.addProvider")}
          </Button>
          {(providers?.providers ?? []).map((provider) => (
            <div
              key={provider.id}
              className="flex flex-wrap items-center gap-3 rounded-lg border bg-card px-4 py-3 text-sm"
            >
              <span className="font-medium">{provider.name}</span>
              <span className="rounded-md border px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                {provider.kind} · {provider.upstream_kind}
              </span>
              <span className="truncate font-mono text-xs text-muted-foreground">
                {provider.base_url} {provider.model ? `· ${provider.model}` : ""}
              </span>
              <span className="ml-auto flex items-center gap-1">
                <Button variant="ghost" size="icon" aria-label={t("admin.editProvider")} onClick={() => setDraft({
                  id: provider.id,
                  kind: provider.kind,
                  upstream_kind: provider.upstream_kind,
                  name: provider.name,
                  base_url: provider.base_url,
                  api_key: "",
                  model: provider.model ?? "",
                  params: JSON.stringify(provider.params ?? {}, null, 2),
                  enabled: provider.enabled,
                  weight: provider.weight,
                })}>
                  <Pencil />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("common.delete")}
                  onClick={async () => {
                    try {
                      await api.delete(`/admin/providers/${provider.id}`);
                      void reloadProviders();
                    } catch (error) {
                      toast.error(String(error));
                    }
                  }}
                >
                  <Trash2 />
                </Button>
              </span>
            </div>
          ))}
          {!providers ? <Skeleton className="h-16" /> : null}
        </div>
      ) : null}

      {tab === "prices" ? (
        <Card>
          <CardContent className="space-y-4 p-4">
            <p className="text-sm text-muted-foreground">{t("admin.pricesNote")}</p>
            <div className="grid gap-4 sm:grid-cols-2">
              {effectivePrices
                ? PRICE_KEYS.map((key) => (
                    <div key={key} className="space-y-2">
                      <Label htmlFor={key}>{key}</Label>
                      <Input
                        id={key}
                        value={effectivePrices[key] ?? "0"}
                        onChange={(event) =>
                          setPrices({ ...(prices ?? effectivePrices), [key]: event.target.value })
                        }
                      />
                    </div>
                  ))
                : null}
            </div>
            <Button
              variant="primary"
              size="sm"
              disabled={!prices}
              onClick={async () => {
                try {
                  await api.put("/admin/settings", { prices: prices ?? {} });
                  toast.success(t("common.save"));
                  setPrices(null);
                  void reloadSettings();
                } catch (error) {
                  toast.error(String(error));
                }
              }}
            >
              {t("common.save")}
            </Button>
          </CardContent>
        </Card>
      ) : null}

      {tab === "templates" ? (
        <p className="text-sm text-muted-foreground">{t("templates.description")}</p>
      ) : null}

      {tab === "runs" ? (
        <div className="space-y-2">
          {(runs?.runs ?? []).length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("admin.runsEmpty")}</p>
          ) : (
            (runs?.runs ?? []).map((run) => (
              <div
                key={run.id}
                className="flex flex-wrap items-center gap-3 rounded-lg border bg-card px-4 py-3 text-sm"
              >
                <StatusDot status={run.status} />
                <span className="font-mono text-xs text-muted-foreground">{run.id.slice(0, 8)}</span>
                <span>{t(`status.${run.status}`)}</span>
                <span className="font-mono text-xs text-muted-foreground">{(run.user_id ?? "").slice(0, 8)}</span>
                <span className="ml-auto text-xs text-muted-foreground">{formatTime(run.created_at)}</span>
                {["queued", "running"].includes(run.status) ? (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={async () => {
                      try {
                        await api.post(`/admin/runs/${run.id}/cancel`);
                        void reloadRuns();
                      } catch (error) {
                        toast.error(String(error));
                      }
                    }}
                  >
                    {t("runs.cancelRun")}
                  </Button>
                ) : null}
              </div>
            ))
          )}
        </div>
      ) : null}

      <Dialog open={!!draft} onOpenChange={(open) => !open && setDraft(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{draft?.id ? t("admin.editProvider") : t("admin.addProvider")}</DialogTitle>
          </DialogHeader>
          {draft ? (
            <div className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label>{t("admin.providerKind")}</Label>
                  <Select
                    value={draft.kind}
                    onChange={(event) => setDraft({ ...draft, kind: event.target.value })}
                  >
                    {PROVIDER_KINDS.map((kind) => (
                      <option key={kind}>{kind}</option>
                    ))}
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label>{t("admin.providerUpstream")}</Label>
                  <Select
                    value={draft.upstream_kind}
                    onChange={(event) => setDraft({ ...draft, upstream_kind: event.target.value })}
                  >
                    {UPSTREAM_KINDS.map((kind) => (
                      <option key={kind}>{kind}</option>
                    ))}
                  </Select>
                </div>
              </div>
              <div className="space-y-2">
                <Label>{t("admin.providerName")}</Label>
                <Input
                  value={draft.name}
                  onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                />
              </div>
              <div className="space-y-2">
                <Label>{t("admin.baseUrl")}</Label>
                <Input
                  value={draft.base_url}
                  placeholder="https://api.example.com"
                  onChange={(event) => setDraft({ ...draft, base_url: event.target.value })}
                />
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label>{t("admin.apiKey")}</Label>
                  <Input
                    type="password"
                    value={draft.api_key}
                    placeholder={draft.id ? t("admin.apiKeyKeep") : "sk-…"}
                    onChange={(event) => setDraft({ ...draft, api_key: event.target.value })}
                  />
                </div>
                <div className="space-y-2">
                  <Label>{t("admin.model")}</Label>
                  <Input
                    value={draft.model}
                    onChange={(event) => setDraft({ ...draft, model: event.target.value })}
                  />
                </div>
              </div>
              <div className="space-y-2">
                <Label>params JSON</Label>
                <textarea
                  className="h-28 w-full rounded-md border border-input bg-transparent p-2 font-mono text-xs focus-visible:outline-none"
                  value={draft.params}
                  onChange={(event) => setDraft({ ...draft, params: event.target.value })}
                />
              </div>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <Label>{t("admin.weight")}</Label>
                  <Input
                    type="number"
                    className="w-24"
                    value={draft.weight}
                    onChange={(event) => setDraft({ ...draft, weight: Number(event.target.value) })}
                  />
                </div>
                <div className="flex items-center gap-2">
                  <Label>{t("manager.enabled")}</Label>
                  <Switch
                    checked={draft.enabled}
                    onCheckedChange={(checked) => setDraft({ ...draft, enabled: checked })}
                  />
                </div>
              </div>
            </div>
          ) : null}
          <DialogFooter>
            <Button variant="outline" onClick={() => setDraft(null)}>
              {t("common.cancel")}
            </Button>
            <Button variant="primary" onClick={saveProvider}>
              {t("common.confirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  </div>
  );
}

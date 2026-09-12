import { useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import useSWR from "swr";
import { Copy, Loader2, Plus, Settings2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { api, type OrgDetail, type OrgKeyEntry, type OrgShareMode } from "@/lib/api";
import { useAuth } from "@/hooks/use-auth";
import { useMarketplaceModels } from "@/lib/swr";

const MODE_LABELS: Record<string, { key: string; tone: "default" | "secondary" | "outline" }> = {
  public: { key: "org.permPublic", tone: "default" },
  private: { key: "org.permPrivate", tone: "outline" },
  allow: { key: "org.permAllow", tone: "secondary" },
  deny: { key: "org.permDeny", tone: "secondary" },
};

function ModeBadge({ mode }: { mode?: string | null }) {
  const { t } = useTranslation();
  const label = MODE_LABELS[mode ?? "private"] ?? MODE_LABELS.private;
  return <Badge variant={label.tone}>{t(label.key)}</Badge>;
}

export function OrgKeys() {
  const { orgId } = useParams();
  const { t } = useTranslation();
  const detail = useSWR<OrgDetail>(orgId ? `/api/dashboard/orgs/${orgId}` : null, () =>
    api.getOrgDetail(orgId!),
  );
  const keys = useSWR(orgId ? `/api/dashboard/orgs/${orgId}/keys` : null, () =>
    api.listOrgKeys(orgId!),
  );

  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [mode, setMode] = useState<OrgShareMode>("private");
  const [modelQuery, setModelQuery] = useState("");
  const [selectedModels, setSelectedModels] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  const [shareTarget, setShareTarget] = useState<OrgKeyEntry | null>(null);
  const [shareMode, setShareMode] = useState<OrgShareMode>("private");
  const [shareMembers, setShareMembers] = useState<string[]>([]);

  const { user } = useAuth();
  const isOwner = detail.data?.my_role === "owner";
  const defaultMode: OrgShareMode = isOwner ? "public" : "private";
  const { data: modelRecords } = useMarketplaceModels();
  const modelOptions = useMemo(
    () => (modelRecords ?? []).map((record) => record.model_id),
    [modelRecords],
  );
  const filteredModels = useMemo(
    () =>
      modelQuery.trim()
        ? modelOptions
            .filter((model) => model.toLowerCase().includes(modelQuery.trim().toLowerCase()))
            .slice(0, 12)
        : modelOptions.slice(0, 12),
    [modelOptions, modelQuery],
  );

  const openCreate = () => {
    setName("");
    setMode(defaultMode);
    setModelQuery("");
    setSelectedModels([]);
    setCreateOpen(true);
  };

  return (
    <div className="mx-auto max-w-5xl space-y-6 p-6">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">{t("org.navKeys")}</h1>
        <Button onClick={openCreate}>
          <Plus className="h-4 w-4 mr-2" />
          {t("org.createKey")}
        </Button>
      </div>
      <p className="text-sm text-muted-foreground">{t("org.keysPageHint")}</p>

      {keys.data && keys.data.mine.length > 0 && (
        <section className="space-y-2">
          <h2 className="text-sm font-semibold text-muted-foreground">{t("org.myKeys")}</h2>
          {keys.data.mine.map((key) => (
            <Card key={key.id} className="rounded-2xl">
              <CardContent className="flex flex-wrap items-center justify-between gap-3 p-4">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <p className="truncate font-medium">{key.name}</p>
                    <ModeBadge mode={key.share_mode} />
                    {key.model_limits_enabled && (
                      <Badge variant="outline">
                        {t("org.modelLimits", { count: key.model_limits?.length ?? 0 })}
                      </Badge>
                    )}
                  </div>
                  <button
                    type="button"
                    className="mt-1 block max-w-full truncate font-mono text-xs text-muted-foreground hover:text-foreground"
                    title={t("org.keyCopied")}
                    onClick={() => {
                      void navigator.clipboard.writeText(key.key ?? "");
                      toast.success(t("org.keyCopied"));
                    }}
                  >
                    {key.key ?? key.key_prefix}
                  </button>
                </div>
                <div className="flex gap-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      void navigator.clipboard.writeText(key.key ?? "");
                      toast.success(t("org.keyCopied"));
                    }}
                  >
                    <Copy className="h-3.5 w-3.5" />
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      setShareTarget(key);
                      setShareMode((key.share_mode as OrgShareMode) ?? "private");
                      setShareMembers([]);
                    }}
                  >
                    <Settings2 className="mr-1 h-3.5 w-3.5" />
                    {t("org.permTitle")}
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </section>
      )}

      {keys.data && keys.data.shared.length > 0 && (
        <section className="space-y-2">
          <h2 className="text-sm font-semibold text-muted-foreground">{t("org.sharedToMe")}</h2>
          {keys.data.shared.map((key) => (
            <Card key={key.id} className="rounded-2xl">
              <CardContent className="flex flex-wrap items-center justify-between gap-3 p-4">
                <div className="min-w-0">
                  <p className="truncate font-medium">
                    {key.name}
                    <span className="ml-2 text-xs text-muted-foreground">@{key.owner_username}</span>
                  </p>
                  <button
                    type="button"
                    className="mt-1 block max-w-full truncate font-mono text-xs text-muted-foreground hover:text-foreground"
                    onClick={() => {
                      void navigator.clipboard.writeText(key.key ?? "");
                      toast.success(t("org.keyCopied"));
                    }}
                  >
                    {key.key ?? key.key_prefix}
                  </button>
                </div>
                <div className="flex items-center gap-2">
                  <ModeBadge mode={key.share_mode} />
                  {key.model_limits_enabled && (
                    <Badge variant="outline">
                      {t("org.modelLimits", { count: key.model_limits?.length ?? 0 })}
                    </Badge>
                  )}
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      void navigator.clipboard.writeText(key.key ?? "");
                      toast.success(t("org.keyCopied"));
                    }}
                  >
                    <Copy className="h-3.5 w-3.5" />
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </section>
      )}

      {keys.data && keys.data.mine.length === 0 && keys.data.shared.length === 0 && (
        <p className="py-10 text-center text-sm text-muted-foreground">{t("org.noKeys")}</p>
      )}

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("org.createKeyTitle")}</DialogTitle>
            <DialogDescription>{t("org.createKeyDescription")}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="org-key-name">{t("org.keyName")}</Label>
              <Input id="org-key-name" value={name} onChange={(event) => setName(event.target.value)} />
            </div>
            <div className="grid gap-2">
              <Label>{t("org.permTitle")}</Label>
              <div className="grid grid-cols-2 gap-2">
                {(
                  [
                    ["private", "org.permPrivate"],
                    ["public", "org.permPublic"],
                    ["allow", "org.permAllow"],
                    ["deny", "org.permDeny"],
                  ] as const
                ).map(([value, label]) => (
                  <Button
                    key={value}
                    type="button"
                    size="sm"
                    variant={mode === value ? "default" : "outline"}
                    onClick={() => setMode(value)}
                  >
                    {t(label)}
                  </Button>
                ))}
              </div>
            </div>
            <div className="grid gap-2">
              <Label>{t("org.modelLimitLabel")}</Label>
              <Input
                placeholder={t("org.modelLimitSearch")}
                value={modelQuery}
                onChange={(event) => setModelQuery(event.target.value)}
              />
              <div className="flex flex-wrap gap-1.5">
                {filteredModels.map((model) => (
                  <button
                    key={model}
                    type="button"
                    onClick={() =>
                      setSelectedModels((previous) =>
                        previous.includes(model)
                          ? previous.filter((entry) => entry !== model)
                          : [...previous, model],
                      )
                    }
                    className={`rounded-full border px-2.5 py-1 font-mono text-xs transition-colors ${
                      selectedModels.includes(model)
                        ? "border-primary bg-accent"
                        : "text-muted-foreground hover:bg-accent/50"
                    }`}
                  >
                    {model}
                  </button>
                ))}
              </div>
              {selectedModels.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  {t("org.modelLimitSelected", { count: selectedModels.length })}
                </p>
              )}
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setCreateOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              disabled={busy || !name.trim()}
              onClick={async () => {
                if (!orgId) return;
                setBusy(true);
                try {
                  await api.createOrgKey(orgId, name.trim(), mode, selectedModels);
                  toast.success(t("org.keyCreated"));
                  setCreateOpen(false);
                  await keys.mutate();
                } catch (error) {
                  toast.error(error instanceof Error ? error.message : t("common.error"));
                } finally {
                  setBusy(false);
                }
              }}
            >
              {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("org.createKey")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!shareTarget} onOpenChange={(open) => !open && setShareTarget(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("org.sharingTitle", { name: shareTarget?.name ?? "" })}</DialogTitle>
            <DialogDescription>{t("org.sharingDescription")}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-3">
            <div className="grid grid-cols-2 gap-2">
              {(
                [
                  ["private", "org.permPrivate"],
                  ["public", "org.permPublic"],
                  ["allow", "org.permAllow"],
                  ["deny", "org.permDeny"],
                ] as const
              ).map(([value, label]) => (
                <Button
                  key={value}
                  type="button"
                  size="sm"
                  variant={shareMode === value ? "default" : "outline"}
                  onClick={() => setShareMode(value)}
                >
                  {t(label)}
                </Button>
              ))}
            </div>
            {(shareMode === "allow" || shareMode === "deny") && (
              <div className="grid gap-2">
                {(detail.data?.members ?? [])
                  .filter((member) => member.user_id !== user?.id)
                  .map((member) => (
                    <label key={member.user_id} className="flex items-center gap-2 text-sm">
                      <Checkbox
                        checked={shareMembers.includes(member.user_id)}
                        onCheckedChange={(checked) =>
                          setShareMembers((previous) =>
                            checked
                              ? [...previous, member.user_id]
                              : previous.filter((id) => id !== member.user_id),
                          )
                        }
                      />
                      {member.username}
                    </label>
                  ))}
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShareTarget(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              disabled={busy}
              onClick={async () => {
                if (!orgId || !shareTarget) return;
                setBusy(true);
                try {
                  await api.updateOrgKeySharing(orgId, shareTarget.id, shareMode, shareMembers);
                  toast.success(t("org.sharingUpdated"));
                  setShareTarget(null);
                  await keys.mutate();
                } catch (error) {
                  toast.error(error instanceof Error ? error.message : t("common.error"));
                } finally {
                  setBusy(false);
                }
              }}
            >
              {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}


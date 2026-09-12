import { useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import useSWR from "swr";
import {
  Copy,
  Globe,
  Loader2,
  Lock,
  Plus,
  Settings2,
  UserCheck,
  UserX,
} from "lucide-react";
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
import { Switch } from "@/components/ui/switch";
import { PageWrapper } from "@/components/ui/motion";
import { api, type OrgDetail, type OrgKeyEntry, type OrgShareMode } from "@/lib/api";
import { useAuth } from "@/hooks/use-auth";
import { cn } from "@/lib/utils";

const MODES = [
  { value: "private", icon: Lock },
  { value: "public", icon: Globe },
  { value: "allow", icon: UserCheck },
  { value: "deny", icon: UserX },
] as const;

function modeLabelKey(mode: string, count?: number): string {
  switch (mode) {
    case "public":
      return "org.permPublic";
    case "allow":
      return count === undefined ? "org.permAllow" : "org.permAllowCount";
    case "deny":
      return count === undefined ? "org.permDeny" : "org.permDenyCount";
    default:
      return "org.permPrivate";
  }
}

/** The sharing state of one key: a subtle badge with an icon and, for the
 *  list modes, the number of members on the list. */
function ShareModeBadge({ mode, count }: { mode?: string | null; count?: number }) {
  const { t } = useTranslation();
  const effective = mode ?? "private";
  const Icon = MODES.find((entry) => entry.value === effective)?.icon ?? Lock;
  const usesCount = (effective === "allow" || effective === "deny") && count !== undefined;
  return (
    <Badge variant="outline" className="gap-1 font-normal text-muted-foreground">
      <Icon className="size-3" />
      {t(modeLabelKey(effective, count), usesCount ? { count } : undefined)}
    </Badge>
  );
}

/** Radio-card picker for the four sharing modes; `withDescriptions` adds the
 *  one-line explanation under each label. */
function ModeSelector({
  value,
  onChange,
  withDescriptions = false,
}: {
  value: OrgShareMode;
  onChange: (mode: OrgShareMode) => void;
  withDescriptions?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="grid gap-1.5">
      {MODES.map(({ value: mode, icon: Icon }) => (
        <button
          key={mode}
          type="button"
          aria-pressed={value === mode}
          onClick={() => onChange(mode)}
          className={cn(
            "flex items-start gap-3 rounded-lg border px-3 py-2.5 text-left transition-colors",
            value === mode
              ? "border-primary bg-accent/60"
              : "hover:bg-accent/40",
          )}
        >
          <Icon className={cn("mt-0.5 size-4 shrink-0", value === mode ? "text-primary" : "text-muted-foreground")} />
          <span className="min-w-0">
            <span className={cn("block text-sm font-medium", value === mode ? "text-foreground" : "text-muted-foreground")}>
              {t(modeLabelKey(mode))}
            </span>
            {withDescriptions && (
              <span className="mt-0.5 block text-xs text-muted-foreground">
                {t(`org.permDesc.${mode}`)}
              </span>
            )}
          </span>
          <span
            className={cn(
              "ml-auto mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border",
              value === mode ? "border-primary" : "border-muted-foreground/40",
            )}
          >
            {value === mode && <span className="size-2 rounded-full bg-primary" />}
          </span>
        </button>
      ))}
    </div>
  );
}

function MemberPicker({
  members,
  selected,
  onChange,
  excludeUserId,
}: {
  members: OrgDetail["members"];
  selected: string[];
  onChange: (memberIds: string[]) => void;
  excludeUserId?: string;
}) {
  const { t } = useTranslation();
  const candidates = members.filter((member) => member.user_id !== excludeUserId);
  if (candidates.length === 0) {
    return <p className="text-sm text-muted-foreground">{t("org.noOtherMembers")}</p>;
  }
  return (
    <div className="grid gap-1">
      {candidates.map((member) => (
        <label
          key={member.user_id}
          className="flex cursor-pointer items-center gap-2.5 rounded-md border px-3 py-2 text-sm transition-colors hover:bg-accent/40"
        >
          <Checkbox
            checked={selected.includes(member.user_id)}
            onCheckedChange={(checked) =>
              onChange(
                checked
                  ? [...selected, member.user_id]
                  : selected.filter((id) => id !== member.user_id),
              )
            }
          />
          <span className="flex-1">{member.username}</span>
          <span className="text-xs text-muted-foreground">
            {member.role === "owner" ? t("org.owner") : t("org.member")}
          </span>
        </label>
      ))}
    </div>
  );
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
  const [modelsEnabled, setModelsEnabled] = useState(false);
  const [modelsInput, setModelsInput] = useState("");
  const [busy, setBusy] = useState(false);

  const [shareTarget, setShareTarget] = useState<OrgKeyEntry | null>(null);
  const [shareMode, setShareMode] = useState<OrgShareMode>("private");
  const [shareMembers, setShareMembers] = useState<string[]>([]);

  const { user } = useAuth();
  const isOwner = detail.data?.my_role === "owner";
  const defaultMode: OrgShareMode = isOwner ? "public" : "private";
  const modelsList = useMemo(
    () => modelsInput.split(",").map((model) => model.trim()).filter(Boolean),
    [modelsInput],
  );

  const openCreate = () => {
    setName("");
    setMode(defaultMode);
    setModelsEnabled(false);
    setModelsInput("");
    setCreateOpen(true);
  };

  const openSharing = (key: OrgKeyEntry) => {
    setShareTarget(key);
    setShareMode((key.share_mode as OrgShareMode) ?? "private");
    // ORG-15: the saved share rows come back with the key, so the editor opens
    // with the previous selection instead of a blank list.
    setShareMembers(key.shared_with ?? []);
  };

  return (
    <PageWrapper className="mx-auto max-w-5xl space-y-6 p-6">
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
                  <div className="flex flex-wrap items-center gap-2">
                    <p className="truncate font-medium">{key.name}</p>
                    <ShareModeBadge mode={key.share_mode} count={key.shared_with?.length} />
                    {key.model_limits_enabled && (
                      <Badge variant="outline" className="gap-1 font-normal text-muted-foreground">
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
                  <Button variant="outline" size="sm" onClick={() => openSharing(key)}>
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
                  <ShareModeBadge mode={key.share_mode} />
                  {key.model_limits_enabled && (
                    <Badge variant="outline" className="gap-1 font-normal text-muted-foreground">
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
              <div className="flex items-center space-x-2">
                <Switch
                  id="org-key-models"
                  checked={modelsEnabled}
                  onCheckedChange={setModelsEnabled}
                />
                <Label htmlFor="org-key-models">{t("apiKeys.enableModelLimits")}</Label>
              </div>
              {modelsEnabled && (
                <div className="space-y-2">
                  <Label htmlFor="org-key-model-list">{t("apiKeys.allowedModels")}</Label>
                  <Input
                    id="org-key-model-list"
                    value={modelsInput}
                    onChange={(event) => setModelsInput(event.target.value)}
                    placeholder="gpt-4, gpt-3.5-turbo"
                  />
                  <p className="text-sm text-muted-foreground">{t("apiKeys.modelsHelp")}</p>
                </div>
              )}
            </div>
            <div className="grid gap-2">
              <Label>{t("org.permTitle")}</Label>
              <ModeSelector value={mode} onChange={setMode} />
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
                  await api.createOrgKey(orgId, name.trim(), mode, modelsEnabled ? modelsList : []);
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
            <ModeSelector value={shareMode} onChange={setShareMode} withDescriptions />
            {(shareMode === "allow" || shareMode === "deny") && (
              <div className="grid gap-2">
                <Label>{t("org.shareListLabel")}</Label>
                <MemberPicker
                  members={detail.data?.members ?? []}
                  selected={shareMembers}
                  onChange={setShareMembers}
                  excludeUserId={user?.id}
                />
                {shareMode === "allow" && shareMembers.length === 0 && (
                  <p className="text-xs text-warning">{t("org.permNoneSelected")}</p>
                )}
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
    </PageWrapper>
  );
}

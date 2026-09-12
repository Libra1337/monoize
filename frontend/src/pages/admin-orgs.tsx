import { useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { Copy, Loader2, Pencil, RefreshCw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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
import { Skeleton } from "@/components/ui/skeleton";
import { PageWrapper, motion, transitions } from "@/components/ui/motion";
import { PageHeader } from "@/components/ui/page-header";
import { EmptyState } from "@/components/ui/empty-state";
import { api, type AdminOrgEntry } from "@/lib/api";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import { formatCoinFromNanoUsdForCurrency } from "@/lib/store-money";
import { OrgAvatar } from "./org/shared";

const ADMIN_ORGS_KEY = "/api/dashboard/admin/orgs";

/** ORG-28: the admin console over every organization space. */
export function AdminOrgsPage() {
  const { t } = useTranslation();
  const { currency } = useStoreCurrency();
  const { data: rate } = useStoreExchangeRate();
  const orgs = useSWR<AdminOrgEntry[]>(ADMIN_ORGS_KEY, () => api.adminListOrgs());
  const [editTarget, setEditTarget] = useState<AdminOrgEntry | null>(null);
  const [maxMembers, setMaxMembers] = useState("");
  const [ownerLimit, setOwnerLimit] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<AdminOrgEntry | null>(null);
  const [busy, setBusy] = useState(false);

  const money = (nano: string) =>
    currency === "CNY" && rate?.cny_per_usd
      ? formatCoinFromNanoUsdForCurrency(nano, "CNY", rate.cny_per_usd)
      : formatCoinFromNanoUsdForCurrency(nano, "USD", "1");

  const reload = async () => {
    await orgs.mutate();
  };

  return (
    <PageWrapper className="space-y-6">
      <motion.div
        initial={{ opacity: 0, y: -10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={transitions.normal}
      >
        <PageHeader
          title={t("adminOrgs.title")}
          description={t("adminOrgs.description")}
          actions={(
            <Button variant="outline" size="sm" onClick={() => void reload()}>
              <RefreshCw className="mr-1 h-3.5 w-3.5" />
              {t("common.refresh")}
            </Button>
          )}
        />
      </motion.div>

      {orgs.isLoading ? (
        <div className="space-y-2">
          {Array.from({ length: 4 }, (_, index) => (
            <Skeleton key={index} className="h-16 w-full rounded-2xl" />
          ))}
        </div>
      ) : (orgs.data?.length ?? 0) === 0 ? (
        <EmptyState title={t("adminOrgs.empty")} description={t("adminOrgs.emptyDescription")} />
      ) : (
        <div className="space-y-2">
          {orgs.data!.map((org) => (
            <motion.div
              key={org.id}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={transitions.normal}
            >
              <Card className="rounded-2xl">
                <CardContent className="flex flex-wrap items-center gap-x-6 gap-y-3 p-4">
                  <div className="flex min-w-56 items-center gap-3">
                    <OrgAvatar
                      emoji={org.avatar_emoji}
                      color={org.avatar_color}
                      image={org.avatar_image}
                      size="size-9"
                      text="text-lg"
                    />
                    <div className="min-w-0">
                      <p className="truncate font-medium">{org.display_name}</p>
                      <p className="truncate text-xs text-muted-foreground">
                        {t("adminOrgs.owner")}: {org.owner_username}
                      </p>
                    </div>
                  </div>
                  <div className="min-w-24">
                    <p className="text-xs text-muted-foreground">{t("adminOrgs.members")}</p>
                    <p className="font-semibold tabular-nums">
                      {org.member_count} / {org.max_members}
                      {org.member_count >= org.max_members && (
                        <Badge variant="outline" className="ml-2 text-warning">{t("org.fullBadge")}</Badge>
                      )}
                    </p>
                  </div>
                  <div className="min-w-24">
                    <p className="text-xs text-muted-foreground">{t("org.walletBalance")}</p>
                    <p className="font-semibold tabular-nums">{money(org.balance_nano_usd)}</p>
                  </div>
                  <div className="min-w-36">
                    <p className="text-xs text-muted-foreground">{t("org.inviteCodeLabel")}</p>
                    <button
                      type="button"
                      className="flex items-center gap-1 font-mono text-sm tracking-[0.2em] hover:text-foreground"
                      onClick={() => {
                        void navigator.clipboard.writeText(org.invite_code);
                        toast.success(t("org.codeCopied"));
                      }}
                    >
                      {org.invite_code || "—"}
                      <Copy className="size-3" />
                    </button>
                  </div>
                  <div className="min-w-32">
                    <p className="text-xs text-muted-foreground">{t("adminOrgs.createdAt")}</p>
                    <p className="text-sm">{new Date(org.created_at).toLocaleDateString()}</p>
                  </div>
                  <div className="ml-auto flex gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => {
                        setEditTarget(org);
                        setMaxMembers(String(org.max_members));
                        setOwnerLimit(
                          org.owner_org_creation_limit == null
                            ? ""
                            : String(org.owner_org_creation_limit),
                        );
                      }}
                    >
                      <Pencil className="mr-1 h-3.5 w-3.5" />
                      {t("adminOrgs.limits")}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-destructive hover:text-destructive"
                      onClick={() => setDeleteTarget(org)}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                </CardContent>
              </Card>
            </motion.div>
          ))}
        </div>
      )}

      <Dialog open={!!editTarget} onOpenChange={(open) => !open && setEditTarget(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("adminOrgs.limitsTitle", { name: editTarget?.display_name ?? "" })}</DialogTitle>
            <DialogDescription>{t("adminOrgs.limitsDescription")}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="admin-org-max-members">
                {t("adminOrgs.maxMembers", { current: editTarget?.max_members ?? 15 })}
              </Label>
              <Input
                id="admin-org-max-members"
                type="number"
                min="1"
                max="1000"
                value={maxMembers}
                onChange={(event) => setMaxMembers(event.target.value)}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="admin-org-owner-limit">
                {t("adminOrgs.ownerLimit", { owner: editTarget?.owner_username ?? "" })}
              </Label>
              <Input
                id="admin-org-owner-limit"
                type="number"
                min="0"
                max="100"
                value={ownerLimit}
                onChange={(event) => setOwnerLimit(event.target.value)}
                placeholder={t("adminOrgs.ownerLimitPlaceholder")}
              />
              <p className="text-xs text-muted-foreground">{t("adminOrgs.ownerLimitHelp")}</p>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditTarget(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              disabled={busy}
              onClick={async () => {
                if (!editTarget) return;
                setBusy(true);
                try {
                  await api.adminUpdateOrg(editTarget.id, {
                    max_members: maxMembers ? parseInt(maxMembers, 10) : undefined,
                    owner_org_creation_limit: ownerLimit === ""
                      ? undefined
                      : parseInt(ownerLimit, 10),
                  });
                  toast.success(t("adminOrgs.limitsSaved"));
                  setEditTarget(null);
                  await reload();
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

      <Dialog open={!!deleteTarget} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("org.deleteConfirmTitle", { name: deleteTarget?.display_name ?? "" })}</DialogTitle>
            <DialogDescription>{t("org.deleteConfirmDescription")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              disabled={busy}
              onClick={async () => {
                if (!deleteTarget) return;
                setBusy(true);
                try {
                  await api.deleteOrg(deleteTarget.id);
                  toast.success(t("org.deleted"));
                  setDeleteTarget(null);
                  await reload();
                } catch (error) {
                  toast.error(error instanceof Error ? error.message : t("common.error"));
                } finally {
                  setBusy(false);
                }
              }}
            >
              {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("org.deleteOrg")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </PageWrapper>
  );
}

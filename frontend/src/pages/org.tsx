import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { Building2, Copy, KeyRound, Link2, Loader2, Plus, RefreshCw, UsersRound } from "lucide-react";
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
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper, motion, transitions } from "@/components/ui/motion";
import { useAuth } from "@/hooks/use-auth";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import {
  api,
  type OrgDetail,
  type OrgKeyEntry,
  type OrgMember,
  type OrgSummary,
  type OrgInviteExpiry,
} from "@/lib/api";
import { formatCoinFromNanoUsdForCurrency } from "@/lib/store-money";

const ORGS_KEY = "/api/dashboard/orgs";
const INVITE_EXPIRIES: OrgInviteExpiry[] = ["24h", "3d", "7d", "30d", "never"];
const EMOJI_CHOICES = ["🏢", "🚀", "⚡", "🧠", "🛠️", "📊", "🎯", "🔮", "🌿", "🐙", "🦾", "💼"];
const COLOR_CHOICES = ["#6366f1", "#0ea5e9", "#10b981", "#f59e0b", "#ef4444", "#8b5cf6"];

function useMyOrgs() {
  return useSWR<OrgSummary[]>(ORGS_KEY, () => api.listMyOrgs(), { fallbackData: [] });
}

function useMoney() {
  const { currency } = useStoreCurrency();
  const { data: rate } = useStoreExchangeRate();
  return (nano: string) =>
    currency === "CNY" && rate?.cny_per_usd
      ? formatCoinFromNanoUsdForCurrency(nano, "CNY", rate.cny_per_usd)
      : formatCoinFromNanoUsdForCurrency(nano, "USD", "1");
}

/** Converts a display-currency input into nano-USD, or null when invalid. */
function amountToNanoUsd(raw: string, currency: "CNY" | "USD", cnyPerUsd?: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed || !/^\\d+(\\.\\d{1,2})?$/.test(trimmed)) return null;
  const minor = Math.round(parseFloat(trimmed) * 100);
  if (!Number.isSafeInteger(minor) || minor <= 0) return null;
  if (currency === "USD") return (BigInt(minor) * 10_000_000n / 100n).toString();
  if (!cnyPerUsd) return null;
  const [whole, frac = ""] = cnyPerUsd.split(".");
  const numerator = BigInt(whole + frac.padEnd(6, "0").slice(0, 6));
  if (numerator <= 0n) return null;
  const scaled = (BigInt(minor) * 1_000_000n * 10_000_000n + numerator / 2n) / numerator;
  return scaled.toString();
}

function OrgAvatar({ emoji, color, size = "size-14" }: { emoji: string; color: string; size?: string }) {
  return (
    <div
      className={`${size} flex shrink-0 items-center justify-center rounded-2xl text-2xl`}
      style={{ backgroundColor: `${color}22`, color }}
    >
      {emoji}
    </div>
  );
}

export function OrgPage() {
  const { t, i18n } = useTranslation();
  const zh = i18n.language.startsWith("zh");
  const c = (zhText: string, enText: string) => (zh ? zhText : enText);
  const { user } = useAuth();
  const { data: orgs, isLoading, mutate: reloadOrgs } = useMyOrgs();
  const money = useMoney();
  const { currency } = useStoreCurrency();
  const { data: rate } = useStoreExchangeRate();

  const [activeOrgId, setActiveOrgId] = useState<string>("");
  const currentOrg = useMemo(
    () => orgs?.find((org) => org.id === activeOrgId) ?? orgs?.[0],
    [orgs, activeOrgId],
  );
  const detail = useSWR<OrgDetail>(
    currentOrg ? `/api/dashboard/orgs/${currentOrg.id}` : null,
    () => api.getOrgDetail(currentOrg!.id),
  );
  const keys = useSWR(
    currentOrg ? `/api/dashboard/orgs/${currentOrg.id}/keys` : null,
    () => api.listOrgKeys(currentOrg!.id),
  );
  const isOwner = detail.data?.my_role === "owner";

  const [createOpen, setCreateOpen] = useState(false);
  const [joinInput, setJoinInput] = useState("");
  const [depositOpen, setDepositOpen] = useState(false);
  const [depositAmount, setDepositAmount] = useState("");
  const [distributeTarget, setDistributeTarget] = useState<OrgMember | null>(null);
  const [distributeAmount, setDistributeAmount] = useState("");
  const [shareTarget, setShareTarget] = useState<OrgKeyEntry | null>(null);
  const [shareMode, setShareMode] = useState<"private" | "all" | "selected">("private");
  const [shareMembers, setShareMembers] = useState<string[]>([]);
  const [createKeyOpen, setCreateKeyOpen] = useState(false);
  const [newKeyName, setNewKeyName] = useState("");
  const [newKeyShare, setNewKeyShare] = useState<"default" | "private" | "all">("default");
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const canCreate =
    user?.account_class === "enterprise" && !user?.parent_user_id && !user?.is_sales_agent;

  const handleCreate = async (input: { name: string; emoji: string; color: string; expiry: OrgInviteExpiry; confirmed: boolean }) => {
    if (!input.confirmed || busy) return;
    setBusy(true);
    try {
      await api.createOrg({
        display_name: input.name.trim(),
        avatar_emoji: input.emoji,
        avatar_color: input.color,
        invite_expiry: input.expiry,
      });
      toast.success(c("组织已创建", "Organization created"));
      setCreateOpen(false);
      await reloadOrgs();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  const handleJoinLink = () => {
    const raw = joinInput.trim();
    const token = raw.split("/join/")[1]?.split(/[?#]/)[0] ?? raw;
    if (!token) return;
    window.location.href = `/join/${encodeURIComponent(token)}`;
  };

  const move = async (action: "deposit" | "distribute", amount: string) => {
    const nano = amountToNanoUsd(amount, currency, rate?.cny_per_usd);
    if (!nano || !currentOrg) {
      toast.error(c("请输入最多两位小数的正数金额", "Enter a positive amount with at most two decimals"));
      return;
    }
    setBusy(true);
    try {
      if (action === "deposit") {
        await api.depositToOrg(currentOrg.id, nano);
        toast.success(c("已充值到组织钱包", "Deposited into the organization wallet"));
        setDepositOpen(false);
        setDepositAmount("");
      } else if (distributeTarget) {
        await api.distributeFromOrg(currentOrg.id, distributeTarget.user_id, nano);
        toast.success(c("已分发余额", "Balance distributed"));
        setDistributeTarget(null);
        setDistributeAmount("");
      }
      await Promise.all([reloadOrgs(), detail.mutate()]);
    } catch (error) {
      toast.error(error instanceof Error ? error.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  const inviteLink = detail.data?.invite?.token
    ? `${window.location.origin}/join/${detail.data.invite.token}`
    : "";

  if (isLoading) {
    return (
      <PageWrapper className="space-y-6">
        <Skeleton className="h-24 w-full rounded-2xl" />
        <Skeleton className="h-64 w-full rounded-2xl" />
      </PageWrapper>
    );
  }

  return (
    <PageWrapper className="space-y-6">
      <motion.div initial={{ opacity: 0, y: -10 }} animate={{ opacity: 1, y: 0 }} transition={transitions.normal}>
        <PageHeader
          title={t("org.title")}
          description={t("org.description")}
          actions={canCreate ? (
            <Button onClick={() => setCreateOpen(true)}>
              <Plus className="h-4 w-4 mr-2" />
              {t("org.create")}
            </Button>
          ) : undefined}
        />
      </motion.div>

      {orgs && orgs.length === 0 ? (
        <EmptyState
          variant="card"
          icon={<Building2 className="h-12 w-12" />}
          title={t("org.empty")}
          description={canCreate ? t("org.emptyCreateHint") : t("org.emptyJoinHint")}
          action={
            <div className="flex flex-col gap-3 sm:flex-row">
              {canCreate && (
                <Button onClick={() => setCreateOpen(true)}>
                  <Plus className="h-4 w-4 mr-2" />
                  {t("org.create")}
                </Button>
              )}
              <div className="flex gap-2">
                <Input
                  className="w-64"
                  placeholder={t("org.joinPlaceholder")}
                  value={joinInput}
                  onChange={(event) => setJoinInput(event.target.value)}
                  onKeyDown={(event) => event.key === "Enter" && handleJoinLink()}
                />
                <Button variant="outline" onClick={handleJoinLink} disabled={!joinInput.trim()}>
                  {t("org.join")}
                </Button>
              </div>
            </div>
          }
        />
      ) : (
        <>
          {(orgs?.length ?? 0) > 1 && (
            <div className="flex flex-wrap gap-2">
              {orgs?.map((org) => (
                <button
                  key={org.id}
                  type="button"
                  onClick={() => setActiveOrgId(org.id)}
                  className={`flex items-center gap-2 rounded-xl border px-3 py-2 text-sm transition-colors ${
                    org.id === currentOrg?.id ? "border-primary bg-accent" : "hover:bg-accent/50"
                  }`}
                >
                  <OrgAvatar emoji={org.avatar_emoji} color={org.avatar_color} size="size-8 text-base" />
                  <span className="font-medium">{org.display_name}</span>
                  {org.role === "owner" && <Badge variant="outline">{t("org.owner")}</Badge>}
                </button>
              ))}
            </div>
          )}

          {detail.data && (
            <>
              <Card className="rounded-2xl">
                <CardContent className="flex flex-wrap items-center justify-between gap-4 p-5">
                  <div className="flex items-center gap-4">
                    <OrgAvatar emoji={detail.data.avatar_emoji} color={detail.data.avatar_color} />
                    <div>
                      <div className="flex items-center gap-2">
                        <p className="text-lg font-semibold">{detail.data.display_name}</p>
                        <Badge variant={isOwner ? "default" : "secondary"}>
                          {isOwner ? t("org.owner") : t("org.member")}
                        </Badge>
                      </div>
                      <p className="mt-1 text-sm text-muted-foreground">
                        {t("org.walletBalance")} ·{" "}
                        <span className="font-semibold tabular-nums">{money(detail.data.balance_nano_usd)}</span>
                        {" · "}
                        {t("org.members", { count: detail.data.members.length })}
                      </p>
                    </div>
                  </div>
                  {isOwner && (
                    <div className="flex gap-2">
                      <Button variant="outline" onClick={() => setDepositOpen(true)}>
                        {t("org.deposit")}
                      </Button>
                      <Button
                        variant="outline"
                        disabled={detail.data.members.length <= 1}
                        onClick={() => {
                          const member = detail.data!.members.find((m) => m.user_id !== user?.id);
                          if (member) setDistributeTarget(member);
                        }}
                      >
                        {t("org.distribute")}
                      </Button>
                    </div>
                  )}
                </CardContent>
              </Card>

              {isOwner && detail.data.invite && (
                <Card className="rounded-2xl">
                  <CardContent className="flex flex-wrap items-center justify-between gap-3 p-5">
                    <div className="flex min-w-0 items-center gap-3">
                      <Link2 className="h-4 w-4 shrink-0 text-muted-foreground" />
                      <span className="truncate font-mono text-xs">{inviteLink}</span>
                      <Badge variant="outline">
                        {detail.data.invite.expires_at
                          ? t("org.inviteExpires", {
                              time: new Date(detail.data.invite.expires_at).toLocaleString(),
                            })
                          : t("org.inviteNeverExpires")}
                      </Badge>
                    </div>
                    <div className="flex gap-2">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          void navigator.clipboard.writeText(inviteLink);
                          toast.success(t("org.linkCopied"));
                        }}
                      >
                        <Copy className="mr-1 h-3.5 w-3.5" />
                        {t("org.copyLink")}
                      </Button>
                      <RegenerateInviteButton orgId={detail.data.id} onDone={() => detail.mutate()} />
                    </div>
                  </CardContent>
                </Card>
              )}

              <div className="grid gap-6 lg:grid-cols-2">
                <section className="rounded-2xl border">
                  <h3 className="border-b px-4 py-3 text-sm font-semibold">{t("org.membersTitle")}</h3>
                  <table className="w-full text-sm">
                    <tbody>
                      {detail.data.members.map((member) => (
                        <tr key={member.user_id} className="border-b last:border-b-0">
                          <td className="px-4 py-2.5 font-medium">{member.username}</td>
                          <td className="px-4 py-2.5">
                            <Badge variant={member.role === "owner" ? "default" : "secondary"}>
                              {member.role === "owner" ? t("org.owner") : t("org.member")}
                            </Badge>
                          </td>
                          <td className="px-4 py-2.5 text-right">
                            {isOwner && member.role !== "owner" && (
                              <Button
                                variant="ghost"
                                size="sm"
                                className="text-destructive"
                                onClick={async () => {
                                  try {
                                    await api.removeOrgMember(detail.data!.id, member.user_id);
                                    toast.success(t("org.memberRemoved"));
                                    await Promise.all([detail.mutate(), reloadOrgs()]);
                                  } catch (error) {
                                    toast.error(error instanceof Error ? error.message : t("common.error"));
                                  }
                                }}
                              >
                                {t("org.removeMember")}
                              </Button>
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </section>

                <section className="rounded-2xl border">
                  <h3 className="flex items-center justify-between border-b px-4 py-3 text-sm font-semibold">
                    <span className="flex items-center gap-2">
                      <KeyRound className="h-4 w-4" />
                      {t("org.keysTitle")}
                    </span>
                    <Button variant="outline" size="sm" onClick={() => { setCreateKeyOpen(true); setCreatedKey(null); setNewKeyName(""); }}>
                      <Plus className="mr-1 h-3.5 w-3.5" />
                      {t("org.createKey")}
                    </Button>
                  </h3>
                  <div className="divide-y">
                    {keys.data?.mine.map((key) => (
                      <div key={key.id} className="flex items-center justify-between gap-2 px-4 py-2.5 text-sm">
                        <div className="min-w-0">
                          <p className="truncate font-medium">{key.name}</p>
                          <p className="font-mono text-xs text-muted-foreground">{key.key_prefix}…</p>
                        </div>
                        <div className="flex shrink-0 items-center gap-2">
                          <Badge variant={key.share_mode === "all" ? "default" : "outline"}>
                            {key.share_mode === "all"
                              ? t("org.sharedAll")
                              : key.share_mode === "selected"
                                ? t("org.sharedSelected")
                                : t("org.private")}
                          </Badge>
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => {
                              setShareTarget(key);
                              setShareMode((key.share_mode as "private" | "all" | "selected") ?? "private");
                              setShareMembers([]);
                            }}
                          >
                            {t("org.sharing")}
                          </Button>
                        </div>
                      </div>
                    ))}
                    {keys.data?.shared.map((key) => (
                      <div key={key.id} className="flex items-center justify-between gap-2 px-4 py-2.5 text-sm">
                        <div className="min-w-0">
                          <p className="truncate font-medium">
                            {key.name}
                            <span className="ml-2 text-xs text-muted-foreground">@{key.owner_username}</span>
                          </p>
                          <p className="truncate font-mono text-xs text-muted-foreground">{key.key}</p>
                        </div>
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
                    ))}
                    {keys.data && keys.data.mine.length === 0 && keys.data.shared.length === 0 && (
                      <p className="px-4 py-6 text-center text-sm text-muted-foreground">
                        {t("org.noKeys")}
                      </p>
                    )}
                  </div>
                </section>
              </div>
            </>
          )}
        </>
      )}

      <CreateOrgDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        busy={busy}
        onSubmit={handleCreate}
      />

      <Dialog open={depositOpen} onOpenChange={setDepositOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("org.depositTitle")}</DialogTitle>
            <DialogDescription>{t("org.transferDescription", { currency })}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-2">
            <Label htmlFor="org-deposit">{t("org.amountLabel", { currency })}</Label>
            <Input
              id="org-deposit"
              inputMode="decimal"
              value={depositAmount}
              onChange={(event) => setDepositAmount(event.target.value)}
            />
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDepositOpen(false)}>{t("common.cancel")}</Button>
            <Button disabled={busy || !depositAmount.trim()} onClick={() => void move("deposit", depositAmount)}>
              {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("org.deposit")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!distributeTarget} onOpenChange={(open) => !open && setDistributeTarget(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("org.distributeTitle", { name: distributeTarget?.username ?? "" })}</DialogTitle>
            <DialogDescription>{t("org.transferDescription", { currency })}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-2">
            <Label htmlFor="org-distribute">{t("org.amountLabel", { currency })}</Label>
            <Input
              id="org-distribute"
              inputMode="decimal"
              value={distributeAmount}
              onChange={(event) => setDistributeAmount(event.target.value)}
            />
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDistributeTarget(null)}>{t("common.cancel")}</Button>
            <Button
              disabled={busy || !distributeAmount.trim()}
              onClick={() => void move("distribute", distributeAmount)}
            >
              {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("org.distribute")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={createKeyOpen} onOpenChange={setCreateKeyOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("org.createKeyTitle")}</DialogTitle>
            <DialogDescription>{t("org.createKeyDescription")}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="org-key-name">{t("org.keyName")}</Label>
              <Input id="org-key-name" value={newKeyName} onChange={(event) => setNewKeyName(event.target.value)} />
            </div>
            <div className="grid gap-2">
              <Label>{t("org.initialSharing")}</Label>
              <div className="flex gap-2">
                {(["default", "all", "private"] as const).map((mode) => (
                  <Button
                    key={mode}
                    type="button"
                    variant={newKeyShare === mode ? "default" : "outline"}
                    size="sm"
                    onClick={() => setNewKeyShare(mode)}
                  >
                    {mode === "default" ? t("org.defaultSharing") : mode === "all" ? t("org.sharedAll") : t("org.private")}
                  </Button>
                ))}
              </div>
            </div>
            {createdKey && (
              <div className="rounded-lg border bg-muted/40 p-3">
                <p className="text-xs text-muted-foreground">{t("org.keyOnce")}</p>
                <button
                  type="button"
                  className="mt-1 w-full break-all text-left font-mono text-xs"
                  onClick={() => {
                    void navigator.clipboard.writeText(createdKey);
                    toast.success(t("org.keyCopied"));
                  }}
                >
                  {createdKey}
                </button>
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setCreateKeyOpen(false)}>{t("common.close")}</Button>
            <Button
              disabled={busy || !!createdKey || !newKeyName.trim()}
              onClick={async () => {
                if (!currentOrg) return;
                setBusy(true);
                try {
                  const created = await api.createOrgKey(currentOrg.id, newKeyName.trim(), newKeyShare);
                  setCreatedKey(created.key);
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
            <div className="flex gap-2">
              {(["private", "all", "selected"] as const).map((mode) => (
                <Button
                  key={mode}
                  type="button"
                  variant={shareMode === mode ? "default" : "outline"}
                  size="sm"
                  onClick={() => setShareMode(mode)}
                >
                  {mode === "private" ? t("org.private") : mode === "all" ? t("org.sharedAll") : t("org.sharedSelected")}
                </Button>
              ))}
            </div>
            {shareMode === "selected" && (
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
            <Button variant="outline" onClick={() => setShareTarget(null)}>{t("common.cancel")}</Button>
            <Button
              disabled={busy}
              onClick={async () => {
                if (!currentOrg || !shareTarget) return;
                setBusy(true);
                try {
                  await api.updateOrgKeySharing(currentOrg.id, shareTarget.id, shareMode, shareMembers);
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

function RegenerateInviteButton({ orgId, onDone }: { orgId: string; onDone: () => void }) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [expiry, setExpiry] = useState<OrgInviteExpiry>("7d");
  return (
    <div className="flex items-center gap-1">
      <select
        className="h-8 rounded-md border bg-background px-2 text-xs"
        value={expiry}
        onChange={(event) => setExpiry(event.target.value as OrgInviteExpiry)}
        aria-label={t("org.inviteExpiry")}
      >
        {INVITE_EXPIRIES.map((value) => (
          <option key={value} value={value}>
            {value === "never" ? t("org.never") : value}
          </option>
        ))}
      </select>
      <Button
        variant="ghost"
        size="sm"
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          try {
            await api.regenerateOrgInvite(orgId, expiry);
            toast.success(t("org.linkRegenerated"));
            onDone();
          } catch (error) {
            toast.error(error instanceof Error ? error.message : t("common.error"));
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
      </Button>
    </div>
  );
}

function CreateOrgDialog({
  open,
  onOpenChange,
  busy,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  busy: boolean;
  onSubmit: (input: { name: string; emoji: string; color: string; expiry: OrgInviteExpiry; confirmed: boolean }) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [emoji, setEmoji] = useState(EMOJI_CHOICES[0]);
  const [color, setColor] = useState(COLOR_CHOICES[0]);
  const [expiry, setExpiry] = useState<OrgInviteExpiry>("7d");
  const [confirmed, setConfirmed] = useState(false);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <UsersRound className="h-5 w-5" />
            {t("org.createTitle")}
          </DialogTitle>
          <DialogDescription>{t("org.createDescription")}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-4">
          <div className="flex items-center gap-4">
            <OrgAvatar emoji={emoji} color={color} />
            <div className="flex-1 grid gap-2">
              <Label htmlFor="org-name">{t("org.nameLabel")}</Label>
              <Input id="org-name" value={name} onChange={(event) => setName(event.target.value)} placeholder={t("org.namePlaceholder")} />
            </div>
          </div>
          <div className="grid gap-2">
            <Label>{t("org.avatarLabel")}</Label>
            <div className="flex flex-wrap gap-1.5">
              {EMOJI_CHOICES.map((choice) => (
                <button
                  key={choice}
                  type="button"
                  onClick={() => setEmoji(choice)}
                  className={`flex size-9 items-center justify-center rounded-lg border text-lg transition-colors ${choice === emoji ? "border-primary bg-accent" : "hover:bg-accent/50"}`}
                >
                  {choice}
                </button>
              ))}
            </div>
          </div>
          <div className="grid gap-2">
            <Label>{t("org.colorLabel")}</Label>
            <div className="flex gap-2">
              {COLOR_CHOICES.map((choice) => (
                <button
                  key={choice}
                  type="button"
                  aria-label={choice}
                  onClick={() => setColor(choice)}
                  className={`size-8 rounded-full border-2 transition-transform ${choice === color ? "scale-110 border-foreground" : "border-transparent"}`}
                  style={{ backgroundColor: choice }}
                />
              ))}
            </div>
          </div>
          <div className="grid gap-2">
            <Label>{t("org.inviteExpiryLabel")}</Label>
            <div className="grid grid-cols-5 gap-1">
              {INVITE_EXPIRIES.map((value) => (
                <Button
                  key={value}
                  type="button"
                  size="sm"
                  variant={expiry === value ? "default" : "outline"}
                  onClick={() => setExpiry(value)}
                >
                  {value === "never" ? t("org.never") : value}
                </Button>
              ))}
            </div>
          </div>
          <div className="flex items-start gap-3 rounded-md border p-3">
            <Checkbox id="org-deposit-confirm" checked={confirmed} onCheckedChange={(checked) => setConfirmed(checked === true)} />
            <Label htmlFor="org-deposit-confirm" className="text-sm font-normal leading-5">
              {t("org.depositConfirm")}
            </Label>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>{t("common.cancel")}</Button>
          <Button disabled={busy || !confirmed || !name.trim()} onClick={() => onSubmit({ name, emoji, color, expiry, confirmed })}>
            {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {t("org.create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export { useMyOrgs, ORGS_KEY, OrgAvatar };

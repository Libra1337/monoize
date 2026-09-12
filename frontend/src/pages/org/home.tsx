import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import useSWR from "swr";
import { ArrowRight, Coins, Copy, KeyRound, Link2, Loader2, RefreshCw, UsersRound } from "lucide-react";
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
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { api, type OrgDetail, type OrgMember } from "@/lib/api";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import { formatCoinFromNanoUsdForCurrency } from "@/lib/store-money";
import { useMyOrgs, OrgAvatar } from "./shell";

function useMoney() {
  const { currency } = useStoreCurrency();
  const { data: rate } = useStoreExchangeRate();
  return (nano: string) =>
    currency === "CNY" && rate?.cny_per_usd
      ? formatCoinFromNanoUsdForCurrency(nano, "CNY", rate.cny_per_usd)
      : formatCoinFromNanoUsdForCurrency(nano, "USD", "1");
}

function amountToNanoUsd(raw: string, currency: "CNY" | "USD", cnyPerUsd?: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed || !/^\d+(\.\d{1,2})?$/.test(trimmed)) return null;
  const minor = Math.round(parseFloat(trimmed) * 100);
  if (!Number.isSafeInteger(minor) || minor <= 0) return null;
  if (currency === "USD") return (BigInt(minor) * 10_000_000n / 100n).toString();
  if (!cnyPerUsd) return null;
  const [whole, frac = ""] = cnyPerUsd.split(".");
  const numerator = BigInt(whole + frac.padEnd(6, "0").slice(0, 6));
  if (numerator <= 0n) return null;
  return ((BigInt(minor) * 1_000_000n * 10_000_000n + numerator / 2n) / numerator).toString();
}

/** ORG-18: the space overview — stats, wallet actions, and the single invite link. */
export function OrgHome() {
  const { orgId } = useParams();
  const { t } = useTranslation();
  const { mutate: reloadOrgs } = useMyOrgs();
  const money = useMoney();
  const { currency } = useStoreCurrency();
  const { data: rate } = useStoreExchangeRate();
  const detail = useSWR<OrgDetail>(orgId ? `/api/dashboard/orgs/${orgId}` : null, () =>
    api.getOrgDetail(orgId!),
  );
  const keys = useSWR(orgId ? `/api/dashboard/orgs/${orgId}/keys` : null, () =>
    api.listOrgKeys(orgId!),
  );

  const [depositOpen, setDepositOpen] = useState(false);
  const [depositAmount, setDepositAmount] = useState("");
  const [distributeTarget, setDistributeTarget] = useState<OrgMember | null>(null);
  const [distributeAmount, setDistributeAmount] = useState("");
  const [busy, setBusy] = useState(false);

  if (!orgId) return null;
  if (detail.isLoading || !detail.data) {
    return (
      <div className="space-y-4 p-6">
        <Skeleton className="h-28 w-full rounded-2xl" />
        <Skeleton className="h-40 w-full rounded-2xl" />
      </div>
    );
  }

  const isOwner = detail.data.my_role === "owner";
  const inviteLink = detail.data.invite?.token
    ? `${window.location.origin}/join/${detail.data.invite.token}`
    : "";
  const keyCount = (keys.data?.mine.length ?? 0) + (keys.data?.shared.length ?? 0);

  return (
    <div className="mx-auto max-w-5xl space-y-6 p-6">
      <header className="flex items-center gap-4">
        <OrgAvatar
          emoji={detail.data.avatar_emoji}
          color={detail.data.avatar_color}
          image={detail.data.avatar_image}
          size="size-16"
          text="text-3xl"
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-2xl font-semibold">{detail.data.display_name}</h1>
            <Badge variant={isOwner ? "default" : "secondary"}>
              {isOwner ? t("org.owner") : t("org.member")}
            </Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("org.members", { count: detail.data.members.length })}
            {" · "}
            {t("org.keysCount", { count: keyCount })}
          </p>
        </div>
      </header>

      <div className="grid gap-4 sm:grid-cols-3">
        <Card className="rounded-2xl">
          <CardContent className="p-5">
            <p className="flex items-center gap-1.5 text-sm text-muted-foreground">
              <Coins className="size-4" />
              {t("org.walletBalance")}
            </p>
            <p className="mt-2 text-2xl font-semibold tabular-nums">
              {money(detail.data.balance_nano_usd)}
            </p>
            {isOwner && (
              <Button variant="outline" size="sm" className="mt-3" onClick={() => setDepositOpen(true)}>
                {t("org.deposit")}
              </Button>
            )}
          </CardContent>
        </Card>
        <Card className="rounded-2xl">
          <CardContent className="p-5">
            <p className="flex items-center gap-1.5 text-sm text-muted-foreground">
              <UsersRound className="size-4" />
              {t("org.navMembers")}
            </p>
            <p className="mt-2 text-2xl font-semibold tabular-nums">
              {detail.data.members.length}
              <span className="ml-1 text-sm text-muted-foreground">/ 15</span>
            </p>
            <Button variant="outline" size="sm" className="mt-3" asChild>
              <Link to="members">
                {t("org.navMembers")}
                <ArrowRight className="ml-1 size-3.5" />
              </Link>
            </Button>
          </CardContent>
        </Card>
        <Card className="rounded-2xl">
          <CardContent className="p-5">
            <p className="flex items-center gap-1.5 text-sm text-muted-foreground">
              <KeyRound className="size-4" />
              {t("org.navKeys")}
            </p>
            <p className="mt-2 text-2xl font-semibold tabular-nums">{keyCount}</p>
            <Button variant="outline" size="sm" className="mt-3" asChild>
              <Link to="keys">
                {t("org.navKeys")}
                <ArrowRight className="ml-1 size-3.5" />
              </Link>
            </Button>
          </CardContent>
        </Card>
      </div>

      {isOwner && detail.data.invite && (
        <Card className="rounded-2xl">
          <CardContent className="flex flex-wrap items-center justify-between gap-3 p-5">
            <div className="flex min-w-0 items-center gap-3">
              <Link2 className="h-4 w-4 shrink-0 text-muted-foreground" />
              <div className="min-w-0">
                <p className="truncate font-mono text-sm">{inviteLink}</p>
                <p className="mt-0.5 text-xs text-muted-foreground">
                  {detail.data.invite.expires_at
                    ? t("org.inviteExpires", {
                        time: new Date(detail.data.invite.expires_at).toLocaleString(),
                      })
                    : t("org.inviteNeverExpires")}
                </p>
              </div>
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
              <Button
                variant="ghost"
                size="sm"
                onClick={async () => {
                  try {
                    await api.regenerateOrgInvite(orgId, "7d");
                    toast.success(t("org.linkRegenerated"));
                    await detail.mutate();
                  } catch (error) {
                    toast.error(error instanceof Error ? error.message : t("common.error"));
                  }
                }}
              >
                <RefreshCw className="h-3.5 w-3.5" />
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

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
            <Button variant="outline" onClick={() => setDepositOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              disabled={busy || !depositAmount.trim()}
              onClick={async () => {
                const nano = amountToNanoUsd(depositAmount, currency, rate?.cny_per_usd);
                if (!nano) {
                  toast.error(t("org.transferInvalidAmount"));
                  return;
                }
                setBusy(true);
                try {
                  await api.depositToOrg(orgId, nano);
                  toast.success(t("org.depositDone"));
                  setDepositOpen(false);
                  setDepositAmount("");
                  await Promise.all([detail.mutate(), reloadOrgs()]);
                } catch (error) {
                  toast.error(error instanceof Error ? error.message : t("common.error"));
                } finally {
                  setBusy(false);
                }
              }}
            >
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
            <Button variant="outline" onClick={() => setDistributeTarget(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              disabled={busy || !distributeAmount.trim()}
              onClick={async () => {
                const nano = amountToNanoUsd(distributeAmount, currency, rate?.cny_per_usd);
                if (!nano || !distributeTarget) {
                  toast.error(t("org.transferInvalidAmount"));
                  return;
                }
                setBusy(true);
                try {
                  await api.distributeFromOrg(orgId, distributeTarget.user_id, nano);
                  toast.success(t("org.distributeDone"));
                  setDistributeTarget(null);
                  setDistributeAmount("");
                  await Promise.all([detail.mutate(), reloadOrgs()]);
                } catch (error) {
                  toast.error(error instanceof Error ? error.message : t("common.error"));
                } finally {
                  setBusy(false);
                }
              }}
            >
              {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("org.distribute")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* expose distribute entry for the owner from the members context */}
      {isOwner && (
        <button
          type="button"
          className="hidden"
          onClick={() => {
            const member = detail.data?.members.find((m) => m.role !== "owner");
            if (member) setDistributeTarget(member);
          }}
        />
      )}
    </div>
  );
}

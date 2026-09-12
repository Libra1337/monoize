import { useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { KeyRound, Loader2, Plus, UsersRound } from "lucide-react";
import { Button } from "@/components/ui/button";
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
import { Card, CardContent } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper, motion, transitions } from "@/components/ui/motion";
import { useAuth } from "@/hooks/use-auth";
import { useStoreCurrency } from "@/hooks/use-store-currency";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import { api, type SubAccount } from "@/lib/api";
import { formatCoinFromNanoUsdForCurrency, parseRate } from "@/lib/store-money";

const SUBACCOUNTS_KEY = "/api/dashboard/subaccounts";

function useSubAccounts() {
  return useSWR<SubAccount[]>(SUBACCOUNTS_KEY, () => api.listSubAccounts(), {
    fallbackData: [],
  });
}

function useBalanceFormatter() {
  const { currency } = useStoreCurrency();
  const { data: rate } = useStoreExchangeRate();
  const cnyPerUsd = rate?.cny_per_usd;
  return (nanoUsd: string) =>
    currency === "CNY" && cnyPerUsd
      ? formatCoinFromNanoUsdForCurrency(nanoUsd, "CNY", cnyPerUsd)
      : formatCoinFromNanoUsdForCurrency(nanoUsd, "USD", "1");
}

/** Converts a display-currency amount into an integer nano-USD string, or null. */
function displayAmountToNanoUsd(
  raw: string,
  currency: "CNY" | "USD",
  cnyPerUsd: string | undefined,
): string | null {
  const trimmed = raw.trim();
  if (!trimmed || !/^\d+(\.\d{1,2})?$/.test(trimmed)) {
    return null;
  }
  const minorUnits = Math.round(parseFloat(trimmed) * 100);
  if (!Number.isSafeInteger(minorUnits) || minorUnits <= 0) {
    return null;
  }
  if (currency === "USD") {
    return (BigInt(minorUnits) * 10_000_000n / 100n).toString();
  }
  const rate = cnyPerUsd ? parseRate(cnyPerUsd) : null;
  if (!rate || rate.numerator <= 0n) {
    return null;
  }
  // nano-USD = CNY minor * 10^7 / (CNY per USD), rounded half away from zero.
  const scaled = (BigInt(minorUnits) * rate.denominator * 10_000_000n + rate.numerator / 2n)
    / rate.numerator;
  return scaled.toString();
}

export function SubAccountsPage() {
  const { t } = useTranslation();
  const { user, refreshUser } = useAuth();
  const { currency } = useStoreCurrency();
  const { data: rate } = useStoreExchangeRate();
  const { data: subs, isLoading, mutate } = useSubAccounts();
  const formatBalance = useBalanceFormatter();

  const [createOpen, setCreateOpen] = useState(false);
  const [newUsername, setNewUsername] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [creating, setCreating] = useState(false);

  const [transferTarget, setTransferTarget] = useState<SubAccount | null>(null);
  const [transferAmount, setTransferAmount] = useState("");
  const [transferring, setTransferring] = useState(false);

  const handleCreate = async () => {
    if (creating) return;
    setCreating(true);
    try {
      await api.createSubAccount({
        username: newUsername.trim(),
        password: newPassword,
      });
      toast.success(t("subaccounts.createSuccess", { name: newUsername.trim() }));
      setCreateOpen(false);
      setNewUsername("");
      setNewPassword("");
      await mutate();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : t("common.error"));
    } finally {
      setCreating(false);
    }
  };

  const handleTransfer = async () => {
    if (!transferTarget || transferring) return;
    const amountNano = displayAmountToNanoUsd(transferAmount, currency, rate?.cny_per_usd);
    if (!amountNano) {
      toast.error(t("subaccounts.transferInvalidAmount"));
      return;
    }
    setTransferring(true);
    try {
      await api.distributeToSubAccount(transferTarget.id, amountNano);
      toast.success(t("subaccounts.transferSuccess", { name: transferTarget.username }));
      setTransferTarget(null);
      setTransferAmount("");
      await mutate();
      await refreshUser();
    } catch (error) {
      toast.error(error instanceof Error ? error.message : t("common.error"));
    } finally {
      setTransferring(false);
    }
  };

  return (
    <PageWrapper className="space-y-6">
      <motion.div
        initial={{ opacity: 0, y: -10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={transitions.normal}
      >
        <PageHeader
          title={t("subaccounts.title")}
          description={t("subaccounts.description")}
          actions={
            <Button onClick={() => setCreateOpen(true)}>
              <Plus className="h-4 w-4 mr-2" />
              {t("subaccounts.create")}
            </Button>
          }
        />
      </motion.div>

      <Card className="rounded-2xl">
        <CardContent className="flex items-center justify-between gap-4 p-5">
          <div>
            <p className="text-sm text-muted-foreground">{t("subaccounts.mainBalance")}</p>
            <p className="mt-1 text-xl font-semibold tabular-nums">
              {user?.balance_unlimited
                ? t("store.ui.accountUnlimited")
                : formatBalance(user?.balance_nano_usd ?? "0")}
            </p>
          </div>
          <p className="max-w-sm text-xs text-muted-foreground">
            {t("subaccounts.distributeHint")}
          </p>
        </CardContent>
      </Card>

      {isLoading ? (
        <div className="space-y-2">
          <Skeleton className="h-16 w-full rounded-xl" />
          <Skeleton className="h-16 w-full rounded-xl" />
        </div>
      ) : !subs || subs.length === 0 ? (
        <EmptyState
          variant="card"
          icon={<UsersRound className="h-12 w-12" />}
          title={t("subaccounts.empty")}
          description={t("subaccounts.emptyDescription")}
          action={
            <Button variant="outline" onClick={() => setCreateOpen(true)}>
              <Plus className="h-4 w-4 mr-2" />
              {t("subaccounts.create")}
            </Button>
          }
        />
      ) : (
        <div className="overflow-x-auto rounded-2xl border">
          <table className="w-full min-w-[48rem] text-sm">
            <thead className="bg-muted/50 text-left text-muted-foreground">
              <tr>
                <th className="px-4 py-3 font-medium">{t("subaccounts.username")}</th>
                <th className="px-4 py-3 font-medium">{t("subaccounts.balance")}</th>
                <th className="px-4 py-3 font-medium">{t("subaccounts.apiKeys")}</th>
                <th className="px-4 py-3 font-medium">{t("subaccounts.todayCalls")}</th>
                <th className="px-4 py-3 font-medium">{t("subaccounts.todaySpend")}</th>
                <th className="px-4 py-3 font-medium">{t("common.created")}</th>
                <th className="px-4 py-3 font-medium">{t("common.actions")}</th>
              </tr>
            </thead>
            <tbody>
              {subs.map((sub) => (
                <tr key={sub.id} className="border-t transition-colors hover:bg-muted/30">
                  <td className="px-4 py-3">
                    <div className="flex items-center gap-2">
                      <span className="font-medium">{sub.username}</span>
                      <Badge variant={sub.enabled ? "secondary" : "outline"}>
                        {sub.enabled ? t("common.enabled") : t("common.disabled")}
                      </Badge>
                    </div>
                  </td>
                  <td className="px-4 py-3 tabular-nums">{formatBalance(sub.balance_nano_usd)}</td>
                  <td className="px-4 py-3 tabular-nums">
                    <span className="inline-flex items-center gap-1">
                      <KeyRound className="size-3.5 text-muted-foreground" />
                      {sub.api_key_count}
                    </span>
                  </td>
                  <td className="px-4 py-3 tabular-nums">{sub.today_calls.toLocaleString()}</td>
                  <td className="px-4 py-3 tabular-nums">
                    {formatBalance(sub.today_cost_nano_usd)}
                  </td>
                  <td className="px-4 py-3 text-muted-foreground">
                    {new Date(sub.created_at).toLocaleDateString()}
                  </td>
                  <td className="px-4 py-3">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => {
                        setTransferTarget(sub);
                        setTransferAmount("");
                      }}
                    >
                      {t("subaccounts.distribute")}
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("subaccounts.createTitle")}</DialogTitle>
            <DialogDescription>{t("subaccounts.createDescription")}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-4">
            <div className="grid gap-2">
              <Label htmlFor="sub-username">{t("subaccounts.username")}</Label>
              <Input
                id="sub-username"
                value={newUsername}
                onChange={(event) => setNewUsername(event.target.value)}
                placeholder="team_member"
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="sub-password">{t("subaccounts.password")}</Label>
              <Input
                id="sub-password"
                type="password"
                value={newPassword}
                onChange={(event) => setNewPassword(event.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                {t("subaccounts.passwordHint")}
              </p>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setCreateOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              onClick={handleCreate}
              disabled={
                creating ||
                !/^[A-Za-z0-9_]{3,22}$/.test(newUsername.trim()) ||
                newPassword.length < 8
              }
            >
              {creating && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("subaccounts.create")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!transferTarget} onOpenChange={(open) => !open && setTransferTarget(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>
              {t("subaccounts.transferTitle", { name: transferTarget?.username ?? "" })}
            </DialogTitle>
            <DialogDescription>
              {t("subaccounts.transferDescription", { currency })}
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-2">
            <Label htmlFor="transfer-amount">
              {t("subaccounts.transferAmount", { currency })}
            </Label>
            <Input
              id="transfer-amount"
              inputMode="decimal"
              value={transferAmount}
              onChange={(event) => setTransferAmount(event.target.value)}
              placeholder={currency === "CNY" ? "100.00" : "15.00"}
            />
            {transferTarget && (
              <p className="text-xs text-muted-foreground">
                {t("subaccounts.transferCurrent", {
                  balance: formatBalance(transferTarget.balance_nano_usd),
                })}
              </p>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setTransferTarget(null)}>
              {t("common.cancel")}
            </Button>
            <Button onClick={handleTransfer} disabled={transferring || !transferAmount.trim()}>
              {transferring && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              {t("subaccounts.distribute")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </PageWrapper>
  );
}

export const SUBACCOUNTS_SWR_KEY = SUBACCOUNTS_KEY;
export { useSubAccounts };

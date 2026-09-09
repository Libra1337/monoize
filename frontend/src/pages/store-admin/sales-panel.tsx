import { useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { Check, Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { CoinAmount } from "@/components/coin-amount";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { formatCoinFromMinor } from "@/lib/store-money";
import { StoreApiError } from "@/lib/store-api";
import { formatBasisPoints, salesApi, type SalesAgent } from "@/lib/sales-api";
import { cn } from "@/lib/utils";

const AGENTS_KEY = "admin:sales:agents";
const WITHDRAWALS_KEY = "admin:sales:withdrawals";
const ENTRIES_KEY = "admin:sales:entries";
const SETTINGS_KEY = "admin:sales:settings";

function coin(minor: string): string {
  const negative = minor.startsWith("-");
  const formatted = formatCoinFromMinor(negative ? minor.slice(1) : minor, "CNY", "1");
  return negative ? `-${formatted}` : formatted;
}

/** Percent input stored as basis points, so no float ever touches a rate. */
function percentToBasisPoints(value: string): number | null {
  const match = /^(\d{1,2})(?:\.(\d{1,2}))?$/.exec(value.trim());
  if (!match) return null;
  const whole = Number(match[1]);
  const fraction = (match[2] ?? "").padEnd(2, "0");
  return whole * 100 + Number(fraction);
}

export function SalesAdminPanel() {
  const { t } = useTranslation();
  const agents = useSWR(AGENTS_KEY, () => salesApi.admin.listAgents());
  const withdrawals = useSWR(WITHDRAWALS_KEY, () => salesApi.admin.listWithdrawals());
  const entries = useSWR(ENTRIES_KEY, () => salesApi.admin.listEntries());
  const settings = useSWR(SETTINGS_KEY, () => salesApi.admin.getSettings());

  const [creating, setCreating] = useState(false);
  const [newDiscount, setNewDiscount] = useState("0");
  const [created, setCreated] = useState<{ code: string; password: string } | null>(null);
  const [ratePercent, setRatePercent] = useState("");
  const [claimAgent, setClaimAgent] = useState("");
  const [claimOrder, setClaimOrder] = useState("");
  const [claimUser, setClaimUser] = useState("");
  const [busy, setBusy] = useState(false);

  const refresh = async () => {
    await Promise.all([agents.mutate(), withdrawals.mutate(), entries.mutate()]);
  };

  const createAgent = async () => {
    const bp = percentToBasisPoints(newDiscount);
    if (bp === null) {
      toast.error(t("store.admin.sales.discountInvalid"));
      return;
    }
    setBusy(true);
    try {
      const result = await salesApi.admin.createAgent(bp);
      // SC-7.2a: the password is shown once and is not retrievable afterwards.
      setCreated({ code: result.agent.code, password: result.password });
      setCreating(false);
      setNewDiscount("0");
      await refresh();
    } catch (error) {
      toast.error(error instanceof StoreApiError ? error.message : t("store.admin.sales.failed"));
    } finally {
      setBusy(false);
    }
  };

  const updateAgent = async (agent: SalesAgent, discountBp: number, enabled: boolean) => {
    try {
      await salesApi.admin.updateAgent(agent.user_id, discountBp, enabled);
      await agents.mutate();
    } catch (error) {
      toast.error(error instanceof StoreApiError ? error.message : t("store.admin.sales.failed"));
    }
  };

  const decide = async (id: string, decision: "paid" | "rejected") => {
    setBusy(true);
    try {
      await salesApi.admin.decideWithdrawal(id, decision, "");
      toast.success(t(`store.admin.sales.decided.${decision}`));
      await Promise.all([withdrawals.mutate(), agents.mutate()]);
    } catch (error) {
      toast.error(error instanceof StoreApiError ? error.message : t("store.admin.sales.failed"));
    } finally {
      setBusy(false);
    }
  };

  const saveRate = async () => {
    const bp = percentToBasisPoints(ratePercent);
    if (bp === null) {
      toast.error(t("store.admin.sales.rateInvalid"));
      return;
    }
    setBusy(true);
    try {
      await salesApi.admin.updateSettings(bp);
      toast.success(t("store.admin.sales.rateSaved"));
      setRatePercent("");
      await settings.mutate();
    } catch (error) {
      toast.error(error instanceof StoreApiError ? error.message : t("store.admin.sales.failed"));
    } finally {
      setBusy(false);
    }
  };

  const submitClaim = async () => {
    setBusy(true);
    try {
      await salesApi.admin.claimForAgent(claimAgent, claimOrder.trim(), claimUser.trim());
      toast.success(t("store.admin.sales.claimSuccess"));
      setClaimOrder("");
      setClaimUser("");
      await refresh();
    } catch (error) {
      toast.error(error instanceof StoreApiError ? error.message : t("store.admin.sales.failed"));
    } finally {
      setBusy(false);
    }
  };

  if (agents.isLoading && !agents.data) {
    return (
      <div className="flex flex-col gap-4" aria-hidden="true">
        <Skeleton className="h-24 rounded-2xl" />
        <Skeleton className="h-64 rounded-2xl" />
        <Skeleton className="h-48 rounded-2xl" />
      </div>
    );
  }

  const pending = (withdrawals.data ?? []).filter((item) => item.state === "requested");
  const agentList = agents.data ?? [];

  return (
    <div className="flex flex-col gap-4">
      <Card className="rounded-2xl">
        <CardContent className="flex flex-wrap items-end justify-between gap-4 p-5">
          <div>
            <p className="text-sm font-medium">{t("store.admin.sales.rate")}</p>
            <p className="mt-1 font-mono text-2xl font-semibold tabular-nums">
              {settings.data ? formatBasisPoints(settings.data.commission_rate_bp) : "—"}
            </p>
            <p className="mt-1 text-xs text-muted-foreground text-pretty">
              {t("store.admin.sales.rateHelp")}
            </p>
          </div>
          <div className="flex items-end gap-2">
            <div className="flex flex-col gap-2">
              <Label htmlFor="sales-rate">{t("store.admin.sales.newRate")}</Label>
              <Input
                id="sales-rate"
                inputMode="decimal"
                value={ratePercent}
                onChange={(event) => setRatePercent(event.target.value)}
                className="h-11 w-28 rounded-xl font-mono"
                placeholder="5"
              />
            </div>
            <Button
              type="button"
              variant="outline"
              className="h-11 rounded-xl"
              disabled={busy || ratePercent.trim() === ""}
              onClick={() => void saveRate()}
            >
              {t("store.admin.sales.saveRate")}
            </Button>
          </div>
        </CardContent>
      </Card>

      {created && (
        <Card className="rounded-2xl border-success-border bg-success-soft">
          <CardContent className="flex flex-wrap items-start justify-between gap-4 p-5">
            <div className="min-w-0">
              <p className="text-sm font-medium text-success-foreground">
                {t("store.admin.sales.createdTitle")}
              </p>
              <dl className="mt-2 flex flex-col gap-1 font-mono text-sm text-success-foreground">
                <div className="flex gap-2">
                  <dt>{t("store.admin.sales.createdCode")}</dt>
                  <dd className="font-semibold">{created.code}</dd>
                </div>
                <div className="flex gap-2">
                  <dt>{t("store.admin.sales.createdPassword")}</dt>
                  <dd className="font-semibold">{created.password}</dd>
                </div>
              </dl>
              <p className="mt-2 text-xs text-success-foreground text-pretty">
                {t("store.admin.sales.createdOnce")}
              </p>
            </div>
            <Button
              type="button"
              variant="outline"
              className="h-11 rounded-xl"
              onClick={() => setCreated(null)}
            >
              {t("store.admin.sales.dismiss")}
            </Button>
          </CardContent>
        </Card>
      )}

      <Card className="rounded-2xl">
        <CardContent className="flex flex-col gap-4 p-5">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <h3 className="text-base font-semibold">{t("store.admin.sales.agents")}</h3>
            {creating ? (
              <div className="flex items-end gap-2">
                <div className="flex flex-col gap-2">
                  <Label htmlFor="sales-new-discount">
                    {t("store.admin.sales.discount")}
                  </Label>
                  <Input
                    id="sales-new-discount"
                    inputMode="decimal"
                    value={newDiscount}
                    onChange={(event) => setNewDiscount(event.target.value)}
                    className="h-11 w-28 rounded-xl font-mono"
                  />
                </div>
                <Button
                  type="button"
                  className="h-11 rounded-xl"
                  disabled={busy}
                  onClick={() => void createAgent()}
                >
                  {t("store.admin.sales.confirmCreate")}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  className="h-11 rounded-xl"
                  onClick={() => setCreating(false)}
                >
                  {t("store.admin.sales.cancel")}
                </Button>
              </div>
            ) : (
              <Button
                type="button"
                className="h-11 rounded-xl"
                onClick={() => setCreating(true)}
              >
                <Plus className="size-4" />
                {t("store.admin.sales.create")}
              </Button>
            )}
          </div>

          {agentList.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("store.admin.sales.noAgents")}</p>
          ) : (
            <ul className="divide-y rounded-xl border">
              {agentList.map((agent) => (
                <li key={agent.user_id} className="flex flex-wrap items-center gap-4 p-4">
                  <div className="min-w-0 flex-1">
                    <p className="font-mono text-sm font-semibold">{agent.code}</p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {t("store.admin.sales.discountOf", {
                        percent: formatBasisPoints(agent.discount_bp),
                      })}
                    </p>
                  </div>
                  <p
                    className={cn(
                      "font-mono text-sm tabular-nums",
                      agent.commission_balance_minor.startsWith("-") && "text-destructive",
                    )}
                  >
                    <CoinAmount value={coin(agent.commission_balance_minor)} />
                  </p>
                  <label className="flex items-center gap-2 text-xs">
                    <span className="text-muted-foreground">
                      {t("store.admin.sales.enabled")}
                    </span>
                    <Switch
                      checked={agent.enabled}
                      onCheckedChange={(enabled) =>
                        void updateAgent(agent, agent.discount_bp, enabled)
                      }
                    />
                  </label>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card className="rounded-2xl">
        <CardContent className="flex flex-col gap-4 p-5">
          <h3 className="text-base font-semibold">
            {t("store.admin.sales.withdrawals")}
            {pending.length > 0 && (
              <span className="ml-2 rounded-md bg-warning-soft px-2 py-0.5 text-xs text-warning-foreground">
                {t("store.admin.sales.pendingCount", { count: pending.length })}
              </span>
            )}
          </h3>
          {(withdrawals.data ?? []).length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {t("store.admin.sales.noWithdrawals")}
            </p>
          ) : (
            <ul className="divide-y rounded-xl border">
              {(withdrawals.data ?? []).map((item) => (
                <li key={item.id} className="flex flex-wrap items-center gap-4 p-4">
                  <div className="min-w-0 flex-1">
                    <p className="font-mono text-sm">{item.agent_username}</p>
                    <p className="mt-1 text-xs text-muted-foreground">{item.requested_at}</p>
                  </div>
                  <p className="font-mono text-sm font-semibold tabular-nums">
                    <CoinAmount value={coin(item.amount_minor)} />
                  </p>
                  {item.state === "requested" ? (
                    <div className="flex gap-2">
                      <Button
                        type="button"
                        className="h-11 rounded-xl"
                        disabled={busy}
                        onClick={() => void decide(item.id, "paid")}
                      >
                        <Check className="size-4" />
                        {t("store.admin.sales.markPaid")}
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        className="h-11 rounded-xl"
                        disabled={busy}
                        onClick={() => void decide(item.id, "rejected")}
                      >
                        <X className="size-4" />
                        {t("store.admin.sales.reject")}
                      </Button>
                    </div>
                  ) : (
                    <span className="text-xs text-muted-foreground">
                      {t(`sales.withdrawal.state.${item.state}`)}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card className="rounded-2xl">
        <CardContent className="flex flex-col gap-4 p-5">
          <div>
            <h3 className="text-base font-semibold">{t("store.admin.sales.claimTitle")}</h3>
            <p className="mt-1 text-sm text-muted-foreground text-pretty">
              {t("store.admin.sales.claimHelp")}
            </p>
          </div>
          <div className="grid gap-3 sm:grid-cols-3">
            <div className="flex flex-col gap-2">
              <Label htmlFor="admin-claim-agent">{t("store.admin.sales.claimAgent")}</Label>
              <select
                id="admin-claim-agent"
                value={claimAgent}
                onChange={(event) => setClaimAgent(event.target.value)}
                className="h-11 rounded-xl border bg-background px-3 text-sm"
              >
                <option value="">{t("store.admin.sales.claimSelect")}</option>
                {agentList.map((agent) => (
                  <option key={agent.user_id} value={agent.user_id}>
                    {agent.code}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="admin-claim-order">{t("store.admin.sales.claimOrder")}</Label>
              <Input
                id="admin-claim-order"
                value={claimOrder}
                onChange={(event) => setClaimOrder(event.target.value)}
                className="h-11 rounded-xl font-mono"
                placeholder="LS-..."
              />
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="admin-claim-user">{t("store.admin.sales.claimUser")}</Label>
              <Input
                id="admin-claim-user"
                value={claimUser}
                onChange={(event) => setClaimUser(event.target.value)}
                className="h-11 rounded-xl font-mono"
              />
            </div>
          </div>
          <Button
            type="button"
            className="h-11 w-full rounded-xl sm:w-auto sm:self-start"
            disabled={
              busy || claimAgent === "" || claimOrder.trim() === "" || claimUser.trim() === ""
            }
            onClick={() => void submitClaim()}
          >
            {t("store.admin.sales.claimSubmit")}
          </Button>
        </CardContent>
      </Card>

      <Card className="rounded-2xl">
        <CardContent className="flex flex-col gap-4 p-5">
          <h3 className="text-base font-semibold">{t("store.admin.sales.entries")}</h3>
          {(entries.data ?? []).length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("store.admin.sales.noEntries")}</p>
          ) : (
            <div className="overflow-x-auto rounded-xl border">
              <table className="w-full text-sm">
                <caption className="sr-only">{t("store.admin.sales.entries")}</caption>
                <thead className="border-b bg-muted/35 text-left">
                  <tr>
                    <th scope="col" className="px-4 py-3 font-medium">
                      {t("store.admin.sales.entryAgent")}
                    </th>
                    <th scope="col" className="px-4 py-3 font-medium">
                      {t("store.admin.sales.entryOrder")}
                    </th>
                    <th scope="col" className="px-4 py-3 font-medium">
                      {t("store.admin.sales.entryBuyer")}
                    </th>
                    <th scope="col" className="px-4 py-3 font-medium">
                      {t("store.admin.sales.entryCommission")}
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y">
                  {(entries.data ?? []).map((entry) => (
                    <tr
                      key={entry.id}
                      className={entry.reversed_at ? "text-muted-foreground" : ""}
                    >
                      <td className="px-4 py-3 font-mono text-xs">{entry.agent_username}</td>
                      <td className="px-4 py-3 font-mono text-xs">
                        {entry.order_number}
                        {entry.reversed_at && (
                          <span className="ml-2 rounded-md bg-destructive/10 px-1.5 py-0.5 text-xs text-destructive">
                            {t("sales.entries.reversed")}
                          </span>
                        )}
                      </td>
                      <td className="px-4 py-3 font-mono text-xs">{entry.buyer_user_id}</td>
                      <td className="px-4 py-3 font-mono tabular-nums">
                        <CoinAmount value={coin(entry.commission_minor)} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

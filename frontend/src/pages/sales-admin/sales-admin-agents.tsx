import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { CoinAmount } from "@/components/coin-amount";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { StoreApiError } from "@/lib/store-api";
import { type SalesAgent, formatBasisPoints, salesApi } from "@/lib/sales-api";
import { cn } from "@/lib/utils";
import { coin, percentToBasisPoints } from "./sales-admin-shared";

export interface SalesAdminAgentsProps {
  agents: SalesAgent[];
  commissionRateBp: number | null;
  onAgentsChanged: () => Promise<unknown>;
  onRateChanged: () => Promise<unknown>;
}

/**
 * Commission rate, agent creation, and the agent roster (SC-7.1, SC-7.2, SC-7.3).
 *
 * Each agent's discount is shown but not editable: it is funded from that agent's own
 * commission, so only the agent may change it (SC-1.2a). Admin controls the enabled flag.
 */
export function SalesAdminAgents({
  agents,
  commissionRateBp,
  onAgentsChanged,
  onRateChanged,
}: SalesAdminAgentsProps) {
  const { t } = useTranslation();
  const [creating, setCreating] = useState(false);
  const [newDiscount, setNewDiscount] = useState("0");
  const [created, setCreated] = useState<{ code: string; password: string } | null>(null);
  const [ratePercent, setRatePercent] = useState("");
  const [busy, setBusy] = useState(false);

  // Summed as BigInt: these are Coin minor units held as strings precisely so no float ever
  // touches money.
  const totals = useMemo(() => {
    const sum = (pick: (agent: SalesAgent) => string) =>
      agents.reduce((total, agent) => total + BigInt(pick(agent)), 0n).toString();
    return {
      accrued: sum((agent) => agent.settlement.accrued_minor),
      available: sum((agent) => agent.settlement.available_minor),
      pending: sum((agent) => agent.settlement.pending_withdrawal_minor),
      withdrawn: sum((agent) => agent.settlement.withdrawn_minor),
    };
  }, [agents]);

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
      await onAgentsChanged();
    } catch (error) {
      toast.error(error instanceof StoreApiError ? error.message : t("store.admin.sales.failed"));
    } finally {
      setBusy(false);
    }
  };

  const setEnabled = async (agent: SalesAgent, enabled: boolean) => {
    try {
      await salesApi.admin.setAgentEnabled(agent.user_id, enabled);
      await onAgentsChanged();
    } catch (error) {
      toast.error(error instanceof StoreApiError ? error.message : t("store.admin.sales.failed"));
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
      await onRateChanged();
    } catch (error) {
      toast.error(error instanceof StoreApiError ? error.message : t("store.admin.sales.failed"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <Card className="rounded-2xl">
        <CardContent className="flex flex-wrap items-end justify-between gap-4 p-5">
          <div>
            <p className="text-sm font-medium">{t("store.admin.sales.rate")}</p>
            <p className="mt-1 font-mono text-2xl font-semibold tabular-nums">
              {commissionRateBp === null ? "—" : formatBasisPoints(commissionRateBp)}
            </p>
            <p className="mt-1 text-xs text-muted-foreground text-pretty">
              {t("store.admin.sales.rateHelp")}
            </p>
          </div>
          {/* SC-6.10: the same four figures, summed across every agent, so Admin can see the
              platform's total commission liability without adding up the roster by hand. */}
          <dl className="grid grid-cols-2 gap-x-6 gap-y-2 sm:grid-cols-4">
            {(
              [
                ["accrued", totals.accrued],
                ["available", totals.available],
                ["pending", totals.pending],
                ["withdrawn", totals.withdrawn],
              ] as const
            ).map(([key, value]) => (
              <div key={key} className="flex flex-col">
                <dt className="text-xs text-muted-foreground">
                  {t(`sales.settlement.${key}`)}
                </dt>
                <dd
                  className={cn(
                    "font-mono text-sm font-medium tabular-nums",
                    value.startsWith("-") && "text-destructive",
                  )}
                >
                  <CoinAmount value={coin(value)} />
                </dd>
              </div>
            ))}
          </dl>
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
                  <Label htmlFor="sales-new-discount">{t("store.admin.sales.discount")}</Label>
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
              <Button type="button" className="h-11 rounded-xl" onClick={() => setCreating(true)}>
                <Plus className="size-4" />
                {t("store.admin.sales.create")}
              </Button>
            )}
          </div>

          {agents.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("store.admin.sales.noAgents")}</p>
          ) : (
            <ul className="divide-y rounded-xl border">
              {agents.map((agent) => (
                <li key={agent.user_id} className="flex flex-wrap items-center gap-4 p-4">
                  <div className="min-w-0 flex-1 basis-40">
                    <p className="font-mono text-sm font-semibold">{agent.code}</p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {t("store.admin.sales.discountReadOnly", {
                        percent: formatBasisPoints(agent.discount_bp),
                      })}
                    </p>
                  </div>
                  <dl className="grid grid-cols-2 gap-x-4 gap-y-1 sm:grid-cols-4">
                    {(
                      [
                        ["accrued", agent.settlement.accrued_minor],
                        ["available", agent.settlement.available_minor],
                        ["pending", agent.settlement.pending_withdrawal_minor],
                        ["withdrawn", agent.settlement.withdrawn_minor],
                      ] as const
                    ).map(([key, value]) => (
                      <div key={key} className="flex flex-col">
                        <dt className="text-xs text-muted-foreground">
                          {t(`sales.settlement.${key}`)}
                        </dt>
                        <dd
                          className={cn(
                            "font-mono text-sm tabular-nums",
                            value.startsWith("-") && "text-destructive",
                          )}
                        >
                          <CoinAmount value={coin(value)} />
                        </dd>
                      </div>
                    ))}
                  </dl>
                  <label className="flex items-center gap-2 text-xs">
                    <span className="text-muted-foreground">
                      {t("store.admin.sales.enabled")}
                    </span>
                    <Switch
                      checked={agent.enabled}
                      onCheckedChange={(enabled) => void setEnabled(agent, enabled)}
                    />
                  </label>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

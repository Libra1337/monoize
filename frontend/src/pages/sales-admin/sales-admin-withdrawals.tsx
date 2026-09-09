import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Check, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { CoinAmount } from "@/components/coin-amount";
import { StoreApiError } from "@/lib/store-api";
import { type SalesWithdrawal, salesApi } from "@/lib/sales-api";
import { coin } from "./sales-admin-shared";

export interface SalesAdminWithdrawalsProps {
  withdrawals: SalesWithdrawal[];
  onDecided: () => Promise<unknown>;
}

/**
 * Withdrawal queue (SC-7.4).
 *
 * A decision is settled outside the system, so this surface records the outcome rather than
 * moving money. Requested items are actionable; every other state is shown as history.
 */
export function SalesAdminWithdrawals({ withdrawals, onDecided }: SalesAdminWithdrawalsProps) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const pending = withdrawals.filter((item) => item.state === "requested");

  const decide = async (id: string, decision: "paid" | "rejected") => {
    setBusy(true);
    try {
      await salesApi.admin.decideWithdrawal(id, decision, "");
      toast.success(t(`store.admin.sales.decided.${decision}`));
      await onDecided();
    } catch (error) {
      toast.error(error instanceof StoreApiError ? error.message : t("store.admin.sales.failed"));
    } finally {
      setBusy(false);
    }
  };

  return (
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
        {withdrawals.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("store.admin.sales.noWithdrawals")}</p>
        ) : (
          <ul className="divide-y rounded-xl border">
            {withdrawals.map((item) => (
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
  );
}

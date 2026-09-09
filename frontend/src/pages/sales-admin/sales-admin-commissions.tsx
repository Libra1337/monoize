import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { CoinAmount } from "@/components/coin-amount";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { StoreApiError } from "@/lib/store-api";
import { type AdminSalesEntry, type SalesAgent, salesApi } from "@/lib/sales-api";
import { coin } from "./sales-admin-shared";

export interface SalesAdminCommissionsProps {
  agents: SalesAgent[];
  entries: AdminSalesEntry[];
  onClaimed: () => Promise<unknown>;
}

/**
 * Delegated claims and the commission ledger (SC-7.5, SC-7.6).
 *
 * A delegated claim applies the same rules as an agent's own claim, including the one-time
 * credit per order, so Admin cannot use it to credit an order twice.
 */
export function SalesAdminCommissions({
  agents,
  entries,
  onClaimed,
}: SalesAdminCommissionsProps) {
  const { t } = useTranslation();
  const [claimAgent, setClaimAgent] = useState("");
  const [claimOrder, setClaimOrder] = useState("");
  const [claimUser, setClaimUser] = useState("");
  const [busy, setBusy] = useState(false);

  const submitClaim = async () => {
    setBusy(true);
    try {
      await salesApi.admin.claimForAgent(claimAgent, claimOrder.trim(), claimUser.trim());
      toast.success(t("store.admin.sales.claimSuccess"));
      setClaimOrder("");
      setClaimUser("");
      await onClaimed();
    } catch (error) {
      toast.error(error instanceof StoreApiError ? error.message : t("store.admin.sales.failed"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
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
                {agents.map((agent) => (
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
          {entries.length === 0 ? (
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
                  {entries.map((entry) => (
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

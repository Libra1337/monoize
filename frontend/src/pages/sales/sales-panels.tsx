import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Banknote, ReceiptText } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { CoinAmount } from "@/components/coin-amount";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { formatCoinFromMinor, decimalToMinor } from "@/lib/store-money";
import { StoreApiError } from "@/lib/store-api";
import {
  salesApi,
  type SalesCommissionEntry,
  type SalesWithdrawal,
} from "@/lib/sales-api";

function coin(minor: string): string {
  return formatCoinFromMinor(minor, "CNY", "1");
}

/** SC-5.1: any positive balance is withdrawable, down to one minor unit. */
const MINIMUM_WITHDRAWAL_MINOR = 1n;

export function SalesEntryList({ entries }: { entries: SalesCommissionEntry[] }) {
  const { t } = useTranslation();

  if (entries.length === 0) {
    return (
      <EmptyState
        icon={<ReceiptText className="size-11" />}
        title={t("sales.entries.empty")}
        description={t("sales.entries.emptyHelp")}
      />
    );
  }

  return (
    <div className="overflow-hidden rounded-2xl border bg-card">
      <table className="w-full text-sm">
        <caption className="sr-only">{t("sales.entries.title")}</caption>
        <thead className="border-b bg-muted/35 text-left">
          <tr>
            <th scope="col" className="px-4 py-3 font-medium">{t("sales.entries.order")}</th>
            <th scope="col" className="px-4 py-3 font-medium">{t("sales.entries.amount")}</th>
            <th scope="col" className="px-4 py-3 font-medium">{t("sales.entries.commission")}</th>
            <th scope="col" className="px-4 py-3 font-medium">{t("sales.entries.when")}</th>
          </tr>
        </thead>
        <tbody className="divide-y">
          {entries.map((entry) => (
            <tr key={entry.id} className={entry.reversed_at ? "text-muted-foreground" : ""}>
              <td className="px-4 py-3 font-mono text-xs">
                {entry.order_number}
                {entry.origin === "claim" && (
                  <span className="ml-2 rounded-md bg-muted px-1.5 py-0.5 text-xs">
                    {t("sales.entries.claimed")}
                  </span>
                )}
                {/* A reversed entry stays visible: it is the origin of any debt (SC-UI-2b). */}
                {entry.reversed_at && (
                  <span className="ml-2 rounded-md bg-destructive/10 px-1.5 py-0.5 text-xs text-destructive">
                    {t("sales.entries.reversed")}
                  </span>
                )}
              </td>
              <td className="px-4 py-3 font-mono tabular-nums">
                <CoinAmount value={coin(entry.base_minor)} />
              </td>
              <td className="px-4 py-3 font-mono tabular-nums">
                <CoinAmount value={coin(entry.commission_minor)} />
              </td>
              <td className="px-4 py-3 text-xs text-muted-foreground">{entry.created_at}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** SC-4: an agent claims a past order the buyer placed without the code. */
export function SalesClaimPanel({ onClaimed }: { onClaimed: () => Promise<unknown> }) {
  const { t } = useTranslation();
  const [orderNumber, setOrderNumber] = useState("");
  const [userId, setUserId] = useState("");
  const [busy, setBusy] = useState(false);

  const ready = orderNumber.trim().length > 0 && userId.trim().length > 0;

  const submit = async () => {
    setBusy(true);
    try {
      await salesApi.claim(orderNumber.trim(), userId.trim());
      toast.success(t("sales.claim.success"));
      setOrderNumber("");
      setUserId("");
      await onClaimed();
    } catch (error) {
      // The server returns one indistinguishable error for a missing order and a mismatched
      // buyer, so the message is shown verbatim rather than reinterpreted here.
      toast.error(
        error instanceof StoreApiError ? error.message : t("sales.claim.failed"),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card className="rounded-2xl">
      <CardContent className="flex flex-col gap-4 p-5">
        <div>
          <h2 className="text-base font-semibold">{t("sales.claim.title")}</h2>
          <p className="mt-1 text-sm text-muted-foreground text-pretty">
            {t("sales.claim.description")}
          </p>
        </div>
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-2">
            <Label htmlFor="sales-claim-order">{t("sales.claim.orderNumber")}</Label>
            <Input
              id="sales-claim-order"
              value={orderNumber}
              onChange={(event) => setOrderNumber(event.target.value)}
              className="h-11 rounded-xl font-mono"
              placeholder="LS-..."
            />
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor="sales-claim-user">{t("sales.claim.userId")}</Label>
            <Input
              id="sales-claim-user"
              value={userId}
              onChange={(event) => setUserId(event.target.value)}
              className="h-11 rounded-xl font-mono"
            />
          </div>
        </div>
        <Button
          type="button"
          className="h-11 rounded-xl"
          disabled={busy || !ready}
          onClick={() => void submit()}
        >
          {t("sales.claim.submit")}
        </Button>
      </CardContent>
    </Card>
  );
}

/** SC-5: request a withdrawal; the Admin settles it out of band. */
export function SalesWithdrawalPanel({
  balanceMinor,
  pending,
  withdrawals,
  onRequested,
}: {
  balanceMinor: string;
  pending: SalesWithdrawal | null;
  withdrawals: SalesWithdrawal[];
  onRequested: () => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [amount, setAmount] = useState("");
  const [busy, setBusy] = useState(false);

  const cancel = async (id: string) => {
    setBusy(true);
    try {
      await salesApi.cancelWithdrawal(id);
      toast.success(t("sales.withdrawal.cancelled"));
      await onRequested();
    } catch (error) {
      toast.error(
        error instanceof StoreApiError ? error.message : t("sales.withdrawal.failed"),
      );
    } finally {
      setBusy(false);
    }
  };

  const minor = decimalToMinor(amount);
  const balance = BigInt(balanceMinor);
  // A negative balance is a debt, so no positive request can satisfy the cap (SC-5.1a).
  const withinBalance = minor !== null && BigInt(minor) <= balance;
  const aboveMinimum = minor !== null && BigInt(minor) >= MINIMUM_WITHDRAWAL_MINOR;
  const ready = pending === null && withinBalance && aboveMinimum;

  const submit = async () => {
    if (minor === null) return;
    setBusy(true);
    try {
      await salesApi.requestWithdrawal(minor);
      toast.success(t("sales.withdrawal.requested"));
      setAmount("");
      await onRequested();
    } catch (error) {
      toast.error(
        error instanceof StoreApiError ? error.message : t("sales.withdrawal.failed"),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card className="rounded-2xl">
      <CardContent className="flex flex-col gap-4 p-5">
        <div>
          <h2 className="text-base font-semibold">{t("sales.withdrawal.title")}</h2>
          <p className="mt-1 text-sm text-muted-foreground text-pretty">
            {t("sales.withdrawal.description")}
          </p>
        </div>

        {pending ? (
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-warning-border bg-warning-soft px-4 py-3 text-sm">
            <div>
              <p className="font-medium text-warning-foreground">
                {t("sales.withdrawal.pending")}
              </p>
              <p className="mt-1 font-mono tabular-nums text-warning-foreground">
                <CoinAmount value={coin(pending.amount_minor)} />
              </p>
            </div>
            <Button
              type="button"
              variant="outline"
              className="h-11 rounded-xl"
              disabled={busy}
              onClick={() => void cancel(pending.id)}
            >
              {t("sales.withdrawal.cancel")}
            </Button>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-2">
              <Label htmlFor="sales-withdrawal-amount">
                {t("sales.withdrawal.amount")}
              </Label>
              <Input
                id="sales-withdrawal-amount"
                inputMode="decimal"
                value={amount}
                onChange={(event) => setAmount(event.target.value)}
                className="h-11 rounded-xl font-mono"
                placeholder="100.00"
                aria-describedby="sales-withdrawal-help"
              />
              <p id="sales-withdrawal-help" className="text-xs text-muted-foreground">
                {t("sales.withdrawal.available", { amount: coin(balanceMinor) })}
              </p>
            </div>
            <Button
              type="button"
              className="h-11 rounded-xl"
              disabled={busy || !ready}
              onClick={() => void submit()}
            >
              {t("sales.withdrawal.submit")}
            </Button>
          </div>
        )}

        {withdrawals.length > 0 && (
          <div className="flex flex-col gap-2 border-t pt-3">
            <h3 className="text-sm font-medium">{t("sales.withdrawal.log")}</h3>
            <ul className="flex flex-col divide-y">
              {withdrawals.map((withdrawal) => (
                <li key={withdrawal.id} className="flex flex-col gap-1 py-2 text-sm">
                  <div className="flex items-center justify-between gap-3">
                    <span className="font-mono tabular-nums">
                      <CoinAmount value={coin(withdrawal.amount_minor)} />
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {t(`sales.withdrawal.state.${withdrawal.state}`)}
                    </span>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    {withdrawal.decided_at ?? withdrawal.requested_at}
                  </p>
                </li>
              ))}
            </ul>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function SalesWithdrawalIcon() {
  return <Banknote className="size-11" />;
}

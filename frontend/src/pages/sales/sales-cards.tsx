import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { CoinAmount } from "@/components/coin-amount";
import { formatCoinFromMinor } from "@/lib/store-money";
import { formatBasisPoints, type SalesAgent, type SalesWindow } from "@/lib/sales-api";
import { cn } from "@/lib/utils";

/** Renders a Coin minor amount. Coin is pegged to CNY, so no exchange rate is applied. */
function coin(minor: string): string {
  return formatCoinFromMinor(minor, "CNY", "1");
}

export function SalesCodeCard({ agent }: { agent: SalesAgent }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    await navigator.clipboard.writeText(agent.code);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 2000);
  };

  return (
    <Card className="rounded-2xl">
      <CardContent className="flex flex-wrap items-center justify-between gap-4 p-5">
        <div className="min-w-0">
          <p className="text-sm text-muted-foreground">{t("sales.code.label")}</p>
          <p className="mt-1 font-mono text-3xl font-semibold tracking-widest">{agent.code}</p>
          {agent.discount_bp > 0 && (
            <p className="mt-2 text-sm text-muted-foreground text-pretty">
              {t("sales.code.discount", { percent: formatBasisPoints(agent.discount_bp) })}
            </p>
          )}
        </div>
        <button
          type="button"
          onClick={() => void copy()}
          className="flex min-h-11 shrink-0 items-center gap-2 rounded-xl border px-4 text-sm font-medium transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
        >
          {copied ? <Check className="size-4 text-success" /> : <Copy className="size-4" />}
          {copied ? t("sales.code.copied") : t("sales.code.copy")}
        </button>
      </CardContent>
    </Card>
  );
}

/**
 * The commission balance, shown signed (SC-UI-2b).
 *
 * A refund reverses an accrual the agent may already have withdrawn, so the balance can go
 * negative. The sign is rendered directly rather than relabelled, because the formatter only
 * accepts a nonnegative amount.
 */
export function SalesBalanceCard({ balanceMinor }: { balanceMinor: string }) {
  const { t } = useTranslation();
  const negative = balanceMinor.startsWith("-");
  const magnitude = negative ? balanceMinor.slice(1) : balanceMinor;

  return (
    <Card className={cn("rounded-2xl", negative && "border-destructive/40")}>
      <CardContent className="p-5">
        <p className="text-sm text-muted-foreground">{t("sales.balance.label")}</p>
        <p
          className={cn(
            "mt-1 font-mono text-2xl font-semibold tabular-nums",
            negative && "text-destructive",
          )}
        >
          {negative && <span aria-hidden="true">-</span>}
          <span className="sr-only">{negative ? t("sales.balance.negative") : ""}</span>
          <CoinAmount value={coin(magnitude)} />
        </p>
      </CardContent>
    </Card>
  );
}

export function SalesWindowCard({
  title,
  window: data,
}: {
  title: string;
  window: SalesWindow;
}) {
  const { t } = useTranslation();
  return (
    <Card className="rounded-2xl">
      <CardContent className="p-5">
        <p className="text-sm text-muted-foreground">{title}</p>
        <dl className="mt-3 flex flex-col gap-2">
          <div className="flex items-baseline justify-between gap-2">
            <dt className="text-xs text-muted-foreground">{t("sales.window.sales")}</dt>
            <dd className="font-mono text-lg font-semibold tabular-nums">
              <CoinAmount value={coin(data.sales_minor)} />
            </dd>
          </div>
          <div className="flex items-baseline justify-between gap-2">
            <dt className="text-xs text-muted-foreground">{t("sales.window.commission")}</dt>
            <dd className="font-mono text-sm tabular-nums">
              <CoinAmount value={coin(data.commission_minor)} />
            </dd>
          </div>
          <div className="flex items-baseline justify-between gap-2">
            <dt className="text-xs text-muted-foreground">{t("sales.window.orders")}</dt>
            <dd className="font-mono text-sm tabular-nums">{data.order_count}</dd>
          </div>
        </dl>
      </CardContent>
    </Card>
  );
}

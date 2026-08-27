import { LoaderCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { StorePaymentChannel, StoreProduct } from "@/lib/store-api";
import { addMinor, convertMinor, formatMinor, type StoreCurrency } from "@/lib/store-money";

interface OrderSummaryProps {
  product: StoreProduct | null;
  paymentChannel: StorePaymentChannel | null;
  currency: StoreCurrency;
  cnyPerUsd: string;
  customRechargeMinor: string | null;
  customAmountInvalid: boolean;
  submitting: boolean;
  onSubmit: () => void;
}

function displayMinor(
  product: StoreProduct,
  currency: StoreCurrency,
  cnyPerUsd: string,
  customRechargeMinor: string | null,
) {
  if (customRechargeMinor !== null) return customRechargeMinor;
  return convertMinor(product.price_minor, product.price_currency, currency, cnyPerUsd);
}

export function OrderSummary({
  product,
  paymentChannel,
  currency,
  cnyPerUsd,
  customRechargeMinor,
  customAmountInvalid,
  submitting,
  onSubmit,
}: OrderSummaryProps) {
  const { t } = useTranslation();
  const totalMinor = product
    ? displayMinor(product, currency, cnyPerUsd, customRechargeMinor)
    : null;
  const actualReceivedMinor = product?.balance
    ? customRechargeMinor ?? addMinor(
        convertMinor(
          product.balance.recharge_minor,
          product.price_currency,
          currency,
          cnyPerUsd,
        ),
        convertMinor(
          product.balance.bonus_minor,
          product.price_currency,
          currency,
          cnyPerUsd,
        ),
      )
    : null;

  return (
    <Card className="min-h-[260px] rounded-2xl">
      <CardHeader className="p-5 pb-3">
        <CardTitle>{t("store.summary.title")}</CardTitle>
      </CardHeader>
      <CardContent className="flex min-h-[190px] flex-col gap-4 p-5 pt-0">
        <dl className="grid gap-3 text-sm">
          <div className="flex items-start justify-between gap-4">
            <dt className="text-muted-foreground">{t("store.summary.product")}</dt>
            <dd className="text-right font-medium">{product?.name ?? "-"}</dd>
          </div>
          <div className="flex items-start justify-between gap-4">
            <dt className="text-muted-foreground">{t("store.summary.payment")}</dt>
            <dd className="text-right font-medium">{paymentChannel?.name ?? "-"}</dd>
          </div>
          {actualReceivedMinor !== null && (
            <div className="flex items-start justify-between gap-4">
              <dt className="text-muted-foreground">{t("store.summary.actualReceived")}</dt>
              <dd className="text-right font-medium">
                {formatMinor(actualReceivedMinor, currency)}
              </dd>
            </div>
          )}
        </dl>
        <div className="mt-auto flex items-end justify-between gap-4 border-t pt-4">
          <div>
            <p className="text-xs text-muted-foreground">{t("store.summary.total")}</p>
            <p className="text-xl font-semibold">
              {totalMinor === null ? "-" : formatMinor(totalMinor, currency)}
            </p>
          </div>
          <Button
            type="button"
            className="h-11 rounded-xl px-5"
            disabled={!product || !paymentChannel || customAmountInvalid || submitting}
            onClick={onSubmit}
          >
            {submitting && <LoaderCircle className="animate-spin motion-reduce:animate-none" />}
            {t("store.summary.submit")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

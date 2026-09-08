import { useState } from "react";
import { LockKeyhole, RefreshCw, RotateCcw, XCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import type {
  AdminStoreOrderDetail,
  StoreOrder,
  StorePaymentAttempt,
  StoreRefundRecord,
} from "@/lib/store-api";
import { formatMinor } from "@/lib/store-money";
import { canCloseAttempt } from "./order-actions";

interface OrderDialogProps {
  open: boolean;
  detail: AdminStoreOrderDetail | undefined;
  loading: boolean;
  error: unknown;
  actionLoading: string | null;
  onOpenChange: (open: boolean) => void;
  onRetry: () => void;
  onQueryAttempt: (attemptId: string) => Promise<void>;
  onCloseAttempt: (attemptId: string) => Promise<void>;
  onCreateRefund: (currentPassword: string) => Promise<void>;
  onQueryRefund: (refundId: string, currentPassword: string) => Promise<void>;
}

function canQueryAttempt(order: StoreOrder, attempt: StorePaymentAttempt): boolean {
  if (order.contract_version === 1 && order.payment_state === "closed") return false;
  if (attempt.adapter_kind === "http") return false;
  return !(attempt.adapter_kind === "stripe" && attempt.state === "created" && !attempt.provider_object_id);
}

function canCreateRefund(order: StoreOrder): boolean {
  return (
    order.payment_state === "paid"
    && !(order.product_kind === "plan" && order.fulfillment_state === "fulfilled")
  );
}

function canQueryRefund(refund: StoreRefundRecord): boolean {
  return refund.state === "created" || refund.state === "pending";
}

function DetailItem({ label, value, mono = false }: { label: string; value: React.ReactNode; mono?: boolean }) {
  return (
    <div className="min-w-0 rounded-xl border bg-muted/20 p-3">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className={`mt-1 break-words text-sm font-medium ${mono ? "font-mono" : ""}`}>{value}</dd>
    </div>
  );
}

export function OrderDialog({
  open,
  detail,
  loading,
  error,
  actionLoading,
  onOpenChange,
  onRetry,
  onQueryAttempt,
  onCloseAttempt,
  onCreateRefund,
  onQueryRefund,
}: OrderDialogProps) {
  const { t, i18n } = useTranslation();
  const [currentPassword, setCurrentPassword] = useState("");

  const formatDate = (value: string) => new Intl.DateTimeFormat(i18n.language, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));

  const submitRefund = async () => {
    if (!currentPassword || actionLoading) return;
    try {
      await onCreateRefund(currentPassword);
      setCurrentPassword("");
    } catch {
      // Keep the password available so the administrator can correct a failed refund request.
    }
  };

  const queryRefund = async (refundId: string) => {
    if (!currentPassword || actionLoading) return;
    try {
      await onQueryRefund(refundId, currentPassword);
      setCurrentPassword("");
    } catch {
      // The parent reports the API error and the dialog remains ready for another query.
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) setCurrentPassword("");
        onOpenChange(nextOpen);
      }}
    >
      <DialogContent className="flex h-[min(46rem,calc(100dvh-2rem))] min-h-96 w-[calc(100vw-2rem)] max-w-3xl flex-col gap-0 overflow-hidden rounded-2xl p-0">
        <DialogHeader className="shrink-0 border-b px-5 py-4 pr-12 sm:px-6">
          <DialogTitle>{t("store.admin.orders.detailTitle")}</DialogTitle>
          <DialogDescription className="break-all font-mono">
            {detail?.order.order_number ?? t("store.admin.orders.detailDescription")}
          </DialogDescription>
        </DialogHeader>

        {loading ? (
          <div className="grid flex-1 content-start gap-4 p-5 sm:p-6" aria-busy="true">
            <Skeleton className="h-24 rounded-2xl" />
            <Skeleton className="h-36 rounded-2xl" />
            <Skeleton className="h-32 rounded-2xl" />
          </div>
        ) : error || !detail ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
            <p className="text-sm text-muted-foreground">{t("store.admin.orders.detailLoadFailed")}</p>
            <Button type="button" variant="outline" className="min-h-11 rounded-xl" onClick={onRetry}>
              <RefreshCw className="size-4" />
              {t("store.admin.retry")}
            </Button>
          </div>
        ) : (
          <ScrollArea className="min-h-0 flex-1">
            <div className="grid min-w-0 gap-6 p-5 sm:p-6">
              <section className="grid gap-3" aria-labelledby="admin-order-summary-title">
                <h3 id="admin-order-summary-title" className="text-sm font-semibold">
                  {t("store.admin.orders.summary")}
                </h3>
                <dl className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                  <DetailItem label={t("store.admin.orders.number")} value={detail.order.order_number} mono />
                  <DetailItem label={t("store.admin.orders.user")} value={detail.order.user_id} mono />
                  <DetailItem
                    label={t("store.admin.orders.amount")}
                    value={formatMinor(detail.order.payment_minor, detail.order.payment_currency)}
                  />
                  <DetailItem
                    label={t("store.orders.paymentStatus")}
                    value={t(`store.orders.paymentStates.${detail.order.payment_state}`)}
                  />
                  <DetailItem
                    label={t("store.orders.fulfillmentStatus")}
                    value={t(`store.orders.fulfillmentStates.${detail.order.fulfillment_state}`)}
                  />
                  <DetailItem label={t("store.admin.orders.created")} value={formatDate(detail.order.created_at)} />
                </dl>
              </section>

              <section className="grid gap-3" aria-labelledby="admin-order-fulfillment-title">
                <h3 id="admin-order-fulfillment-title" className="text-sm font-semibold">
                  {t("store.admin.orders.fulfillment")}
                </h3>
                <dl className="grid gap-3 sm:grid-cols-2">
                  <DetailItem label={t("store.admin.orders.product")} value={detail.order.quote.product.name} />
                  <DetailItem
                    label={t("store.admin.orders.productKind")}
                    value={t(`store.admin.orders.productKinds.${detail.order.product_kind}`)}
                  />
                  <DetailItem label={t("store.admin.orders.contractVersion")} value={String(detail.order.contract_version)} mono />
                  <DetailItem label={t("store.admin.orders.expires")} value={formatDate(detail.order.expires_at)} />
                </dl>
              </section>

              <section className="grid gap-3" aria-labelledby="admin-order-attempts-title">
                <div>
                  <h3 id="admin-order-attempts-title" className="text-sm font-semibold">
                    {t("store.admin.orders.attempts")}
                  </h3>
                  <p className="text-xs text-muted-foreground">{t("store.admin.orders.attemptsDescription")}</p>
                </div>
                {detail.attempts.length === 0 ? (
                  <p className="rounded-xl border border-dashed p-4 text-sm text-muted-foreground">
                    {t("store.admin.orders.noAttempts")}
                  </p>
                ) : detail.attempts.map((attempt) => (
                  <article key={attempt.id} className="grid min-w-0 gap-3 rounded-2xl border p-4">
                    <div className="flex min-w-0 flex-wrap items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="break-all font-mono text-sm font-medium">{attempt.id}</p>
                        <p className="text-xs text-muted-foreground">
                          {t(`store.admin.channels.kinds.${attempt.adapter_kind}`)} · {formatDate(attempt.created_at)}
                        </p>
                      </div>
                      <Badge variant={attempt.state === "paid" ? "default" : "secondary"}>
                        {t(`store.admin.orders.attemptStates.${attempt.state}`)}
                      </Badge>
                    </div>
                    <dl className="grid gap-2 text-xs sm:grid-cols-2">
                      <DetailItem
                        label={t("store.admin.orders.providerObject")}
                        value={attempt.provider_object_id || t("store.admin.orders.notAvailable")}
                        mono
                      />
                      <DetailItem
                        label={t("store.admin.orders.paymentMethod")}
                        value={attempt.expected_payment_method || t("store.admin.orders.notAvailable")}
                      />
                    </dl>
                    {(canQueryAttempt(detail.order, attempt) || canCloseAttempt(detail.order, detail.attempts, attempt)) && (
                      <div className="flex flex-wrap justify-end gap-2">
                        {canQueryAttempt(detail.order, attempt) && (
                          <Button
                            type="button"
                            variant="outline"
                            className="min-h-11 rounded-xl"
                            disabled={actionLoading !== null}
                            onClick={() => void onQueryAttempt(attempt.id)}
                          >
                            <RefreshCw className={`size-4 ${actionLoading === `query:${attempt.id}` ? "animate-spin" : ""}`} />
                            {t("store.admin.orders.queryAttempt")}
                          </Button>
                        )}
                        {canCloseAttempt(detail.order, detail.attempts, attempt) && (
                          <Button
                            type="button"
                            variant="outline"
                            className="min-h-11 rounded-xl text-destructive hover:text-destructive"
                            disabled={actionLoading !== null}
                            onClick={() => void onCloseAttempt(attempt.id)}
                          >
                            <XCircle className="size-4" />
                            {t("store.admin.orders.closeAttempt")}
                          </Button>
                        )}
                      </div>
                    )}
                  </article>
                ))}
              </section>

              <section className="grid gap-3" aria-labelledby="admin-order-refunds-title">
                <div>
                  <h3 id="admin-order-refunds-title" className="text-sm font-semibold">
                    {t("store.admin.orders.refunds")}
                  </h3>
                  <p className="text-xs text-muted-foreground">{t("store.admin.orders.refundsDescription")}</p>
                </div>
                {detail.refunds.length === 0 ? (
                  <p className="rounded-xl border border-dashed p-4 text-sm text-muted-foreground">
                    {t("store.admin.orders.noRefunds")}
                  </p>
                ) : detail.refunds.map((refund) => (
                  <article key={refund.id} className="flex min-w-0 flex-wrap items-center justify-between gap-3 rounded-2xl border p-4">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <p className="break-all font-mono text-sm font-medium">{refund.id}</p>
                        <Badge variant={refund.state === "succeeded" ? "default" : "secondary"}>
                          {t(`store.admin.orders.refundStates.${refund.state}`)}
                        </Badge>
                      </div>
                      <p className="mt-1 text-sm tabular-nums">
                        {formatMinor(refund.amount_minor, refund.currency)}
                      </p>
                      {refund.provider_refund_id && (
                        <p className="break-all font-mono text-xs text-muted-foreground">
                          {refund.provider_refund_id}
                        </p>
                      )}
                    </div>
                    {canQueryRefund(refund) && (
                      <Button
                        type="button"
                        variant="outline"
                        className="min-h-11 rounded-xl"
                        disabled={!currentPassword || actionLoading !== null}
                        onClick={() => void queryRefund(refund.id)}
                      >
                        <RefreshCw className={`size-4 ${actionLoading === `refund:query:${refund.id}` ? "animate-spin" : ""}`} />
                        {t("store.admin.orders.queryRefund")}
                      </Button>
                    )}
                  </article>
                ))}

                {(canCreateRefund(detail.order) || detail.refunds.some(canQueryRefund)) && (
                  <div className="grid gap-3 rounded-2xl border bg-muted/20 p-4">
                    <div className="grid gap-2">
                      <Label htmlFor="store-refund-password">{t("store.admin.orders.currentPassword")}</Label>
                      <div className="relative">
                        <LockKeyhole className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                        <Input
                          id="store-refund-password"
                          type="password"
                          className="min-h-11 rounded-xl pl-9"
                          value={currentPassword}
                          onChange={(event) => setCurrentPassword(event.target.value)}
                        />
                      </div>
                    </div>
                    {canCreateRefund(detail.order) && (
                      <Button
                        type="button"
                        variant="destructive"
                        className="min-h-11 rounded-xl sm:w-fit"
                        disabled={!currentPassword || actionLoading !== null}
                        onClick={() => void submitRefund()}
                      >
                        <RotateCcw className="size-4" />
                        {actionLoading === "refund:create"
                          ? t("store.admin.orders.refunding")
                          : t("store.admin.orders.createRefund")}
                      </Button>
                    )}
                  </div>
                )}
              </section>
            </div>
          </ScrollArea>
        )}
      </DialogContent>
    </Dialog>
  );
}

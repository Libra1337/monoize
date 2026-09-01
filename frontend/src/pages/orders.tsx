import { useEffect, useState } from "react";
import { AlertCircle, Eye } from "lucide-react";
import { useTranslation } from "react-i18next";
import useSWR, { mutate } from "swr";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { PageHeader } from "@/components/ui/page-header";
import { Skeleton } from "@/components/ui/skeleton";
import { useAuth } from "@/hooks/use-auth";
import { useStoreExchangeRate } from "@/hooks/use-store-exchange-rate";
import { storeApi, type StoreOrder } from "@/lib/store-api";
import { convertMinor } from "@/lib/store-money";
import { isPaymentPollingTerminal } from "./store/checkout-state";

const ORDERS_KEY = "/api/dashboard/store/orders";
const ENTITLEMENT_KEY = "/api/dashboard/store/entitlement";

function OrdersSkeleton() {
  return (
    <div className="grid gap-3" aria-hidden="true">
      {[0, 1, 2].map((item) => (
        <div key={item} className="rounded-2xl border p-5">
          <div className="flex justify-between gap-4">
            <div className="flex-1">
              <Skeleton className="h-5 w-36" />
              <Skeleton className="mt-3 h-4 w-56 max-w-full" />
            </div>
            <Skeleton className="h-8 w-20" />
          </div>
        </div>
      ))}
    </div>
  );
}

export function OrdersPage({ embedded = false }: { embedded?: boolean } = {}) {
  const { t, i18n } = useTranslation();
  const { refreshUser } = useAuth();
  const exchangeRate = useStoreExchangeRate(true);
  const [selectedOrder, setSelectedOrder] = useState<StoreOrder | null>(null);
  const orders = useSWR(ORDERS_KEY, () => storeApi.listOrders(100));
  const formatOrderAmount = (order: StoreOrder) => {
    const cnyRate = exchangeRate.data?.cny_per_usd;
    if (!cnyRate) return "--";
    const cents = order.payment_currency === "USD"
      ? convertMinor(order.payment_minor, "USD", "CNY", cnyRate)
      : order.payment_minor;
    const amount = BigInt(cents);
    return `C${(amount / 100n).toString()}.${(amount % 100n).toString().padStart(2, "0")}`;
  };

  useEffect(() => {
    if (!selectedOrder) return;
    const paymentTerminal = isPaymentPollingTerminal(selectedOrder.payment_state);
    const fulfillmentTerminal = selectedOrder.fulfillment_state === "fulfilled" || selectedOrder.fulfillment_state === "failed";
    if (paymentTerminal || fulfillmentTerminal || Date.parse(selectedOrder.expires_at) <= Date.now()) return;
    let active = true;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const current = await storeApi.getOrder(selectedOrder.id);
        if (!active) return;
        setSelectedOrder(current);
        await mutate<StoreOrder[]>(
          ORDERS_KEY,
          (items = []) => [current, ...items.filter((item) => item.id !== current.id)],
          { revalidate: false },
        );
        const done = isPaymentPollingTerminal(current.payment_state)
          || current.fulfillment_state === "fulfilled"
          || current.fulfillment_state === "failed"
          || Date.parse(current.expires_at) <= Date.now();
        if (done) {
          await Promise.all([refreshUser(), mutate(ENTITLEMENT_KEY), orders.mutate()]);
          return;
        }
      } catch {
        // A later poll retries transient and rate-limit responses.
      }
      if (active) timer = window.setTimeout(() => void poll(), 2_000);
    };
    timer = window.setTimeout(() => void poll(), 2_000);
    return () => {
      active = false;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [orders, refreshUser, selectedOrder]);

  return (
    <div className="flex flex-col gap-6">
      {!embedded && <PageHeader title={t("store.orders.title")} description={t("store.orders.description")} />}

      {orders.isLoading ? (
        <OrdersSkeleton />
      ) : orders.error ? (
        <Card className="rounded-2xl border-destructive/40">
          <CardContent className="flex flex-col items-start gap-4 p-6 sm:flex-row sm:items-center sm:justify-between">
            <p className="flex items-center gap-2 text-sm text-destructive">
              <AlertCircle className="size-4" />
              {t("store.ui.loadFailed")}
            </p>
            <Button
              type="button"
              variant="outline"
              className="h-11 rounded-xl"
              onClick={() => void orders.mutate()}
            >
              {t("store.ui.retry")}
            </Button>
          </CardContent>
        </Card>
      ) : !orders.data?.length ? (
        <Card className="rounded-2xl">
          <CardContent className="p-8 text-center text-sm text-muted-foreground">
            {t("store.orders.empty")}
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-3">
          {orders.data.map((order) => (
            <Card key={order.id} className="rounded-2xl">
              <CardContent className="grid gap-4 p-5 sm:grid-cols-[minmax(0,1.5fr)_minmax(8rem,1fr)_auto] sm:items-center">
                <div className="min-w-0">
                  <p className="truncate font-semibold">{order.quote.product.name}</p>
                  <p className="mt-1 truncate font-mono text-xs text-muted-foreground">
                    {order.order_number}
                  </p>
                </div>
                <div>
                  <p className="font-medium">
                    {formatOrderAmount(order)}
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {new Date(order.created_at).toLocaleString(i18n.language)}
                  </p>
                </div>
                <div className="flex items-center justify-between gap-3 sm:justify-end">
                  <span className="flex flex-wrap justify-end gap-2">
                    <span className="rounded-full bg-muted px-3 py-1 text-xs font-medium">
                      {t(`store.orders.paymentStates.${order.payment_state}`)}
                    </span>
                    <span className="rounded-full border px-3 py-1 text-xs font-medium">
                      {t(`store.orders.fulfillmentStates.${order.fulfillment_state}`)}
                    </span>
                  </span>
                  <Button
                    type="button"
                    variant="ghost"
                    className="h-11 rounded-xl px-3"
                    onClick={() => setSelectedOrder(order)}
                  >
                    <Eye />
                    {t("store.orders.details")}
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      <Dialog open={selectedOrder !== null} onOpenChange={(open) => !open && setSelectedOrder(null)}>
        <DialogContent className="rounded-2xl" closeLabel={t("store.ui.close")}>
          <DialogHeader>
            <DialogTitle>{t("store.orders.details")}</DialogTitle>
            <DialogDescription className="font-mono">
              {selectedOrder?.order_number}
            </DialogDescription>
          </DialogHeader>
          {selectedOrder && (
            <dl className="grid gap-3 text-sm">
              <div className="flex justify-between gap-4 border-b pb-3">
                <dt className="text-muted-foreground">{t("store.orders.product")}</dt>
                <dd className="text-right font-medium">{selectedOrder.quote.product.name}</dd>
              </div>
              <div className="flex justify-between gap-4 border-b pb-3">
                <dt className="text-muted-foreground">{t("store.orders.amount")}</dt>
                <dd className="text-right font-medium">
                  {formatOrderAmount(selectedOrder)}
                </dd>
              </div>
              <div className="flex justify-between gap-4 border-b pb-3">
                <dt className="text-muted-foreground">{t("store.orders.payment")}</dt>
                <dd className="text-right font-medium">
                  {selectedOrder.quote.payment_channel.name}
                </dd>
              </div>
              <div className="flex justify-between gap-4 border-b pb-3">
                <dt className="text-muted-foreground">{t("store.orders.paymentStatus")}</dt>
                <dd className="text-right font-medium">
                  {t(`store.orders.paymentStates.${selectedOrder.payment_state}`)}
                </dd>
              </div>
              <div className="flex justify-between gap-4 border-b pb-3">
                <dt className="text-muted-foreground">{t("store.orders.fulfillmentStatus")}</dt>
                <dd className="text-right font-medium">
                  {t(`store.orders.fulfillmentStates.${selectedOrder.fulfillment_state}`)}
                </dd>
              </div>
              <div className="flex justify-between gap-4">
                <dt className="text-muted-foreground">{t("store.orders.created")}</dt>
                <dd className="text-right font-medium">
                  {new Date(selectedOrder.created_at).toLocaleString(i18n.language)}
                </dd>
              </div>
            </dl>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}

import { useEffect, useMemo, useState } from "react";
import { AlertCircle, Check, Infinity as InfinityIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { QRCodeSVG } from "qrcode.react";
import useSWR, { mutate } from "swr";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { PageHeader } from "@/components/ui/page-header";
import { useAuth } from "@/hooks/use-auth";
import { api } from "@/lib/api";
import {
  storeApi,
  StoreApiError,
  type CreateStoreOrderInput,
  type StoreCheckoutAction,
  type StoreOrder,
  type StorePaymentChannel,
  type StoreProduct,
} from "@/lib/store-api";
import {
  addMinor,
  convertMinor,
  formatMinor,
  formatNanoUsd,
  formatPlanQuota,
  type StoreCurrency,
} from "@/lib/store-money";
import { cn } from "@/lib/utils";
import { OrderSummary } from "./order-summary";
import { PaymentMethods } from "./payment-methods";
import { RedemptionPanel } from "./redemption-panel";
import { selectStoreProduct, validateCustomAmount } from "./store-selection";
import { StoreModeContent } from "./store-mode-content";
import { StoreSkeleton } from "./store-skeleton";
import { StoreTabs, type StoreTab } from "./store-tabs";
import {
  checkoutFingerprint,
  clearPendingCheckout,
  isDefiniteAttemptFailure,
  isPaymentPollingTerminal,
  loadPendingCheckout,
  preparePendingCheckout,
  rotatePendingAttempt,
  savePendingCheckout,
} from "./checkout-state";

const CATALOG_KEY = "/api/dashboard/store/catalog";
const EXCHANGE_RATE_KEY = "/api/dashboard/store/exchange-rate";
const ENTITLEMENT_KEY = "/api/dashboard/store/entitlement";
const ORDERS_KEY = "/api/dashboard/store/orders";
const REDEMPTION_STATUS_KEY = "/api/dashboard/store/redemption-status";

interface RedemptionStatus {
  state: "idle" | "redeeming";
  code: string | null;
}

const IDLE_REDEMPTION_STATUS: RedemptionStatus = { state: "idle", code: null };

function monthStartIso() {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), 1).toISOString();
}

function optimisticOrder(
  product: StoreProduct,
  channel: StorePaymentChannel,
  currency: StoreCurrency,
  paymentMinor: string,
  cnyPerUsd: string,
): StoreOrder {
  const now = new Date().toISOString();
  const fractionalDigits = cnyPerUsd.split(".")[1]?.length ?? 0;
  return {
    id: `optimistic-${Date.now()}`,
    order_number: "...",
    user_id: "",
    product_id: product.id,
    product_kind: product.kind,
    payment_state: "unpaid",
    fulfillment_state: "pending",
    dispute_state: "none",
    payment_hold: false,
    payment_channel_id: channel.id,
    payment_currency: currency,
    payment_minor: paymentMinor,
    cny_per_usd: cnyPerUsd,
    rate_numerator: cnyPerUsd.replace(".", ""),
    rate_denominator: `1${"0".repeat(fractionalDigits)}`,
    quote: {
      version: 2,
      product: {
        id: product.id,
        kind: product.kind,
        name: product.name,
        description: product.description,
        price_currency: product.price_currency,
        price_minor: product.price_minor,
        duration_seconds: product.duration_seconds,
        group_ids: product.group_ids,
        balance: product.balance,
        quotas: product.quotas,
      },
      payment_channel: {
        id: channel.id,
        adapter_kind: channel.adapter_kind,
        name: channel.name,
        icon_kind: channel.icon_kind,
        icon_value: channel.icon_value,
      },
      rate: {
        decimal: cnyPerUsd,
        numerator: cnyPerUsd.replace(".", ""),
        denominator: `1${"0".repeat(fractionalDigits)}`,
        source_updated_at: now,
        refreshed_at: now,
      },
    },
    contract_version: 2,
    state_revision: 0,
    expires_at: new Date(Date.now() + 30 * 60 * 1000).toISOString(),
    created_at: now,
    updated_at: now,
  };
}

function CurrencyControl({
  currency,
  onCurrencyChange,
}: {
  currency: StoreCurrency;
  onCurrencyChange: (currency: StoreCurrency) => void;
}) {
  const { t } = useTranslation();
  return (
    <div
      className="grid grid-cols-2 rounded-xl bg-muted p-1"
      role="group"
      aria-label={t("store.currency.label")}
    >
      {(["CNY", "USD"] as const).map((value) => (
        <button
          key={value}
          type="button"
          aria-pressed={currency === value}
          className={cn(
            "min-h-11 rounded-lg px-4 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            currency === value ? "bg-background text-foreground shadow-sm" : "text-muted-foreground",
          )}
          onClick={() => onCurrencyChange(value)}
        >
          {t(`store.currency.${value.toLowerCase()}`)}
        </button>
      ))}
    </div>
  );
}

function ProductPicker({
  products,
  kind,
  selectedProduct,
  onSelect,
  currency,
  cnyPerUsd,
  customAmount,
  customAmountInvalid,
  customMinimumMinor,
  customMaximumMinor,
  onCustomAmountChange,
}: {
  products: StoreProduct[];
  kind: "balance" | "plan";
  selectedProduct: StoreProduct | null;
  onSelect: (product: StoreProduct) => void;
  currency: StoreCurrency;
  cnyPerUsd: string;
  customAmount: string;
  customAmountInvalid: boolean;
  customMinimumMinor: string;
  customMaximumMinor: string;
  onCustomAmountChange: (amount: string) => void;
}) {
  const { t } = useTranslation();

  return (
    <div className="flex min-w-0 flex-col gap-4">
      {products.length === 0 ? (
        <Card className="rounded-2xl">
          <CardContent className="p-6 text-sm text-muted-foreground">
            {t(kind === "balance" ? "store.ui.balanceEmpty" : "store.ui.planEmpty")}
          </CardContent>
        </Card>
      ) : (
        <div className="grid items-start gap-3 sm:grid-cols-2">
          {products.map((product) => {
            const selected = selectedProduct?.id === product.id;
            const priceMinor = convertMinor(
              product.price_minor,
              product.price_currency,
              currency,
              cnyPerUsd,
            );
            const bonusMinor = product.balance
              ? convertMinor(
                  product.balance.bonus_minor,
                  product.price_currency,
                  currency,
                  cnyPerUsd,
                )
              : null;
            const actualReceivedMinor = product.balance && bonusMinor !== null
              ? addMinor(
                  convertMinor(
                    product.balance.recharge_minor,
                    product.price_currency,
                    currency,
                    cnyPerUsd,
                  ),
                  bonusMinor,
                )
              : null;
            return (
              <button
                key={product.id}
                type="button"
                aria-pressed={selected}
                className={cn(
                  "flex min-h-11 flex-col items-stretch rounded-2xl border bg-card p-5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
                  selected ? "border-foreground" : "hover:border-foreground/30",
                )}
                onClick={() => onSelect(product)}
              >
                <span className="flex items-start justify-between gap-3">
                  <span className="min-w-0">
                    <span className="block font-semibold">{product.name}</span>
                    {product.description && (
                      <span className="mt-1 block text-sm text-muted-foreground">
                        {product.description}
                      </span>
                    )}
                  </span>
                  {selected && <Check className="size-5 shrink-0" />}
                </span>
                <span className="mt-4 text-xl font-semibold">
                  {formatMinor(priceMinor, currency)}
                </span>
                {product.balance && (
                  <span className="mt-3 grid gap-1 text-xs text-muted-foreground">
                    <span>
                      {t("store.balanceProduct.bonus")}: {formatMinor(bonusMinor ?? "0", currency)}
                    </span>
                    <span className="font-medium text-foreground">
                      {t("store.balanceProduct.actualReceived")}: {formatMinor(
                        actualReceivedMinor ?? "0",
                        currency,
                      )}
                    </span>
                  </span>
                )}
                {product.kind === "plan" && (
                  <span className="mt-3 grid gap-1 text-xs text-muted-foreground">
                    {product.quotas.map((quota) => (
                      <span key={quota.id}>
                        {quota.window_kind}: {formatPlanQuota(
                          quota.quota_fen_cny,
                          currency,
                          cnyPerUsd,
                        )}
                      </span>
                    ))}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      )}
      {kind === "balance" && products.length > 0 && (
        <Card className="rounded-2xl">
          <CardContent className="p-5">
            <label htmlFor="store-custom-amount" className="mb-2 block text-sm font-medium">
              {t("store.balanceProduct.custom")}
            </label>
            <div className="relative max-w-sm">
              <span className="pointer-events-none absolute inset-y-0 left-3 flex items-center text-sm text-muted-foreground">
                {currency === "CNY" ? "CNY" : "USD"}
              </span>
              <Input
                id="store-custom-amount"
                inputMode="decimal"
                value={customAmount}
                aria-invalid={customAmountInvalid}
                aria-describedby={customAmountInvalid ? "store-custom-amount-error" : undefined}
                placeholder={t("store.balanceProduct.customPlaceholder")}
                className={cn("h-11 rounded-xl pl-14", customAmountInvalid && "border-destructive")}
                onChange={(event) => onCustomAmountChange(event.target.value)}
              />
            </div>
            {customAmountInvalid && (
              <p id="store-custom-amount-error" className="mt-2 text-sm text-destructive" role="alert">
                {t("store.ui.customAmountInvalid", {
                  minimum: formatMinor(customMinimumMinor, currency),
                  maximum: formatMinor(customMaximumMinor, currency),
                })}
              </p>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  );
}

export function StorePage() {
  const { t } = useTranslation();
  const { user, refreshUser } = useAuth();
  const [activeTab, setActiveTab] = useState<StoreTab>("balance");
  const [currency, setCurrency] = useState<StoreCurrency>("CNY");
  const [selectedProductId, setSelectedProductId] = useState<string | null>(null);
  const [selectedChannelId, setSelectedChannelId] = useState<string | null>(null);
  const [customAmount, setCustomAmount] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [pollingOrderId, setPollingOrderId] = useState<string | null>(() => {
    if (typeof window === "undefined") return null;
    return new URLSearchParams(window.location.search).get("order_id")
      ?? loadPendingCheckout(window.sessionStorage)?.orderId
      ?? null;
  });
  const [qrAction, setQrAction] = useState<Extract<StoreCheckoutAction, { kind: "qr" }> | null>(null);
  const monthStart = useMemo(monthStartIso, []);

  const catalog = useSWR(CATALOG_KEY, storeApi.getCatalog);
  const exchangeRate = useSWR(EXCHANGE_RATE_KEY, storeApi.getExchangeRate);
  const entitlement = useSWR(ENTITLEMENT_KEY, storeApi.getEntitlement);
  const mutateEntitlement = entitlement.mutate;
  const redemptionStatus = useSWR<RedemptionStatus>(REDEMPTION_STATUS_KEY, null, {
    fallbackData: IDLE_REDEMPTION_STATUS,
  });
  const monthlyUsage = useSWR(
    ["store-monthly-usage", monthStart],
    () => api.listRequestLogs(1, 0, { time_from: monthStart }),
  );

  const loading = catalog.isLoading || exchangeRate.isLoading || entitlement.isLoading || monthlyUsage.isLoading;
  const error = catalog.error || exchangeRate.error || entitlement.error || monthlyUsage.error;
  const rate = exchangeRate.data?.cny_per_usd ?? "1";
  const products = (catalog.data?.products ?? []).filter(
    (product) => product.enabled && product.kind === activeTab,
  );
  const selectedProduct = activeTab === "redeem"
    ? null
    : selectStoreProduct(catalog.data?.products ?? [], activeTab, selectedProductId);
  const channels = catalog.data?.payment_channels ?? [];
  const selectedChannel = channels.find((channel) => channel.id === selectedChannelId)
    ?? channels.find((channel) => channel.enabled)
    ?? null;
  const settings = catalog.data?.settings;
  const customValidation = settings && activeTab === "balance"
    ? validateCustomAmount(customAmount, currency, catalog.data?.settings)
    : { hasCustomAmount: false, minor: null, invalid: false };
  const customRechargeMinor = customValidation.hasCustomAmount
    ? customValidation.minor
    : null;
  const customAmountInvalid = customValidation.invalid;
  const redeeming = redemptionStatus.data?.state === "redeeming";
  const customMinimumMinor = currency === "CNY"
    ? settings?.custom_recharge_cny_min_minor ?? "0"
    : settings?.custom_recharge_usd_min_minor ?? "0";
  const customMaximumMinor = currency === "CNY"
    ? settings?.custom_recharge_cny_max_minor ?? "0"
    : settings?.custom_recharge_usd_max_minor ?? "0";

  const retry = () => Promise.all([
    catalog.mutate(),
    exchangeRate.mutate(),
    entitlement.mutate(),
    monthlyUsage.mutate(),
  ]);

  useEffect(() => {
    if (!pollingOrderId) return;
    let active = true;
    let timer: number | undefined;

    const poll = async () => {
      try {
        const order = await storeApi.getOrder(pollingOrderId);
        await mutate<StoreOrder[]>(
          ORDERS_KEY,
          (current = []) => [order, ...current.filter((item) => item.id !== order.id)],
          { revalidate: false },
        );
        const paymentTerminal = isPaymentPollingTerminal(order.payment_state);
        const fulfillmentTerminal = order.fulfillment_state === "fulfilled" || order.fulfillment_state === "failed";
        const expired = Date.parse(order.expires_at) <= Date.now();
        if (paymentTerminal || fulfillmentTerminal || expired) {
          clearPendingCheckout(window.sessionStorage, order.id);
          await Promise.all([mutate(ORDERS_KEY), refreshUser(), mutateEntitlement()]);
          if (active) {
            setPollingOrderId(null);
            setQrAction(null);
            const url = new URL(window.location.href);
            url.searchParams.delete("order_id");
            url.searchParams.delete("checkout");
            window.history.replaceState(null, "", `${url.pathname}${url.search}${url.hash}`);
          }
          return;
        }
      } catch (cause) {
        if (cause instanceof StoreApiError && cause.status === 404) {
          clearPendingCheckout(window.sessionStorage, pollingOrderId);
          if (active) setPollingOrderId(null);
          return;
        }
      }
      if (active) timer = window.setTimeout(() => void poll(), 2_000);
    };

    void poll();
    return () => {
      active = false;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [mutateEntitlement, pollingOrderId, refreshUser]);

  const handleTabChange = (tab: StoreTab) => {
    setActiveTab(tab);
    setSelectedProductId(null);
    setCustomAmount("");
  };

  const handleCreateOrder = async () => {
    if (!user || !selectedProduct || !selectedChannel || customAmountInvalid) return;
    const paymentMinor = customValidation.hasCustomAmount
      ? customRechargeMinor
      : convertMinor(
          selectedProduct.price_minor,
          selectedProduct.price_currency,
          currency,
          rate,
        );
    if (paymentMinor === null) return;
    const pending = optimisticOrder(
      selectedProduct,
      selectedChannel,
      currency,
      paymentMinor,
      rate,
    );
    const request: CreateStoreOrderInput = {
      product_id: selectedProduct.id,
      payment_channel_id: selectedChannel.id,
      payment_currency: currency,
      custom_recharge_minor: customRechargeMinor,
    };
    const pendingCheckout = preparePendingCheckout(
      window.sessionStorage,
      checkoutFingerprint(user.id, request),
    );
    setSubmitting(true);
    try {
      const updatedOrders = await mutate<StoreOrder[]>(
        ORDERS_KEY,
        async (current = []) => {
          const created = pendingCheckout.orderId
            ? await storeApi.getOrder(pendingCheckout.orderId)
            : await storeApi.createOrder(request, pendingCheckout.orderIdempotencyKey);
          if (!pendingCheckout.orderId) {
            pendingCheckout.orderId = created.id;
            savePendingCheckout(window.sessionStorage, pendingCheckout);
          }
          return [created, ...current.filter((order) => order.id !== pending.id)];
        },
        {
          optimisticData: (current = []) => [pending, ...current],
          rollbackOnError: true,
          revalidate: false,
        },
      );
      const createdOrder = updatedOrders?.[0];
      if (!createdOrder) throw new Error(t("store.ui.orderFailed"));
      const mobileCheckout = window.matchMedia("(max-width: 767px)").matches;
      const expectedPaymentMethod = selectedChannel.adapter_kind === "wechat"
        ? (mobileCheckout ? "h5" : "native")
        : selectedChannel.adapter_kind === "alipay"
          ? (mobileCheckout ? "mobile_web" : "computer_web")
        : {
            stripe: "card",
            http: null,
          }[selectedChannel.adapter_kind];
      const checkout = await storeApi.createPaymentAttempt(
        createdOrder.id,
        pendingCheckout.attemptIdempotencyKey,
        expectedPaymentMethod,
      );
      if (checkout.action.kind === "redirect") {
        window.location.assign(checkout.action.url);
        return;
      }
      if (checkout.action.kind === "form") {
        const form = document.createElement("form");
        form.method = "POST";
        form.action = checkout.action.action;
        for (const [name, value] of checkout.action.fields) {
          const input = document.createElement("input");
          input.type = "hidden";
          input.name = name;
          input.value = value;
          form.append(input);
        }
        document.body.append(form);
        form.submit();
        return;
      }
      setQrAction(checkout.action);
      setPollingOrderId(createdOrder.id);
    } catch (cause) {
      if (
        cause instanceof StoreApiError
        && cause.code !== "payment_provider_ambiguous"
        && pendingCheckout.orderId
      ) {
        if (cause.status === 404) {
          clearPendingCheckout(window.sessionStorage, pendingCheckout.orderId);
        } else if (isDefiniteAttemptFailure(cause.code)) {
          rotatePendingAttempt(window.sessionStorage, pendingCheckout);
        }
      }
      toast.error(cause instanceof Error ? cause.message : t("store.ui.orderFailed"));
    } finally {
      setSubmitting(false);
    }
  };

  const handleRedeem = async (code: string) => {
    await redemptionStatus.mutate(
      async () => {
        await storeApi.redeem(code);
        await Promise.all([refreshUser(), entitlement.mutate()]);
        return IDLE_REDEMPTION_STATUS;
      },
      {
        optimisticData: { state: "redeeming", code },
        rollbackOnError: true,
        revalidate: false,
      },
    );
  };

  return (
    <div className="flex flex-col gap-6">
      <PageHeader title={t("store.title")} description={t("store.description")} />
      <Dialog open={qrAction !== null} onOpenChange={(open) => !open && setQrAction(null)}>
        <DialogContent className="max-w-sm rounded-2xl" closeLabel={t("store.ui.close")}>
          <DialogHeader>
            <DialogTitle>{t("store.payment.qrTitle")}</DialogTitle>
            <DialogDescription>{t("store.payment.qrDescription")}</DialogDescription>
          </DialogHeader>
          <div className="mx-auto grid size-[252px] place-items-center rounded-2xl border bg-white p-4 shadow-sm">
            {qrAction && (
              <QRCodeSVG
                value={qrAction.payload}
                size={220}
                level="M"
                marginSize={1}
                bgColor="#ffffff"
                fgColor="#111111"
                title={t("store.payment.qrTitle")}
              />
            )}
          </div>
        </DialogContent>
      </Dialog>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <StoreTabs activeTab={activeTab} onTabChange={handleTabChange} />
        {activeTab !== "redeem" && (
          <CurrencyControl currency={currency} onCurrencyChange={setCurrency} />
        )}
      </div>

      {loading ? (
        <StoreSkeleton />
      ) : error ? (
        <Card className="rounded-2xl border-destructive/40">
          <CardContent className="flex flex-col items-start gap-4 p-6 sm:flex-row sm:items-center sm:justify-between">
            <p className="flex items-center gap-2 text-sm text-destructive">
              <AlertCircle className="size-4" />
              {t("store.ui.loadFailed")}
            </p>
            <Button type="button" variant="outline" className="h-11 rounded-xl" onClick={() => void retry()}>
              {t("store.ui.retry")}
            </Button>
          </CardContent>
        </Card>
      ) : (
        <>
          <section className="grid gap-3 sm:grid-cols-3" aria-label={t("store.ui.accountLabel")}>
            <Card className="rounded-2xl">
              <CardContent className="p-5">
                <p className="text-sm text-muted-foreground">{t("store.account.balance")}</p>
                <p className="mt-2 text-xl font-semibold">
                  {user?.balance_unlimited ? (
                    <span className="flex items-center gap-2"><InfinityIcon className="size-5" />{t("store.ui.accountUnlimited")}</span>
                  ) : formatNanoUsd(user?.balance_nano_usd ?? "0", currency, rate)}
                </p>
              </CardContent>
            </Card>
            <Card className="rounded-2xl">
              <CardContent className="p-5">
                <p className="text-sm text-muted-foreground">{t("store.account.monthlyUsage")}</p>
                <p className="mt-2 text-xl font-semibold">
                  {formatNanoUsd(monthlyUsage.data?.total_charge_nano_usd ?? "0", currency, rate)}
                </p>
              </CardContent>
            </Card>
            <Card className="rounded-2xl">
              <CardContent className="p-5">
                <p className="text-sm text-muted-foreground">{t("store.account.currentPlan")}</p>
                <p className="mt-2 font-semibold">
                  {entitlement.data?.product_name ?? t("store.account.noPlan")}
                </p>
                {entitlement.data && (
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t("store.ui.accountEnds", { date: new Date(entitlement.data.ends_at).toLocaleDateString() })}
                  </p>
                )}
              </CardContent>
            </Card>
          </section>

          <StoreModeContent
            activeTab={activeTab}
            purchaseContent={(
              <>
              <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1fr)_360px]">
                <ProductPicker
                  products={products}
                  kind={activeTab}
                  selectedProduct={selectedProduct}
                  onSelect={(product) => setSelectedProductId(product.id)}
                  currency={currency}
                  cnyPerUsd={rate}
                  customAmount={customAmount}
                  customAmountInvalid={customAmountInvalid}
                  customMinimumMinor={customMinimumMinor}
                  customMaximumMinor={customMaximumMinor}
                  onCustomAmountChange={setCustomAmount}
                />
                <OrderSummary
                  product={selectedProduct}
                  paymentChannel={selectedChannel}
                  currency={currency}
                  cnyPerUsd={rate}
                  customRechargeMinor={customRechargeMinor}
                  customAmountInvalid={customAmountInvalid}
                  submitting={submitting}
                  onSubmit={handleCreateOrder}
                />
              </div>
              <PaymentMethods
                channels={channels}
                selectedId={selectedChannel?.id ?? null}
                onSelect={(channel) => setSelectedChannelId(channel.id)}
              />
              </>
            )}
            redemptionContent={(
              <RedemptionPanel
                onRedeem={handleRedeem}
                redeeming={redeeming}
              />
            )}
          />
        </>
      )}
    </div>
  );
}

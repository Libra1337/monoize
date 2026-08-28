import { useState } from "react";
import { SiAlipay, SiStripe, SiWechat } from "@icons-pack/react-simple-icons";
import { CreditCard, ClipboardCheck, Eye, FileCheck2, Layers3, Package, Pencil, Plus, ReceiptText, ShieldCheck, TicketCheck, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import type {
  RedemptionCodeRecord,
  StoreOrder,
  StorePaymentChannel,
  StoreProduct,
} from "@/lib/store-api";
import { formatMinor } from "@/lib/store-money";

interface LoadStateProps {
  loading: boolean;
  error: unknown;
  onRetry: () => void;
  children: React.ReactNode;
}

export function AdminLoadState({ loading, error, onRetry, children }: LoadStateProps) {
  const { t } = useTranslation();
  if (loading) {
    return (
      <div className="grid gap-3" aria-busy="true">
        <Skeleton className="h-20 rounded-2xl" />
        <Skeleton className="h-20 rounded-2xl" />
        <Skeleton className="h-20 rounded-2xl" />
      </div>
    );
  }
  if (error) {
    return (
      <div className="flex min-h-44 flex-col items-center justify-center gap-3 rounded-2xl border border-dashed p-6 text-center">
        <p className="text-sm text-muted-foreground">{t("store.admin.loadFailed")}</p>
        <Button type="button" variant="outline" className="rounded-xl" onClick={onRetry}>
          {t("store.admin.retry")}
        </Button>
      </div>
    );
  }
  return <>{children}</>;
}

function EmptyPanel({ icon, title }: { icon: React.ReactNode; title: string }) {
  return (
    <div className="flex min-h-44 flex-col items-center justify-center gap-3 rounded-2xl border border-dashed p-6 text-center text-muted-foreground">
      {icon}
      <p className="text-sm">{title}</p>
    </div>
  );
}

export function ProductsPanel({
  products,
  onCreate,
  onEdit,
  onDelete,
}: {
  products: StoreProduct[];
  onCreate: () => void;
  onEdit: (product: StoreProduct) => void;
  onDelete: (product: StoreProduct) => void;
}) {
  const { t } = useTranslation();
  return (
    <section className="grid gap-4" aria-labelledby="store-admin-products-title">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 id="store-admin-products-title" className="text-lg font-semibold">
            {t("store.admin.products.title")}
          </h2>
          <p className="text-sm text-muted-foreground">
            {t("store.admin.products.descriptionText")}
          </p>
        </div>
        <Button type="button" className="min-h-11 rounded-xl" onClick={onCreate}>
          <Plus className="size-4" />
          {t("store.admin.products.create")}
        </Button>
      </div>
      {products.length === 0 ? (
        <EmptyPanel icon={<Package className="size-8" />} title={t("store.admin.products.empty")} />
      ) : (
        <div className="grid gap-3 md:grid-cols-2">
          {products.map((product) => (
            <article key={product.id} className="flex min-h-32 flex-col gap-4 rounded-2xl border bg-card p-4">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <h3 className="truncate font-semibold">{product.name}</h3>
                    <Badge variant={product.enabled ? "default" : "secondary"}>
                      {t(product.enabled ? "store.admin.enabled" : "store.admin.disabled")}
                    </Badge>
                    <Badge variant="outline">{t(`store.admin.products.kinds.${product.kind}`)}</Badge>
                  </div>
                  <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">
                    {product.description || t("store.admin.noDescription")}
                  </p>
                </div>
                <span className="shrink-0 font-semibold tabular-nums">
                  {formatMinor(product.price_minor, product.price_currency)}
                </span>
              </div>
              <div className="mt-auto flex items-center justify-end gap-1">
                <Button type="button" variant="ghost" size="icon" className="size-11 rounded-xl" aria-label={t("common.edit")} onClick={() => onEdit(product)}>
                  <Pencil className="size-4" />
                </Button>
                <Button type="button" variant="ghost" size="icon" className="size-11 rounded-xl" aria-label={t("common.delete")} onClick={() => onDelete(product)}>
                  <Trash2 className="size-4 text-destructive" />
                </Button>
              </div>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

function ChannelIcon({ channel }: { channel: StorePaymentChannel }) {
  const [failedIconValue, setFailedIconValue] = useState<string | null>(null);
  if (channel.icon_kind !== "builtin" && channel.icon_value && channel.icon_value !== failedIconValue) {
    return <img src={channel.icon_value} alt="" className="size-7 rounded-lg object-contain" onError={() => setFailedIconValue(channel.icon_value)} />;
  }
  if (channel.adapter_kind === "alipay") return <SiAlipay className="size-6 text-[#1677ff]" />;
  if (channel.adapter_kind === "wechat") return <SiWechat className="size-6 text-[#07c160]" />;
  if (channel.adapter_kind === "stripe") return <SiStripe className="size-6 text-[#635bff]" />;
  return <CreditCard className="size-6 text-muted-foreground" />;
}

export function ChannelsPanel({
  channels,
  onCreate,
  onPrivacyRecords,
  onRetention,
  onCompliance,
  onCapabilities,
  onReadiness,
  onEdit,
  onDelete,
}: {
  channels: StorePaymentChannel[];
  onCreate: () => void;
  onPrivacyRecords: () => void;
  onRetention: () => void;
  onCompliance: (channel: StorePaymentChannel) => void;
  onCapabilities: (channel: StorePaymentChannel) => void;
  onReadiness: (channel: StorePaymentChannel) => void;
  onEdit: (channel: StorePaymentChannel) => void;
  onDelete: (channel: StorePaymentChannel) => void;
}) {
  const { t } = useTranslation();
  return (
    <section className="grid gap-4" aria-labelledby="store-admin-channels-title">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 id="store-admin-channels-title" className="text-lg font-semibold">
            {t("store.admin.channels.title")}
          </h2>
          <p className="text-sm text-muted-foreground">{t("store.admin.channels.descriptionText")}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" variant="outline" className="min-h-11 rounded-xl" onClick={onPrivacyRecords}>
            <FileCheck2 className="size-4" />
            {t("store.admin.governance.privacyRecords.action")}
          </Button>
          <Button type="button" variant="outline" className="min-h-11 rounded-xl" onClick={onRetention}>
            <ShieldCheck className="size-4" />
            {t("store.admin.governance.retention.action")}
          </Button>
          <Button type="button" className="min-h-11 rounded-xl" onClick={onCreate}>
            <Plus className="size-4" />
            {t("store.admin.channels.create")}
          </Button>
        </div>
      </div>
      {channels.length === 0 ? (
        <EmptyPanel icon={<CreditCard className="size-8" />} title={t("store.admin.channels.empty")} />
      ) : (
        <div className="grid gap-3">
          {channels.map((channel) => (
            <article key={channel.id} className="flex flex-wrap items-center gap-4 rounded-2xl border bg-card p-4">
              <div className="flex size-11 items-center justify-center rounded-xl bg-muted">
                <ChannelIcon channel={channel} />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="font-semibold">{channel.name}</h3>
                </div>
                <p className="truncate text-sm text-muted-foreground">
                  {t(`store.admin.channels.kinds.${channel.adapter_kind}`)}
                </p>
                <div className="mt-2 flex flex-wrap items-center gap-3 text-xs">
                  <span className="flex items-center gap-2 text-muted-foreground">
                    {t("store.admin.channelAvailability.configuredState")}
                    <Badge variant={channel.enabled ? "outline" : "secondary"}>
                      {t(channel.enabled ? "store.admin.enabled" : "store.admin.disabled")}
                    </Badge>
                  </span>
                  <span className="flex items-center gap-2 text-muted-foreground">
                    {t("store.admin.channelAvailability.effectiveState")}
                    <Badge variant={channel.effective_available ? "default" : "secondary"}>
                      {t(channel.effective_available
                        ? "store.admin.channelAvailability.available"
                        : "store.admin.channelAvailability.unavailable")}
                    </Badge>
                  </span>
                </div>
                {!channel.effective_available && channel.unavailable_reasons.length > 0 && (
                  <div className="mt-2 flex min-w-0 flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <span>{t("store.admin.channelAvailability.unavailableReasons")}</span>
                    {[...channel.unavailable_reasons].sort().map((reason) => (
                      <code key={reason} className="max-w-full break-all rounded-md bg-muted px-2 py-1">
                        {reason}
                      </code>
                    ))}
                  </div>
                )}
              </div>
              <div className="flex items-center gap-1">
                {channel.adapter_kind !== "http" && (
                  <>
                    <Button type="button" variant="ghost" size="icon" className="size-11 rounded-xl" aria-label={t("store.admin.governance.compliance.action")} onClick={() => onCompliance(channel)}>
                      <ClipboardCheck className="size-4" />
                    </Button>
                    <Button type="button" variant="ghost" size="icon" className="size-11 rounded-xl" aria-label={t("store.admin.governance.capabilities.action")} onClick={() => onCapabilities(channel)}>
                      <Layers3 className="size-4" />
                    </Button>
                    <Button type="button" variant="ghost" size="icon" className="size-11 rounded-xl" aria-label={t("store.admin.governance.readiness.action")} onClick={() => onReadiness(channel)}>
                      <ShieldCheck className="size-4" />
                    </Button>
                  </>
                )}
                <Button type="button" variant="ghost" size="icon" className="size-11 rounded-xl" aria-label={t("common.edit")} onClick={() => onEdit(channel)}>
                  <Pencil className="size-4" />
                </Button>
                <Button type="button" variant="ghost" size="icon" className="size-11 rounded-xl" aria-label={t("common.delete")} onClick={() => onDelete(channel)}>
                  <Trash2 className="size-4 text-destructive" />
                </Button>
              </div>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

export function OrdersPanel({
  orders,
  onSelectOrder,
}: {
  orders: StoreOrder[];
  onSelectOrder: (orderId: string) => void;
}) {
  const { t, i18n } = useTranslation();
  return (
    <section className="grid gap-4" aria-labelledby="store-admin-orders-title">
      <div>
        <h2 id="store-admin-orders-title" className="text-lg font-semibold">
          {t("store.admin.orders.title")}
        </h2>
        <p className="text-sm text-muted-foreground">{t("store.admin.orders.descriptionText")}</p>
      </div>
      {orders.length === 0 ? (
        <EmptyPanel icon={<ReceiptText className="size-8" />} title={t("store.admin.orders.empty")} />
      ) : (
        <div className="overflow-x-auto rounded-2xl border">
          <table className="w-full min-w-[56rem] text-sm">
            <thead className="bg-muted/50 text-left text-muted-foreground">
              <tr>
                <th className="px-4 py-3 font-medium">{t("store.admin.orders.number")}</th>
                <th className="px-4 py-3 font-medium">{t("store.admin.orders.user")}</th>
                <th className="px-4 py-3 font-medium">{t("store.admin.orders.amount")}</th>
                <th className="px-4 py-3 font-medium">{t("store.orders.paymentStatus")}</th>
                <th className="px-4 py-3 font-medium">{t("store.orders.fulfillmentStatus")}</th>
                <th className="px-4 py-3 font-medium">{t("store.admin.orders.created")}</th>
              </tr>
            </thead>
            <tbody>
              {orders.map((order) => (
                <tr key={order.id} className="border-t transition-colors hover:bg-muted/30">
                  <td className="px-4 py-2 font-medium">
                    <div className="flex min-w-44 items-center justify-between gap-2">
                      <span>{order.order_number}</span>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="size-11 shrink-0 rounded-xl"
                        aria-label={t("store.admin.orders.viewDetail", { number: order.order_number })}
                        onClick={() => onSelectOrder(order.id)}
                      >
                        <Eye className="size-4" />
                      </Button>
                    </div>
                  </td>
                  <td className="px-4 py-3 text-muted-foreground">{order.user_id}</td>
                  <td className="px-4 py-3 tabular-nums">{formatMinor(order.payment_minor, order.payment_currency)}</td>
                  <td className="px-4 py-3">
                    <Badge variant={order.payment_state === "paid" ? "default" : "outline"}>
                      {t(`store.orders.paymentStates.${order.payment_state}`)}
                    </Badge>
                  </td>
                  <td className="px-4 py-3">
                    <Badge variant={order.fulfillment_state === "fulfilled" ? "default" : "secondary"}>
                      {t(`store.orders.fulfillmentStates.${order.fulfillment_state}`)}
                    </Badge>
                  </td>
                  <td className="px-4 py-3 text-muted-foreground">
                    {new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(new Date(order.created_at))}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function rewardLabel(code: RedemptionCodeRecord, t: (key: string) => string): string {
  const reward = code.reward as {
    kind?: string;
    currency?: string;
    amount_minor?: string;
    product_name?: string;
  } | null;
  if (reward?.kind === "balance" && reward.currency && reward.amount_minor) {
    return formatMinor(reward.amount_minor, reward.currency as "CNY" | "USD");
  }
  return reward?.product_name || t(`store.admin.redemptions.rewardKinds.${code.reward_kind}`);
}

export function RedemptionsPanel({ codes, onGenerate }: { codes: RedemptionCodeRecord[]; onGenerate: () => void }) {
  const { t, i18n } = useTranslation();
  return (
    <section className="grid gap-4" aria-labelledby="store-admin-redemptions-title">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 id="store-admin-redemptions-title" className="text-lg font-semibold">
            {t("store.admin.redemptions.title")}
          </h2>
          <p className="text-sm text-muted-foreground">{t("store.admin.redemptions.descriptionText")}</p>
        </div>
        <Button type="button" className="min-h-11 rounded-xl" onClick={onGenerate}>
          <Plus className="size-4" />
          {t("store.admin.redemptions.generate")}
        </Button>
      </div>
      {codes.length === 0 ? (
        <EmptyPanel icon={<TicketCheck className="size-8" />} title={t("store.admin.redemptions.empty")} />
      ) : (
        <div className="overflow-x-auto rounded-2xl border">
          <table className="w-full min-w-[44rem] text-sm">
            <thead className="bg-muted/50 text-left text-muted-foreground">
              <tr>
                <th className="px-4 py-3 font-medium">{t("store.admin.redemptions.hint")}</th>
                <th className="px-4 py-3 font-medium">{t("store.admin.redemptions.reward")}</th>
                <th className="px-4 py-3 font-medium">{t("store.admin.redemptions.status")}</th>
                <th className="px-4 py-3 font-medium">{t("store.admin.redemptions.expires")}</th>
              </tr>
            </thead>
            <tbody>
              {codes.map((code) => (
                <tr key={code.id} className="border-t">
                  <td className="px-4 py-3 font-mono font-medium">****-{code.code_hint}</td>
                  <td className="px-4 py-3">{rewardLabel(code, t)}</td>
                  <td className="px-4 py-3">
                    <Badge variant={code.status === "unused" ? "default" : "secondary"}>
                      {t(`store.admin.redemptions.statuses.${code.status}`)}
                    </Badge>
                  </td>
                  <td className="px-4 py-3 text-muted-foreground">
                    {new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(new Date(code.expires_at))}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

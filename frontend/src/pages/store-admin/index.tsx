import { useState } from "react";
import { Save } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import useSWR from "swr";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { PageHeader } from "@/components/ui/page-header";
import { api } from "@/lib/api";
import {
  storeApi,
  type GenerateRedemptionCodesInput,
  type PaymentChannelInput,
  type PaymentCredentialPayload,
  type StorePaymentChannel,
  type StoreProduct,
  type StoreProductInput,
  type StoreSettings,
} from "@/lib/store-api";
import { addMinor, decimalToMinor, minorToDecimal } from "@/lib/store-money";
import {
  AdminLoadState,
  ChannelsPanel,
  OrdersPanel,
  ProductsPanel,
  RedemptionsPanel,
} from "./admin-panels";
import { ChannelDialog } from "./channel-dialog";
import { ProductDialog } from "./product-dialog";
import { RedemptionDialog } from "./redemption-dialog";
import { StoreAdminTabs, type StoreAdminTab } from "./store-admin-tabs";

const PRODUCTS_KEY = "/api/dashboard/store/admin/products";
const CHANNELS_KEY = "/api/dashboard/store/admin/payment-channels";
const ORDERS_KEY = "/api/dashboard/store/admin/orders";
const REDEMPTIONS_KEY = "/api/dashboard/store/admin/redemption-codes";
const SETTINGS_KEY = "/api/dashboard/store/admin/settings";
const GROUPS_KEY = "/api/dashboard/groups";

type DeleteTarget =
  | { kind: "product"; record: StoreProduct }
  | { kind: "channel"; record: StorePaymentChannel };

function optimisticProduct(input: StoreProductInput, id: string): StoreProduct {
  const now = new Date().toISOString();
  return {
    ...input,
    id,
    created_at: now,
    updated_at: now,
    balance: input.balance ? { ...input.balance, actual_received_minor: addMinor(input.balance.recharge_minor, input.balance.bonus_minor) } : null,
    quotas: input.quotas.map((quota, index) => ({ ...quota, id: `${id}-quota-${index}` })),
  };
}

function optimisticChannel(input: PaymentChannelInput, id: string): StorePaymentChannel {
  const now = new Date().toISOString();
  return { ...input, id, revision: 1, created_at: now, updated_at: now };
}

function SettingsPanel({ settings, saving, onSave }: { settings: StoreSettings; saving: boolean; onSave: (settings: StoreSettings) => Promise<void> }) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState({
    cnyMin: minorToDecimal(settings.custom_recharge_cny_min_minor),
    cnyMax: minorToDecimal(settings.custom_recharge_cny_max_minor),
    usdMin: minorToDecimal(settings.custom_recharge_usd_min_minor),
    usdMax: minorToDecimal(settings.custom_recharge_usd_max_minor),
  });
  const [error, setError] = useState<string | null>(null);
  const submit = async () => {
    const values = [draft.cnyMin, draft.cnyMax, draft.usdMin, draft.usdMax].map(decimalToMinor);
    if (values.some((value) => value === null || value === "0") || BigInt(values[0]!) > BigInt(values[1]!) || BigInt(values[2]!) > BigInt(values[3]!)) {
      setError(t("store.admin.settings.invalid"));
      return;
    }
    setError(null);
    await onSave({ custom_recharge_cny_min_minor: values[0]!, custom_recharge_cny_max_minor: values[1]!, custom_recharge_usd_min_minor: values[2]!, custom_recharge_usd_max_minor: values[3]! });
  };
  const fields = [
    ["cnyMin", "store.admin.settings.cnyMin"], ["cnyMax", "store.admin.settings.cnyMax"],
    ["usdMin", "store.admin.settings.usdMin"], ["usdMax", "store.admin.settings.usdMax"],
  ] as const;
  return <section className="grid gap-4 border-t pt-6" aria-labelledby="store-settings-title"><div><h2 id="store-settings-title" className="text-lg font-semibold">{t("store.admin.settings.title")}</h2><p className="text-sm text-muted-foreground">{t("store.admin.settings.description")}</p></div><div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">{fields.map(([key, label]) => <div key={key} className="grid gap-2"><Label htmlFor={`store-${key}`}>{t(label)}</Label><Input id={`store-${key}`} className="min-h-11 rounded-xl" inputMode="decimal" value={draft[key]} onChange={(event) => setDraft((current) => ({ ...current, [key]: event.target.value }))} /></div>)}</div>{error && <p className="text-sm text-destructive" role="alert">{error}</p>}<Button type="button" className="min-h-11 w-fit rounded-xl" disabled={saving} onClick={() => void submit()}><Save className="h-4 w-4" />{saving ? t("common.loading") : t("common.save")}</Button></section>;
}

export function StoreAdminPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<StoreAdminTab>("products");
  const [productDialogOpen, setProductDialogOpen] = useState(false);
  const [selectedProduct, setSelectedProduct] = useState<StoreProduct | null>(null);
  const [channelDialogOpen, setChannelDialogOpen] = useState(false);
  const [selectedChannel, setSelectedChannel] = useState<StorePaymentChannel | null>(null);
  const [redemptionDialogOpen, setRedemptionDialogOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<DeleteTarget | null>(null);

  const products = useSWR(PRODUCTS_KEY, storeApi.admin.listProducts);
  const channels = useSWR(CHANNELS_KEY, storeApi.admin.listPaymentChannels);
  const orders = useSWR(ORDERS_KEY, () => storeApi.admin.listOrders(100));
  const redemptions = useSWR(REDEMPTIONS_KEY, () => storeApi.admin.listRedemptionCodes(100));
  const settings = useSWR(SETTINGS_KEY, storeApi.admin.getSettings);
  const groups = useSWR(GROUPS_KEY, () => api.listDashboardGroups());

  const saveProduct = async (input: StoreProductInput) => {
    setSaving(true);
    const target = selectedProduct;
    const optimistic = optimisticProduct(input, target?.id ?? `optimistic-${Date.now()}`);
    try {
      await products.mutate(async (current = []) => {
        const saved = target ? await storeApi.admin.updateProduct(target.id, input) : await storeApi.admin.createProduct(input);
        return target ? current.map((item) => item.id === target.id ? saved : item) : [...current.filter((item) => item.id !== optimistic.id), saved];
      }, { optimisticData: (current = []) => target ? current.map((item) => item.id === target.id ? { ...optimistic, created_at: item.created_at } : item) : [...current, optimistic], rollbackOnError: true, revalidate: false });
      toast.success(t("store.admin.saved"));
      setProductDialogOpen(false);
    } catch (cause) { toast.error(cause instanceof Error ? cause.message : t("common.error")); } finally { setSaving(false); }
  };

  const deleteProduct = async (product: StoreProduct) => {
    try {
      await products.mutate(async (current = []) => { await storeApi.admin.deleteProduct(product.id); return current.filter((item) => item.id !== product.id); }, { optimisticData: (current = []) => current.filter((item) => item.id !== product.id), rollbackOnError: true, revalidate: false });
      toast.success(t("store.admin.deleted"));
    } catch (cause) { toast.error(cause instanceof Error ? cause.message : t("common.error")); }
  };

  const saveChannel = async (input: PaymentChannelInput) => {
    setSaving(true);
    const target = selectedChannel;
    const optimistic = optimisticChannel(input, target?.id ?? `optimistic-${Date.now()}`);
    try {
      await channels.mutate(async (current = []) => {
        const saved = target ? await storeApi.admin.updatePaymentChannel(target.id, { ...input, expected_revision: target.revision }) : await storeApi.admin.createPaymentChannel(input);
        return target ? current.map((item) => item.id === target.id ? saved : item) : [...current.filter((item) => item.id !== optimistic.id), saved];
      }, { optimisticData: (current = []) => target ? current.map((item) => item.id === target.id ? { ...optimistic, created_at: item.created_at, revision: item.revision + 1 } : item) : [...current, optimistic], rollbackOnError: true, revalidate: false });
      toast.success(t("store.admin.saved"));
      setChannelDialogOpen(false);
    } catch (cause) { toast.error(cause instanceof Error ? cause.message : t("common.error")); } finally { setSaving(false); }
  };

  const saveChannelCredential = async (
    channelId: string,
    credential: PaymentCredentialPayload,
    currentPassword: string,
  ) => {
    try {
      await channels.mutate(
        async (current = []) => {
          const grant = await storeApi.admin.createReauthGrant(currentPassword);
          await storeApi.admin.replacePaymentCredential(channelId, credential, grant.token);
          return current.map((item) =>
            item.id === channelId
              ? { ...item, enabled: false, revision: item.revision + 1 }
              : item,
          );
        },
        {
          optimisticData: (current = []) =>
            current.map((item) =>
              item.id === channelId ? { ...item, enabled: false } : item,
            ),
          rollbackOnError: true,
          revalidate: true,
        },
      );
      toast.success(t("store.admin.channels.credential.saved"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("common.error"));
      throw cause;
    }
  };

  const deleteChannel = async (channel: StorePaymentChannel) => {
    try {
      await channels.mutate(async (current = []) => { await storeApi.admin.deletePaymentChannel(channel.id); return current.filter((item) => item.id !== channel.id); }, { optimisticData: (current = []) => current.filter((item) => item.id !== channel.id), rollbackOnError: true, revalidate: false });
      toast.success(t("store.admin.deleted"));
    } catch (cause) { toast.error(cause instanceof Error ? cause.message : t("common.error")); }
  };

  const saveSettings = async (input: StoreSettings) => {
    setSaving(true);
    try {
      await settings.mutate(() => storeApi.admin.updateSettings(input), { optimisticData: input, rollbackOnError: true, revalidate: false });
      toast.success(t("store.admin.saved"));
    } catch (cause) { toast.error(cause instanceof Error ? cause.message : t("common.error")); } finally { setSaving(false); }
  };

  const generateCodes = async (input: GenerateRedemptionCodesInput) => {
    setSaving(true);
    try {
      let generated: Awaited<ReturnType<typeof storeApi.admin.generateRedemptionCodes>> = [];
      const now = new Date();
      const optimisticRecords = Array.from({ length: input.count }, (_, index) => ({
        id: `optimistic-${now.getTime()}-${index}`,
        code_hint: "....",
        reward_kind: input.reward.kind,
        reward: input.reward,
        status: "unused" as const,
        expires_at: new Date(now.getTime() + input.validity_days * 86_400_000).toISOString(),
        redeemed_by_user_id: null,
        redeemed_at: null,
        created_by_user_id: "",
        created_at: now.toISOString(),
      }));
      await redemptions.mutate(async (current = []) => {
        generated = await storeApi.admin.generateRedemptionCodes(input);
        return [...generated.map((item) => item.record), ...current];
      }, { optimisticData: (current = []) => [...optimisticRecords, ...current], rollbackOnError: true, revalidate: false });
      toast.success(t("store.admin.redemptions.generated"));
      return generated;
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("common.error"));
      throw cause;
    } finally { setSaving(false); }
  };

  return <div className="flex flex-col gap-6">
    <PageHeader title={t("store.admin.title")} description={t("store.admin.description")} />
    <StoreAdminTabs activeTab={activeTab} onTabChange={setActiveTab} />
    <div role="tabpanel" className="grid gap-6">
      {activeTab === "products" && <AdminLoadState loading={products.isLoading || settings.isLoading || groups.isLoading} error={products.error || settings.error || groups.error} onRetry={() => { void products.mutate(); void settings.mutate(); void groups.mutate(); }}><ProductsPanel products={products.data ?? []} onCreate={() => { setSelectedProduct(null); setProductDialogOpen(true); }} onEdit={(product) => { setSelectedProduct(product); setProductDialogOpen(true); }} onDelete={(product) => setDeleteTarget({ kind: "product", record: product })} />{settings.data && <SettingsPanel settings={settings.data} saving={saving} onSave={saveSettings} />}</AdminLoadState>}
      {activeTab === "channels" && <AdminLoadState loading={channels.isLoading} error={channels.error} onRetry={() => void channels.mutate()}><ChannelsPanel channels={channels.data ?? []} onCreate={() => { setSelectedChannel(null); setChannelDialogOpen(true); }} onEdit={(channel) => { setSelectedChannel(channel); setChannelDialogOpen(true); }} onDelete={(channel) => setDeleteTarget({ kind: "channel", record: channel })} /></AdminLoadState>}
      {activeTab === "orders" && <AdminLoadState loading={orders.isLoading} error={orders.error} onRetry={() => void orders.mutate()}><OrdersPanel orders={orders.data ?? []} /></AdminLoadState>}
      {activeTab === "redemptions" && <AdminLoadState loading={redemptions.isLoading || products.isLoading} error={redemptions.error || products.error} onRetry={() => { void redemptions.mutate(); void products.mutate(); }}><RedemptionsPanel codes={redemptions.data ?? []} onGenerate={() => setRedemptionDialogOpen(true)} /></AdminLoadState>}
    </div>
    <ProductDialog open={productDialogOpen} product={selectedProduct} groups={groups.data?.groups ?? []} saving={saving} onOpenChange={setProductDialogOpen} onSave={saveProduct} />
    <ChannelDialog open={channelDialogOpen} channel={selectedChannel} saving={saving} onOpenChange={setChannelDialogOpen} onSave={saveChannel} onSaveCredential={saveChannelCredential} />
    <RedemptionDialog open={redemptionDialogOpen} plans={(products.data ?? []).filter((product) => product.kind === "plan" && product.enabled)} generating={saving} onOpenChange={setRedemptionDialogOpen} onGenerate={generateCodes} />
    <AlertDialog open={deleteTarget !== null} onOpenChange={(open) => { if (!open) setDeleteTarget(null); }}>
      <AlertDialogContent className="rounded-2xl">
        <AlertDialogHeader>
          <AlertDialogTitle>{t("common.delete")}</AlertDialogTitle>
          <AlertDialogDescription>
            {deleteTarget?.kind === "product"
              ? t("store.admin.products.deleteConfirm", { name: deleteTarget.record.name })
              : deleteTarget?.kind === "channel"
                ? t("store.admin.channels.deleteConfirm", { name: deleteTarget.record.name })
                : ""}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            onClick={() => {
              const target = deleteTarget;
              setDeleteTarget(null);
              if (target?.kind === "product") void deleteProduct(target.record);
              if (target?.kind === "channel") void deleteChannel(target.record);
            }}
          >
            {t("common.delete")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>;
}

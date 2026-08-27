import { useEffect, useState } from "react";
import { ImageOff, Upload } from "lucide-react";
import { SiAlipay, SiStripe, SiWechat } from "@icons-pack/react-simple-icons";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  storeApi,
  type PaymentAdapterKind,
  type PaymentChannelIconKind,
  type PaymentChannelInput,
  type PaymentCredentialPayload,
  type StorePaymentChannel,
} from "@/lib/store-api";

interface ChannelDialogProps {
  open: boolean;
  channel: StorePaymentChannel | null;
  saving: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (input: PaymentChannelInput) => Promise<void>;
  onSaveCredential: (
    channelId: string,
    credential: PaymentCredentialPayload,
    currentPassword: string,
  ) => Promise<void>;
}

interface CredentialDraft {
  appId: string;
  sellerId: string;
  merchantId: string;
  secretKey: string;
  publishableKey: string;
  webhookSigningSecret: string;
  apiVersion: string;
  accountId: string;
  apiV3Key: string;
  certificateSerial: string;
  merchantPrivateKeyPem: string;
  alipayPublicKeyPem: string;
  environment: "production" | "sandbox";
  liveMode: boolean;
}

const EMPTY_CREDENTIAL: CredentialDraft = {
  appId: "",
  sellerId: "",
  merchantId: "",
  secretKey: "",
  publishableKey: "",
  webhookSigningSecret: "",
  apiVersion: "",
  accountId: "",
  apiV3Key: "",
  certificateSerial: "",
  merchantPrivateKeyPem: "",
  alipayPublicKeyPem: "",
  environment: "sandbox",
  liveMode: false,
};

function buildCredential(
  adapterKind: PaymentAdapterKind,
  draft: CredentialDraft,
): PaymentCredentialPayload | null {
  if (adapterKind === "stripe") {
    const values = [
      draft.secretKey,
      draft.publishableKey,
      draft.webhookSigningSecret,
      draft.apiVersion,
      draft.accountId,
    ].map((value) => value.trim());
    if (values.some((value) => !value)) return null;
    return {
      secret_key: values[0],
      publishable_key: values[1],
      webhook_signing_secret: values[2],
      api_version: values[3],
      account_id: values[4],
      live_mode: draft.liveMode,
    };
  }
  if (adapterKind === "alipay") {
    const values = [
      draft.appId,
      draft.sellerId,
      draft.merchantPrivateKeyPem,
      draft.alipayPublicKeyPem,
    ].map((value) => value.trim());
    if (values.some((value) => !value)) return null;
    return {
      app_id: values[0],
      seller_id: values[1],
      merchant_private_key_pem: values[2],
      alipay_public_key_pem: values[3],
      environment: draft.environment,
    };
  }
  if (adapterKind === "wechat") {
    const values = [
      draft.merchantId,
      draft.appId,
      draft.apiV3Key,
      draft.certificateSerial,
      draft.merchantPrivateKeyPem,
    ].map((value) => value.trim());
    if (values.some((value) => !value)) return null;
    return {
      merchant_id: values[0],
      app_id: values[1],
      api_v3_key: values[2],
      merchant_certificate_serial: values[3],
      merchant_private_key_pem: values[4],
    };
  }
  return null;
}

function ChannelMark({ adapterKind }: { adapterKind: PaymentAdapterKind }) {
  if (adapterKind === "alipay") return <SiAlipay className="size-6 text-[#1677ff]" />;
  if (adapterKind === "wechat") return <SiWechat className="size-6 text-[#07c160]" />;
  if (adapterKind === "stripe") return <SiStripe className="size-6 text-[#635bff]" />;
  return <ImageOff className="size-6 text-muted-foreground" />;
}

export function ChannelDialog({ open, channel, saving, onOpenChange, onSave, onSaveCredential }: ChannelDialogProps) {
  const { t } = useTranslation();
  const [adapterKind, setAdapterKind] = useState<PaymentAdapterKind>("http");
  const [name, setName] = useState("");
  const [iconKind, setIconKind] = useState<PaymentChannelIconKind>("builtin");
  const [iconValue, setIconValue] = useState("");
  const [sortOrder, setSortOrder] = useState("0");
  const [enabled, setEnabled] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [imageFailed, setImageFailed] = useState(false);
  const [credential, setCredential] = useState<CredentialDraft>(EMPTY_CREDENTIAL);
  const [currentPassword, setCurrentPassword] = useState("");
  const [credentialSaving, setCredentialSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const clearSensitiveFields = () => {
    setCredential(EMPTY_CREDENTIAL);
    setCurrentPassword("");
  };

  useEffect(() => {
    if (!open) {
      setCredential(EMPTY_CREDENTIAL);
      setCurrentPassword("");
      return;
    }
    setAdapterKind(channel?.adapter_kind ?? "http");
    setName(channel?.name ?? "");
    setIconKind(channel?.icon_kind ?? "builtin");
    setIconValue(channel?.icon_value ?? "");
    setSortOrder(String(channel?.sort_order ?? 0));
    setEnabled(channel?.enabled ?? false);
    setCredential(EMPTY_CREDENTIAL);
    setCurrentPassword("");
    setCredentialSaving(false);
    setImageFailed(false);
    setError(null);
  }, [channel, open]);

  const uploadIcon = async (file: File) => {
    setUploading(true);
    setError(null);
    try {
      const result = await storeApi.admin.uploadIcon(file);
      setIconKind("upload");
      setIconValue(result.url);
      setImageFailed(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : t("store.admin.channels.uploadFailed"));
    } finally {
      setUploading(false);
    }
  };

  const submit = async () => {
    setError(null);
    const parsedSortOrder = Number(sortOrder);
    if (!name.trim() || !Number.isInteger(parsedSortOrder)) {
      setError(t("store.admin.channels.invalidChannel"));
      return;
    }
    if (iconKind === "url") {
      try {
        if (new URL(iconValue).protocol !== "https:") throw new Error();
      } catch {
        setError(t("store.admin.channels.httpsIconRequired"));
        return;
      }
    }
    if (iconKind === "upload" && !iconValue.startsWith("/api/dashboard/store/icons/")) {
      setError(t("store.admin.channels.uploadRequired"));
      return;
    }
    await onSave({
      adapter_kind: adapterKind,
      name: name.trim(),
      icon_kind: iconKind,
      icon_value: iconKind === "builtin" ? null : iconValue.trim(),
      sort_order: parsedSortOrder,
      enabled,
    });
  };

  const submitCredential = async () => {
    if (!channel || adapterKind === "http") return;
    setError(null);
    const payload = buildCredential(adapterKind, credential);
    if (!payload || !currentPassword) {
      setError(t("store.admin.channels.credential.invalid"));
      return;
    }
    setCredentialSaving(true);
    try {
      await onSaveCredential(channel.id, payload, currentPassword);
      clearSensitiveFields();
      setEnabled(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : t("common.error"));
    } finally {
      setCredentialSaving(false);
    }
  };

  const updateCredential = <Key extends keyof CredentialDraft>(
    key: Key,
    value: CredentialDraft[Key],
  ) => setCredential((current) => ({ ...current, [key]: value }));

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => {
      if (!nextOpen) clearSensitiveFields();
      onOpenChange(nextOpen);
    }}>
      <DialogContent className="max-h-[90vh] overflow-y-auto rounded-2xl sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t(channel ? "store.admin.channels.edit" : "store.admin.channels.create")}</DialogTitle>
          <DialogDescription>{t("store.admin.channels.dialogDescription")}</DialogDescription>
        </DialogHeader>

        <div className="grid gap-5 py-2">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-2">
              <Label>{t("store.admin.channels.kind")}</Label>
              <Select value={adapterKind} disabled={Boolean(channel)} onValueChange={(value) => setAdapterKind(value as PaymentAdapterKind)}>
                <SelectTrigger className="min-h-11 rounded-xl"><SelectValue /></SelectTrigger>
                <SelectContent>{["alipay", "wechat", "stripe", "http"].map((value) => <SelectItem key={value} value={value}>{t(`store.admin.channels.kinds.${value}`)}</SelectItem>)}</SelectContent>
              </Select>
            </div>
            <div className="grid gap-2"><Label htmlFor="store-channel-name">{t("store.admin.channels.name")}</Label><Input id="store-channel-name" className="min-h-11 rounded-xl" maxLength={80} value={name} onChange={(event) => setName(event.target.value)} /></div>
          </div>

          <div className="grid gap-3">
            <Label>{t("store.admin.channels.icon")}</Label>
            <div className="grid grid-cols-3 gap-2 rounded-xl bg-muted p-1">
              {(["builtin", "url", "upload"] as const).map((value) => <Button key={value} type="button" variant={iconKind === value ? "secondary" : "ghost"} className="min-h-11 rounded-lg" onClick={() => { setIconKind(value); setImageFailed(false); }}>{t(`store.admin.channels.iconKinds.${value}`)}</Button>)}
            </div>
            {iconKind === "url" && <Input className="min-h-11 rounded-xl" type="url" placeholder="https://" value={iconValue} onChange={(event) => { setIconValue(event.target.value); setImageFailed(false); }} />}
            {iconKind === "upload" && (
              <label className="flex min-h-11 cursor-pointer items-center justify-center gap-2 rounded-xl border border-dashed px-4 text-sm font-medium hover:bg-accent">
                <Upload className="h-4 w-4" />{uploading ? t("common.loading") : t("store.admin.channels.chooseImage")}
                <input className="sr-only" type="file" accept="image/png,image/jpeg,image/webp,image/svg+xml" disabled={uploading} onChange={(event) => { const file = event.target.files?.[0]; if (file) void uploadIcon(file); event.target.value = ""; }} />
              </label>
            )}
            <div className="flex min-h-16 items-center gap-3 rounded-xl border px-4">
              {iconKind !== "builtin" && iconValue && !imageFailed ? <img key={iconValue} src={iconValue} alt="" className="size-8 rounded-lg object-contain" onError={() => setImageFailed(true)} /> : <ChannelMark adapterKind={adapterKind} />}
              <div className="min-w-0"><p className="text-sm font-medium">{name || t(`store.admin.channels.kinds.${adapterKind}`)}</p><p className="truncate text-xs text-muted-foreground">{iconKind === "builtin" ? t("store.admin.channels.builtinPreview") : iconValue || t("store.admin.channels.noImage")}</p></div>
            </div>
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-2"><Label htmlFor="store-channel-sort">{t("store.admin.channels.sortOrder")}</Label><Input id="store-channel-sort" className="min-h-11 rounded-xl" type="number" value={sortOrder} onChange={(event) => setSortOrder(event.target.value)} /></div>
            <label className="flex min-h-11 cursor-pointer items-center justify-between gap-3 self-end rounded-xl border px-3"><span className="text-sm font-medium">{t("store.admin.enabled")}</span><Switch checked={enabled} onCheckedChange={setEnabled} /></label>
          </div>

          {channel && adapterKind !== "http" && (
            <section className="grid gap-4 border-t pt-5" aria-labelledby="store-channel-credential-title">
              <div>
                <h3 id="store-channel-credential-title" className="font-semibold">
                  {t("store.admin.channels.credential.title")}
                </h3>
              </div>

              {adapterKind === "stripe" && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <div className="grid gap-2"><Label htmlFor="store-stripe-account">{t("store.admin.channels.credential.accountId")}</Label><Input id="store-stripe-account" className="min-h-11 rounded-xl" value={credential.accountId} onChange={(event) => updateCredential("accountId", event.target.value)} /></div>
                  <div className="grid gap-2"><Label htmlFor="store-stripe-version">{t("store.admin.channels.credential.apiVersion")}</Label><Input id="store-stripe-version" className="min-h-11 rounded-xl" placeholder="2026-08-01" value={credential.apiVersion} onChange={(event) => updateCredential("apiVersion", event.target.value)} /></div>
                  <div className="grid gap-2"><Label htmlFor="store-stripe-public">{t("store.admin.channels.credential.publishableKey")}</Label><Input id="store-stripe-public" className="min-h-11 rounded-xl" value={credential.publishableKey} onChange={(event) => updateCredential("publishableKey", event.target.value)} /></div>
                  <div className="grid gap-2"><Label htmlFor="store-stripe-secret">{t("store.admin.channels.credential.secretKey")}</Label><Input id="store-stripe-secret" className="min-h-11 rounded-xl" type="password" autoComplete="new-password" value={credential.secretKey} onChange={(event) => updateCredential("secretKey", event.target.value)} /></div>
                  <div className="grid gap-2 sm:col-span-2"><Label htmlFor="store-stripe-webhook">{t("store.admin.channels.credential.webhookSecret")}</Label><Input id="store-stripe-webhook" className="min-h-11 rounded-xl" type="password" autoComplete="new-password" value={credential.webhookSigningSecret} onChange={(event) => updateCredential("webhookSigningSecret", event.target.value)} /></div>
                  <label className="flex min-h-11 cursor-pointer items-center justify-between gap-3 rounded-xl border px-3 sm:col-span-2"><span className="text-sm font-medium">{t("store.admin.channels.credential.liveMode")}</span><Switch checked={credential.liveMode} onCheckedChange={(value) => updateCredential("liveMode", value)} /></label>
                </div>
              )}

              {adapterKind === "alipay" && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <div className="grid gap-2"><Label htmlFor="store-alipay-app">{t("store.admin.channels.credential.appId")}</Label><Input id="store-alipay-app" className="min-h-11 rounded-xl" value={credential.appId} onChange={(event) => updateCredential("appId", event.target.value)} /></div>
                  <div className="grid gap-2"><Label htmlFor="store-alipay-seller">{t("store.admin.channels.credential.sellerId")}</Label><Input id="store-alipay-seller" className="min-h-11 rounded-xl" value={credential.sellerId} onChange={(event) => updateCredential("sellerId", event.target.value)} /></div>
                  <div className="grid gap-2 sm:col-span-2"><Label>{t("store.admin.channels.credential.environment")}</Label><Select value={credential.environment} onValueChange={(value) => updateCredential("environment", value as "production" | "sandbox")}><SelectTrigger className="min-h-11 rounded-xl"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="sandbox">{t("store.admin.channels.credential.sandbox")}</SelectItem><SelectItem value="production">{t("store.admin.channels.credential.production")}</SelectItem></SelectContent></Select></div>
                  <div className="grid gap-2 sm:col-span-2"><Label htmlFor="store-alipay-private">{t("store.admin.channels.credential.merchantPrivateKey")}</Label><Textarea id="store-alipay-private" className="min-h-28 rounded-xl font-mono text-xs" value={credential.merchantPrivateKeyPem} onChange={(event) => updateCredential("merchantPrivateKeyPem", event.target.value)} /></div>
                  <div className="grid gap-2 sm:col-span-2"><Label htmlFor="store-alipay-public">{t("store.admin.channels.credential.alipayPublicKey")}</Label><Textarea id="store-alipay-public" className="min-h-28 rounded-xl font-mono text-xs" value={credential.alipayPublicKeyPem} onChange={(event) => updateCredential("alipayPublicKeyPem", event.target.value)} /></div>
                </div>
              )}

              {adapterKind === "wechat" && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <div className="grid gap-2"><Label htmlFor="store-wechat-merchant">{t("store.admin.channels.credential.merchantId")}</Label><Input id="store-wechat-merchant" className="min-h-11 rounded-xl" value={credential.merchantId} onChange={(event) => updateCredential("merchantId", event.target.value)} /></div>
                  <div className="grid gap-2"><Label htmlFor="store-wechat-app">{t("store.admin.channels.credential.appId")}</Label><Input id="store-wechat-app" className="min-h-11 rounded-xl" value={credential.appId} onChange={(event) => updateCredential("appId", event.target.value)} /></div>
                  <div className="grid gap-2"><Label htmlFor="store-wechat-serial">{t("store.admin.channels.credential.certificateSerial")}</Label><Input id="store-wechat-serial" className="min-h-11 rounded-xl" value={credential.certificateSerial} onChange={(event) => updateCredential("certificateSerial", event.target.value)} /></div>
                  <div className="grid gap-2"><Label htmlFor="store-wechat-v3">{t("store.admin.channels.credential.apiV3Key")}</Label><Input id="store-wechat-v3" className="min-h-11 rounded-xl" type="password" autoComplete="new-password" value={credential.apiV3Key} onChange={(event) => updateCredential("apiV3Key", event.target.value)} /></div>
                  <div className="grid gap-2 sm:col-span-2"><Label htmlFor="store-wechat-private">{t("store.admin.channels.credential.merchantPrivateKey")}</Label><Textarea id="store-wechat-private" className="min-h-28 rounded-xl font-mono text-xs" value={credential.merchantPrivateKeyPem} onChange={(event) => updateCredential("merchantPrivateKeyPem", event.target.value)} /></div>
                </div>
              )}

              <div className="grid gap-2">
                <Label htmlFor="store-channel-current-password">{t("store.admin.channels.credential.currentPassword")}</Label>
                <Input id="store-channel-current-password" className="min-h-11 rounded-xl" type="password" autoComplete="current-password" value={currentPassword} onChange={(event) => setCurrentPassword(event.target.value)} />
              </div>
              <Button type="button" variant="secondary" className="min-h-11 w-fit rounded-xl" disabled={credentialSaving} onClick={() => void submitCredential()}>{credentialSaving ? t("common.loading") : t("store.admin.channels.credential.replace")}</Button>
            </section>
          )}
          {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" className="rounded-xl" onClick={() => onOpenChange(false)}>{t("common.cancel")}</Button>
          <Button type="button" className="rounded-xl" disabled={saving || uploading} onClick={() => void submit()}>{saving ? t("common.loading") : t("common.save")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

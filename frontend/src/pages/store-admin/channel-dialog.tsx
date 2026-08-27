import { useEffect, useState } from "react";
import { ImageOff, Upload } from "lucide-react";
import { SiAlipay, SiWechat } from "@icons-pack/react-simple-icons";
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
import {
  storeApi,
  type PaymentChannelIconKind,
  type PaymentChannelInput,
  type PaymentChannelKind,
  type PaymentChannelMode,
  type StorePaymentChannel,
} from "@/lib/store-api";

interface ChannelDialogProps {
  open: boolean;
  channel: StorePaymentChannel | null;
  saving: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (input: PaymentChannelInput) => Promise<void>;
}

function ChannelMark({ kind }: { kind: PaymentChannelKind }) {
  if (kind === "alipay") return <SiAlipay className="size-6 text-[#1677ff]" />;
  if (kind === "wechat") return <SiWechat className="size-6 text-[#07c160]" />;
  return <ImageOff className="size-6 text-muted-foreground" />;
}

export function ChannelDialog({ open, channel, saving, onOpenChange, onSave }: ChannelDialogProps) {
  const { t } = useTranslation();
  const [kind, setKind] = useState<PaymentChannelKind>("custom");
  const [name, setName] = useState("");
  const [mode, setMode] = useState<PaymentChannelMode>("redirect");
  const [endpoint, setEndpoint] = useState("");
  const [iconKind, setIconKind] = useState<PaymentChannelIconKind>("builtin");
  const [iconValue, setIconValue] = useState("");
  const [secret, setSecret] = useState("");
  const [sortOrder, setSortOrder] = useState("0");
  const [enabled, setEnabled] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [imageFailed, setImageFailed] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setKind(channel?.kind ?? "custom");
    setName(channel?.name ?? "");
    setMode(channel?.mode ?? "redirect");
    setEndpoint(channel?.endpoint ?? "");
    setIconKind(channel?.icon_kind ?? "builtin");
    setIconValue(channel?.icon_value ?? "");
    setSecret("");
    setSortOrder(String(channel?.sort_order ?? 0));
    setEnabled(channel?.enabled ?? false);
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
      kind,
      name: name.trim(),
      mode,
      endpoint: endpoint.trim() || null,
      icon_kind: iconKind,
      icon_value: iconKind === "builtin" ? null : iconValue.trim(),
      config_secret: secret || null,
      sort_order: parsedSortOrder,
      enabled,
    });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto rounded-2xl sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t(channel ? "store.admin.channels.edit" : "store.admin.channels.create")}</DialogTitle>
          <DialogDescription>{t("store.admin.channels.dialogDescription")}</DialogDescription>
        </DialogHeader>

        <div className="grid gap-5 py-2">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-2">
              <Label>{t("store.admin.channels.kind")}</Label>
              <Select value={kind} disabled={Boolean(channel)} onValueChange={(value) => setKind(value as PaymentChannelKind)}>
                <SelectTrigger className="min-h-11 rounded-xl"><SelectValue /></SelectTrigger>
                <SelectContent>{["alipay", "wechat", "custom"].map((value) => <SelectItem key={value} value={value}>{t(`store.admin.channels.kinds.${value}`)}</SelectItem>)}</SelectContent>
              </Select>
            </div>
            <div className="grid gap-2"><Label htmlFor="store-channel-name">{t("store.admin.channels.name")}</Label><Input id="store-channel-name" className="min-h-11 rounded-xl" maxLength={80} value={name} onChange={(event) => setName(event.target.value)} /></div>
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-2"><Label>{t("store.admin.channels.mode")}</Label><Select value={mode} onValueChange={(value) => setMode(value as PaymentChannelMode)}><SelectTrigger className="min-h-11 rounded-xl"><SelectValue /></SelectTrigger><SelectContent>{["redirect", "qr", "manual"].map((value) => <SelectItem key={value} value={value}>{t(`store.admin.channels.modes.${value}`)}</SelectItem>)}</SelectContent></Select></div>
            <div className="grid gap-2"><Label htmlFor="store-channel-endpoint">{t("store.admin.channels.endpoint")}</Label><Input id="store-channel-endpoint" className="min-h-11 rounded-xl" value={endpoint} onChange={(event) => setEndpoint(event.target.value)} /></div>
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
              {iconKind !== "builtin" && iconValue && !imageFailed ? <img key={iconValue} src={iconValue} alt="" className="size-8 rounded-lg object-contain" onError={() => setImageFailed(true)} /> : <ChannelMark kind={kind} />}
              <div className="min-w-0"><p className="text-sm font-medium">{name || t(`store.admin.channels.kinds.${kind}`)}</p><p className="truncate text-xs text-muted-foreground">{iconKind === "builtin" ? t("store.admin.channels.builtinPreview") : iconValue || t("store.admin.channels.noImage")}</p></div>
            </div>
          </div>

          <div className="grid gap-2"><Label htmlFor="store-channel-secret">{t("store.admin.channels.secret")}</Label><Input id="store-channel-secret" className="min-h-11 rounded-xl" type="password" autoComplete="new-password" value={secret} onChange={(event) => setSecret(event.target.value)} /></div>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="grid gap-2"><Label htmlFor="store-channel-sort">{t("store.admin.channels.sortOrder")}</Label><Input id="store-channel-sort" className="min-h-11 rounded-xl" type="number" value={sortOrder} onChange={(event) => setSortOrder(event.target.value)} /></div>
            <label className="flex min-h-11 cursor-pointer items-center justify-between gap-3 self-end rounded-xl border px-3"><span className="text-sm font-medium">{t("store.admin.enabled")}</span><Switch checked={enabled} onCheckedChange={setEnabled} /></label>
          </div>
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

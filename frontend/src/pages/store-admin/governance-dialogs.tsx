import { useState } from "react";
import { FileCheck2, Plus, ShieldCheck } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import useSWR from "swr";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  storeApi,
  type PutStoreChannelReadinessInput,
  type StoreChannelReadinessProfile,
  type StoreCheckoutActionKind,
  type StorePaymentChannel,
  type StorePrivacyRecord,
  type StorePrivacyRecordsView,
  type StoreRetentionOverview,
} from "@/lib/store-api";
import type { StoreCurrency } from "@/lib/store-money";
import {
  buildPrivacyRecordInput,
  optimisticReadiness,
  validateReadinessInput,
} from "./governance-state";

const PRIVACY_KEY = "/api/dashboard/store/admin/privacy-records";

function DialogLoading() {
  return (
    <div className="grid gap-3" aria-busy="true">
      <Skeleton className="h-16 rounded-xl" />
      <Skeleton className="h-24 rounded-xl" />
      <Skeleton className="h-11 rounded-xl" />
    </div>
  );
}

function DialogError({ onRetry }: { onRetry: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex min-h-36 flex-col items-center justify-center gap-3 rounded-xl border border-dashed p-5 text-center">
      <p className="text-sm text-muted-foreground">{t("store.admin.loadFailed")}</p>
      <Button type="button" variant="outline" className="rounded-xl" onClick={onRetry}>
        {t("store.admin.retry")}
      </Button>
    </div>
  );
}

const initialPrivacyDraft = {
  policyVersion: "",
  jurisdiction: "CN",
  regions: "CN",
  legalBasis: "",
  evidenceDigest: "",
  financialDays: "2557",
  expiredGrantHours: "24",
  reviewDays: "90",
};

export function PrivacyRecordsDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t, i18n } = useTranslation();
  const records = useSWR<StorePrivacyRecordsView>(open ? PRIVACY_KEY : null, storeApi.admin.listPrivacyRecords);
  const [draft, setDraft] = useState(initialPrivacyDraft);
  const [saving, setSaving] = useState(false);

  const update = (key: keyof typeof draft, value: string) => {
    setDraft((current) => ({ ...current, [key]: value }));
  };

  const submit = async () => {
    const input = buildPrivacyRecordInput(draft);
    if (!input) {
      toast.error(t("store.admin.governance.invalid"));
      return;
    }
    const now = new Date();
    const optimistic: StorePrivacyRecord = {
      id: `optimistic-${crypto.randomUUID()}`,
      policy_version: input.policy_version,
      jurisdiction: input.jurisdiction,
      allowed_regions: input.allowed_regions,
      retention: input.retention,
      legal_basis: input.legal_basis,
      reviewer_id: "",
      evidence_digest: input.evidence_digest,
      approved_at: now.toISOString(),
      next_review_at: new Date(now.getTime() + input.review_after_days * 86_400_000).toISOString(),
      accepted: true,
    };
    setSaving(true);
    try {
      await records.mutate(
        async (current = { records: [] }) => {
          const saved = await storeApi.admin.createPrivacyRecord(input);
          return { records: [saved, ...current.records.filter((record) => record.id !== optimistic.id)] };
        },
        {
          optimisticData: (current = { records: [] }) => ({ records: [optimistic, ...current.records] }),
          rollbackOnError: true,
          revalidate: false,
        },
      );
      setDraft(initialPrivacyDraft);
      toast.success(t("store.admin.governance.privacyRecords.created"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("common.error"));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[88vh] overflow-y-auto rounded-2xl sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-lg">
            <FileCheck2 className="size-5" />
            {t("store.admin.governance.privacyRecords.title")}
          </DialogTitle>
          <DialogDescription>{t("store.admin.governance.privacyRecords.description")}</DialogDescription>
        </DialogHeader>
        {records.isLoading ? (
          <DialogLoading />
        ) : records.error ? (
          <DialogError onRetry={() => void records.mutate()} />
        ) : (
          <div className="grid gap-5">
            <div className="grid max-h-44 gap-2 overflow-y-auto pr-1">
              {(records.data?.records ?? []).length === 0 ? (
                <p className="rounded-xl border border-dashed p-4 text-sm text-muted-foreground">
                  {t("store.admin.governance.privacyRecords.empty")}
                </p>
              ) : (
                records.data?.records.map((record) => (
                  <div key={record.id} className="flex flex-wrap items-center justify-between gap-3 rounded-xl border p-3">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">{record.policy_version}</p>
                      <p className="text-xs text-muted-foreground">
                        {record.jurisdiction} · {record.allowed_regions.join(", ")}
                      </p>
                    </div>
                    <div className="text-right text-xs text-muted-foreground">
                      <Badge variant={record.accepted ? "default" : "secondary"}>
                        {t(record.accepted ? "store.admin.governance.accepted" : "store.admin.governance.rejected")}
                      </Badge>
                      <p className="mt-1">{new Intl.DateTimeFormat(i18n.language).format(new Date(record.next_review_at))}</p>
                    </div>
                  </div>
                ))
              )}
            </div>
            <section className="grid gap-4 border-t pt-5" aria-labelledby="new-privacy-record-title">
              <div>
                <h3 id="new-privacy-record-title" className="font-semibold">
                  {t("store.admin.governance.privacyRecords.create")}
                </h3>
                <p className="text-sm text-muted-foreground">{t("store.admin.governance.privacyRecords.immutable")}</p>
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="grid gap-2"><Label htmlFor="privacy-version">{t("store.admin.governance.fields.policyVersion")}</Label><Input id="privacy-version" className="min-h-11 rounded-xl" value={draft.policyVersion} onChange={(event) => update("policyVersion", event.target.value)} /></div>
                <div className="grid gap-2"><Label htmlFor="privacy-jurisdiction">{t("store.admin.governance.fields.jurisdiction")}</Label><Input id="privacy-jurisdiction" className="min-h-11 rounded-xl" value={draft.jurisdiction} onChange={(event) => update("jurisdiction", event.target.value)} /></div>
                <div className="grid gap-2 sm:col-span-2"><Label htmlFor="privacy-regions">{t("store.admin.governance.fields.allowedRegions")}</Label><Input id="privacy-regions" className="min-h-11 rounded-xl" value={draft.regions} onChange={(event) => update("regions", event.target.value)} /></div>
                <div className="grid gap-2 sm:col-span-2"><Label htmlFor="privacy-legal">{t("store.admin.governance.fields.legalBasis")}</Label><Textarea id="privacy-legal" className="min-h-20 rounded-xl" value={draft.legalBasis} onChange={(event) => update("legalBasis", event.target.value)} /></div>
                <div className="grid gap-2 sm:col-span-2"><Label htmlFor="privacy-evidence">{t("store.admin.governance.fields.evidenceDigest")}</Label><Input id="privacy-evidence" className="min-h-11 rounded-xl font-mono text-xs" value={draft.evidenceDigest} onChange={(event) => update("evidenceDigest", event.target.value)} /></div>
                <div className="grid gap-2"><Label htmlFor="privacy-financial-days">{t("store.admin.governance.fields.financialDays")}</Label><Input id="privacy-financial-days" className="min-h-11 rounded-xl" type="number" min={1} max={36500} value={draft.financialDays} onChange={(event) => update("financialDays", event.target.value)} /></div>
                <div className="grid gap-2"><Label htmlFor="privacy-grant-hours">{t("store.admin.governance.fields.expiredGrantHours")}</Label><Input id="privacy-grant-hours" className="min-h-11 rounded-xl" type="number" min={1} max={24} value={draft.expiredGrantHours} onChange={(event) => update("expiredGrantHours", event.target.value)} /></div>
                <div className="grid gap-2"><Label htmlFor="privacy-review-days">{t("store.admin.governance.fields.reviewDays")}</Label><Input id="privacy-review-days" className="min-h-11 rounded-xl" type="number" min={1} max={365} value={draft.reviewDays} onChange={(event) => update("reviewDays", event.target.value)} /></div>
                <div className="flex items-end"><p className="pb-3 text-xs text-muted-foreground">{t("store.admin.governance.privacyRecords.fixedRetention")}</p></div>
              </div>
            </section>
          </div>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" className="rounded-xl" onClick={() => onOpenChange(false)}>{t("common.close")}</Button>
          <Button type="button" className="rounded-xl" disabled={saving || records.isLoading || Boolean(records.error)} onClick={() => void submit()}><Plus className="size-4" />{saving ? t("common.loading") : t("store.admin.governance.privacyRecords.create")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function adapterDefaults(channel: StorePaymentChannel): {
  currencies: StoreCurrency[];
  actions: StoreCheckoutActionKind[];
} {
  if (channel.adapter_kind === "stripe") return { currencies: ["CNY", "USD"], actions: ["redirect"] };
  if (channel.adapter_kind === "alipay") return { currencies: ["CNY"], actions: ["form"] };
  return { currencies: ["CNY"], actions: ["qr", "redirect"] };
}

function ReadinessForm({
  channel,
  profile,
  privacyRecords,
  saving,
  onSave,
}: {
  channel: StorePaymentChannel;
  profile: StoreChannelReadinessProfile | null;
  privacyRecords: StorePrivacyRecord[];
  saving: boolean;
  onSave: (input: PutStoreChannelReadinessInput) => Promise<void>;
}) {
  const { t } = useTranslation();
  const defaults = adapterDefaults(channel);
  const [privacyId, setPrivacyId] = useState(profile?.privacy_record_id ?? privacyRecords[0]?.id ?? "");
  const [callbackPassed, setCallbackPassed] = useState(profile?.callback_verification_passed ?? false);
  const [currencies, setCurrencies] = useState<StoreCurrency[]>(profile?.supported_currencies ?? defaults.currencies);
  const [actions, setActions] = useState<StoreCheckoutActionKind[]>(profile?.checkout_action_kinds ?? defaults.actions);
  const [limits, setLimits] = useState(profile?.amount_limits ?? { CNY: { min_minor: "1", max_minor: "100000000" }, USD: { min_minor: "1", max_minor: "100000000" } });
  const [licenseDigest, setLicenseDigest] = useState(profile?.license_evidence_digest ?? "");
  const [runtimeDigest, setRuntimeDigest] = useState(profile?.runtime_evidence_digest ?? "");
  const [availabilityDigest, setAvailabilityDigest] = useState(profile?.availability_evidence_digest ?? "");
  const [validDays, setValidDays] = useState("30");

  const toggleCurrency = (currency: StoreCurrency, checked: boolean) => {
    setCurrencies((current) => checked ? [...new Set([...current, currency])] : current.filter((item) => item !== currency));
  };
  const toggleAction = (action: StoreCheckoutActionKind, checked: boolean) => {
    setActions((current) => checked ? [...new Set([...current, action])] : current.filter((item) => item !== action));
  };
  const updateLimit = (currency: StoreCurrency, key: "min_minor" | "max_minor", value: string) => {
    setLimits((current) => ({ ...current, [currency]: { min_minor: current[currency]?.min_minor ?? "1", max_minor: current[currency]?.max_minor ?? "1", [key]: value } }));
  };

  const submit = async () => {
    const days = Number(validDays);
    const amountLimits = Object.fromEntries(currencies.map((currency) => [currency, limits[currency]]));
    const input: PutStoreChannelReadinessInput = {
      privacy_record_id: privacyId,
      callback_verification_passed: callbackPassed,
      supported_currencies: currencies,
      amount_limits: amountLimits,
      checkout_action_kinds: actions,
      license_evidence_digest: licenseDigest,
      runtime_evidence_digest: runtimeDigest,
      availability_evidence_digest: availabilityDigest,
      valid_for_days: days,
    };
    if (!validateReadinessInput(channel.adapter_kind, input)) {
      toast.error(t("store.admin.governance.invalid"));
      return;
    }
    await onSave(input);
  };

  const currencyOptions: StoreCurrency[] = channel.adapter_kind === "stripe" ? ["CNY", "USD"] : ["CNY"];
  const actionOptions: StoreCheckoutActionKind[] = channel.adapter_kind === "wechat" ? ["qr", "redirect"] : defaults.actions;

  return (
    <div className="grid gap-5">
      {profile && (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-muted p-3 text-sm">
          <span>{t("store.admin.governance.readiness.currentProfile")}</span>
          <Badge variant={profile.callback_verification_passed ? "default" : "secondary"}>{t(profile.callback_verification_passed ? "store.admin.governance.readiness.verified" : "store.admin.governance.readiness.pending")}</Badge>
        </div>
      )}
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="grid gap-2 sm:col-span-2"><Label>{t("store.admin.governance.fields.privacyRecord")}</Label><Select value={privacyId} onValueChange={setPrivacyId}><SelectTrigger className="min-h-11 rounded-xl"><SelectValue placeholder={t("store.admin.governance.readiness.selectPrivacy")} /></SelectTrigger><SelectContent>{privacyRecords.map((record) => <SelectItem key={record.id} value={record.id}>{record.policy_version} · {record.jurisdiction}</SelectItem>)}</SelectContent></Select></div>
        <div className="flex min-h-11 items-center justify-between gap-3 rounded-xl border p-3 sm:col-span-2"><div><Label htmlFor="readiness-callback">{t("store.admin.governance.fields.callbackVerification")}</Label><p className="text-xs text-muted-foreground">{t("store.admin.governance.readiness.callbackHelp")}</p></div><Switch id="readiness-callback" checked={callbackPassed} onCheckedChange={setCallbackPassed} /></div>
        <fieldset className="grid gap-3 rounded-xl border p-3"><legend className="px-1 text-sm font-medium">{t("store.admin.governance.fields.currencies")}</legend>{currencyOptions.map((currency) => <label key={currency} className="flex min-h-9 items-center gap-3 text-sm"><Checkbox checked={currencies.includes(currency)} disabled={channel.adapter_kind !== "stripe"} onCheckedChange={(checked) => toggleCurrency(currency, checked === true)} />{currency}</label>)}</fieldset>
        <fieldset className="grid gap-3 rounded-xl border p-3"><legend className="px-1 text-sm font-medium">{t("store.admin.governance.fields.actions")}</legend>{actionOptions.map((action) => <label key={action} className="flex min-h-9 items-center gap-3 text-sm"><Checkbox checked={actions.includes(action)} disabled={channel.adapter_kind !== "wechat"} onCheckedChange={(checked) => toggleAction(action, checked === true)} />{t(`store.admin.governance.actions.${action}`)}</label>)}</fieldset>
        {currencies.map((currency) => <div key={currency} className="grid gap-3 rounded-xl border p-3 sm:col-span-2"><p className="text-sm font-medium">{t("store.admin.governance.readiness.amountRange", { currency })}</p><div className="grid gap-3 sm:grid-cols-2"><div className="grid gap-2"><Label htmlFor={`readiness-${currency}-min`}>{t("store.admin.governance.fields.minimumMinor")}</Label><Input id={`readiness-${currency}-min`} className="min-h-11 rounded-xl" inputMode="numeric" value={limits[currency]?.min_minor ?? ""} onChange={(event) => updateLimit(currency, "min_minor", event.target.value)} /></div><div className="grid gap-2"><Label htmlFor={`readiness-${currency}-max`}>{t("store.admin.governance.fields.maximumMinor")}</Label><Input id={`readiness-${currency}-max`} className="min-h-11 rounded-xl" inputMode="numeric" value={limits[currency]?.max_minor ?? ""} onChange={(event) => updateLimit(currency, "max_minor", event.target.value)} /></div></div></div>)}
        {(["license", "runtime", "availability"] as const).map((kind) => <div key={kind} className="grid gap-2 sm:col-span-2"><Label htmlFor={`readiness-${kind}`}>{t(`store.admin.governance.fields.${kind}Evidence`)}</Label><Input id={`readiness-${kind}`} className="min-h-11 rounded-xl font-mono text-xs" value={kind === "license" ? licenseDigest : kind === "runtime" ? runtimeDigest : availabilityDigest} onChange={(event) => { if (kind === "license") setLicenseDigest(event.target.value); else if (kind === "runtime") setRuntimeDigest(event.target.value); else setAvailabilityDigest(event.target.value); }} /></div>)}
        <div className="grid gap-2"><Label htmlFor="readiness-valid-days">{t("store.admin.governance.fields.validDays")}</Label><Input id="readiness-valid-days" className="min-h-11 rounded-xl" type="number" min={1} max={90} value={validDays} onChange={(event) => setValidDays(event.target.value)} /></div>
      </div>
      <Button type="button" className="min-h-11 w-fit rounded-xl" disabled={saving || privacyRecords.length === 0} onClick={() => void submit()}><ShieldCheck className="size-4" />{saving ? t("common.loading") : t("common.save")}</Button>
    </div>
  );
}

export function ChannelReadinessDialog({
  channel,
  open,
  onOpenChange,
  onSaved,
}: {
  channel: StorePaymentChannel | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: () => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const readinessKey = channel && open ? `/api/dashboard/store/admin/payment-channels/${encodeURIComponent(channel.id)}/readiness` : null;
  const readiness = useSWR(readinessKey, () => storeApi.admin.getChannelReadiness(channel!.id));
  const privacy = useSWR<StorePrivacyRecordsView>(open ? PRIVACY_KEY : null, storeApi.admin.listPrivacyRecords);
  const [saving, setSaving] = useState(false);

  const save = async (input: PutStoreChannelReadinessInput) => {
    if (!channel) return;
    setSaving(true);
    try {
      await readiness.mutate(
        async () => ({ readiness: await storeApi.admin.putChannelReadiness(channel.id, input) }),
        {
          optimisticData: (current) => optimisticReadiness(channel, input, current),
          rollbackOnError: true,
          revalidate: false,
        },
      );
      await onSaved();
      toast.success(t("store.admin.governance.readiness.saved"));
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : t("common.error"));
    } finally {
      setSaving(false);
    }
  };

  const error = readiness.error || privacy.error;
  const loading = readiness.isLoading || privacy.isLoading;
  const profileKey = readiness.data?.readiness?.verified_at ?? `${channel?.id ?? "none"}-new`;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[88vh] overflow-y-auto rounded-2xl sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-lg"><ShieldCheck className="size-5" />{t("store.admin.governance.readiness.title", { name: channel?.name ?? "" })}</DialogTitle>
          <DialogDescription>{t("store.admin.governance.readiness.description")}</DialogDescription>
        </DialogHeader>
        {loading ? <DialogLoading /> : error ? <DialogError onRetry={() => { void readiness.mutate(); void privacy.mutate(); }} /> : channel ? <ReadinessForm key={profileKey} channel={channel} profile={readiness.data?.readiness ?? null} privacyRecords={privacy.data?.records ?? []} saving={saving} onSave={save} /> : null}
        <DialogFooter><Button type="button" variant="outline" className="rounded-xl" onClick={() => onOpenChange(false)}>{t("common.close")}</Button></DialogFooter>
      </DialogContent>
    </Dialog>
  );
}


const RETENTION_KEY = "/api/dashboard/store/admin/retention";

export function RetentionDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const [password, setPassword] = useState("");
  const [reason, setReason] = useState("");
  const [evidenceDigest, setEvidenceDigest] = useState("");
  const [busy, setBusy] = useState(false);
  const { data, error, isLoading, mutate } = useSWR<StoreRetentionOverview>(
    open ? RETENTION_KEY : null,
    () => storeApi.admin.getRetention(),
  );

  async function withRetentionReauth<T>(action: (token: string) => Promise<T>) {
    const grant = await storeApi.admin.createReauthGrant(password, "retention_operation");
    return action(grant.token);
  }

  async function runRetention() {
    if (!reason.trim()) {
      toast.error(t("store.admin.governance.invalid"));
      return;
    }
    setBusy(true);
    try {
      await withRetentionReauth((token) => storeApi.admin.runRetention(reason.trim(), token));
      toast.success(t("store.admin.governance.retention.ran"));
      setReason("");
      setPassword("");
      await mutate();
    } catch {
      toast.error(t("store.admin.governance.invalid"));
    } finally {
      setBusy(false);
    }
  }

  async function containRetention() {
    if (!reason.trim() || evidenceDigest.trim().length !== 64) {
      toast.error(t("store.admin.governance.invalid"));
      return;
    }
    setBusy(true);
    try {
      await withRetentionReauth((token) =>
        storeApi.admin.containRetention(
          { reason: reason.trim(), evidence_digest: evidenceDigest.trim() },
          token,
        ),
      );
      toast.success(t("store.admin.governance.retention.contained"));
      setReason("");
      setEvidenceDigest("");
      setPassword("");
      await mutate();
    } catch {
      toast.error(t("store.admin.governance.invalid"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("store.admin.governance.retention.title")}</DialogTitle>
          <DialogDescription>{t("store.admin.governance.retention.description")}</DialogDescription>
        </DialogHeader>
        {isLoading ? (
          <DialogLoading />
        ) : error || !data ? (
          <DialogError onRetry={() => void mutate()} />
        ) : (
          <div className="grid gap-4">
            <div className="grid gap-2 rounded-xl border p-4 text-sm">
              <p>
                {t("store.admin.governance.retention.failures")}: {data.status.consecutive_failures}
              </p>
              <p>
                {t("store.admin.governance.retention.checkout")}:{" "}
                {data.status.checkout_paused
                  ? t("store.admin.governance.retention.paused")
                  : t("store.admin.governance.retention.open")}
              </p>
              <p>
                {t("store.admin.governance.retention.runs")}: {data.runs.length} ·{" "}
                {t("store.admin.governance.retention.holds")}: {data.holds.length}
              </p>
            </div>
            <div className="grid gap-3">
              <div className="grid gap-2">
                <Label htmlFor="retention-password">{t("store.admin.orders.currentPassword")}</Label>
                <Input
                  id="retention-password"
                  type="password"
                  value={password}
                  onChange={(event) => setPassword(event.target.value)}
                  className="rounded-xl"
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="retention-reason">{t("store.admin.governance.retention.reason")}</Label>
                <Textarea
                  id="retention-reason"
                  value={reason}
                  onChange={(event) => setReason(event.target.value)}
                  className="min-h-24 rounded-xl"
                />
              </div>
              {data.status.checkout_paused && (
                <div className="grid gap-2">
                  <Label htmlFor="retention-evidence">{t("store.admin.governance.fields.evidenceDigest")}</Label>
                  <Input
                    id="retention-evidence"
                    value={evidenceDigest}
                    onChange={(event) => setEvidenceDigest(event.target.value)}
                    className="rounded-xl font-mono text-xs"
                  />
                </div>
              )}
            </div>
            <DialogFooter className="gap-2 sm:justify-between">
              <Button type="button" variant="outline" className="rounded-xl" disabled={busy} onClick={() => void runRetention()}>
                {t("store.admin.governance.retention.run")}
              </Button>
              {data.status.checkout_paused && (
                <Button type="button" className="rounded-xl" disabled={busy} onClick={() => void containRetention()}>
                  {t("store.admin.governance.retention.contain")}
                </Button>
              )}
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

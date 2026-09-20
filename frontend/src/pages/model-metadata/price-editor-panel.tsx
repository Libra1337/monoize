import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pin, RefreshCw, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  deleteBillingRateOptimistic,
  upsertBillingRateOptimistic,
} from "@/lib/swr";
import type { BillingRateRecord, RateCurrency } from "@/lib/api";
import { nanoPerTokenToPerMillion, perMillionToNanoPerToken } from "@/lib/exact-decimal";

type UsageClass =
  | "input_uncached"
  | "cache_read"
  | "cache_write_5m"
  | "cache_write_1h"
  | "output";

const VISIBLE_USAGE_CLASSES: Array<{ id: UsageClass; label: string; required: boolean }> = [
  { id: "input_uncached", label: "Input", required: true },
  { id: "cache_read", label: "Cache read", required: false },
  { id: "cache_write_5m", label: "Cache write 5m", required: false },
  { id: "cache_write_1h", label: "Cache write 1h", required: false },
  { id: "output", label: "Output", required: true },
];

/** UI19d: the cache-write rate rows carry the TTL on the rate itself. */
const CACHE_WRITE_TTL: Partial<Record<UsageClass, string>> = {
  cache_write_5m: "5m",
  cache_write_1h: "1h",
};

interface OverrideFormState {
  input: string;
  inputPeak: string;
  cache: string;
  cachePeak: string;
  cacheWrite5m: string;
  cacheWrite5mPeak: string;
  cacheWrite1h: string;
  cacheWrite1hPeak: string;
  output: string;
  outputPeak: string;
}

const EMPTY_FORM: OverrideFormState = {
  input: "",
  inputPeak: "",
  cache: "",
  cachePeak: "",
  cacheWrite5m: "",
  cacheWrite5mPeak: "",
  cacheWrite1h: "",
  cacheWrite1hPeak: "",
  output: "",
  outputPeak: "",
};

/**
 * UI19c: prefill only from rates of the selected currency. Digits are never
 * reinterpreted across currencies — a USD rate leaves the field empty in CNY mode.
 */
function nanoToInput(rate: BillingRateRecord | undefined, currency: RateCurrency): string {
  if (!rate || rate.unit_price_currency !== currency) return "";
  return nanoPerTokenToPerMillion(rate.unit_price_nano) ?? "";
}

function nanoToPeakInput(
  rate: BillingRateRecord | undefined,
  currency: RateCurrency
): string {
  if (!rate || rate.unit_price_currency !== currency || !rate.peak_unit_price_nano)
    return "";
  return nanoPerTokenToPerMillion(rate.peak_unit_price_nano) ?? "";
}

function effectiveRate(rates: BillingRateRecord[], usageClass: UsageClass) {
  return rates
    .filter((rate) => rate.enabled && rate.rate_kind === "token" && rate.usage_class === usageClass)
    .sort((a, b) => {
      if (a.source === "manual" && b.source !== "manual") return -1;
      if (b.source === "manual" && a.source !== "manual") return 1;
      return b.priority - a.priority;
    })[0];
}

function prefillForm(
  modelRates: BillingRateRecord[],
  currency: RateCurrency
): OverrideFormState {
  return {
    input: nanoToInput(effectiveRate(modelRates, "input_uncached"), currency),
    inputPeak: nanoToPeakInput(effectiveRate(modelRates, "input_uncached"), currency),
    cache: nanoToInput(effectiveRate(modelRates, "cache_read"), currency),
    cachePeak: nanoToPeakInput(effectiveRate(modelRates, "cache_read"), currency),
    cacheWrite5m: nanoToInput(effectiveRate(modelRates, "cache_write_5m"), currency),
    cacheWrite5mPeak: nanoToPeakInput(effectiveRate(modelRates, "cache_write_5m"), currency),
    cacheWrite1h: nanoToInput(effectiveRate(modelRates, "cache_write_1h"), currency),
    cacheWrite1hPeak: nanoToPeakInput(effectiveRate(modelRates, "cache_write_1h"), currency),
    output: nanoToInput(effectiveRate(modelRates, "output"), currency),
    outputPeak: nanoToPeakInput(effectiveRate(modelRates, "output"), currency),
  };
}

function perMillionToNano(value: string): string {
  const converted = perMillionToNanoPerToken(value);
  if (converted == null) throw new Error("Price must be a non-negative decimal");
  return converted;
}

function safeIdPart(value: string) {
  return value.toLowerCase().replace(/[^a-z0-9._-]+/g, "-");
}

/**
 * UI17a: the sticky right-hand price editor. One instance per selected model;
 * saving writes manual rate rows, deleting the override reveals synced prices.
 */
export function PriceEditorPanel({
  profile,
  model,
  modelRates,
  onRatesChanged,
}: {
  profile: string;
  model: string | null;
  modelRates: BillingRateRecord[];
  onRatesChanged: () => void;
}) {
  const { t } = useTranslation();
  const [currency, setCurrency] = useState<RateCurrency>("CNY");
  const [form, setForm] = useState<OverrideFormState>(EMPTY_FORM);
  const [saving, setSaving] = useState(false);

  // When the target changes, default the currency to the model's manual rates.
  useEffect(() => {
    if (!model) return;
    const manualCurrencies = new Set(
      modelRates.filter((rate) => rate.source === "manual").map((rate) => rate.unit_price_currency)
    );
    const nextCurrency: RateCurrency =
      manualCurrencies.size === 1 ? [...manualCurrencies][0]! : "CNY";
    setCurrency(nextCurrency);
    setForm(prefillForm(modelRates, nextCurrency));
    // modelRates changes identity per fetch; re-prefill only on model/currency change
    // would drop in-progress edits, so re-prefill on model change only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [model, profile]);

  const changeCurrency = (next: RateCurrency) => {
    setCurrency(next);
    setForm(prefillForm(modelRates, next));
  };

  const hasManualOverride = useMemo(
    () => modelRates.some((rate) => rate.source === "manual"),
    [modelRates]
  );

  if (!model) {
    return (
      <EmptyState
        className="py-16"
        title={t("modelMetadata.editor.emptyTitle")}
        description={t("modelMetadata.editor.emptyDescription")}
      />
    );
  }

  const save = async () => {
    setSaving(true);
    try {
      const values: Record<UsageClass, { offPeak: string; peak: string }> = {
        input_uncached: { offPeak: form.input, peak: form.inputPeak },
        cache_read: { offPeak: form.cache, peak: form.cachePeak },
        cache_write_5m: { offPeak: form.cacheWrite5m, peak: form.cacheWrite5mPeak },
        cache_write_1h: { offPeak: form.cacheWrite1h, peak: form.cacheWrite1hPeak },
        output: { offPeak: form.output, peak: form.outputPeak },
      };
      for (const { id: usageClass, required } of VISIBLE_USAGE_CLASSES) {
        const value = values[usageClass].offPeak;
        const peakValue = values[usageClass].peak;
        if (!value.trim() && !required) {
          const existing = modelRates.find(
            (rate) =>
              rate.source === "manual" &&
              rate.pricing_profile === profile &&
              rate.model_pattern === model &&
              rate.usage_class === usageClass
          );
          if (existing) await deleteBillingRateOptimistic(existing.id);
          continue;
        }
        if (!value.trim()) {
          throw new Error(t("modelMetadata.editor.requiredError"));
        }
        const id = `manual:${safeIdPart(profile)}:${safeIdPart(model)}:${usageClass}`;
        await upsertBillingRateOptimistic(
          id,
          {
            source: "manual",
            pricing_profile: profile,
            model_pattern: model,
            provider_type: null,
            rate_kind: "token",
            usage_class: usageClass,
            unit: "token",
            unit_price_nano: perMillionToNano(value),
            // UI19c: write the selected currency, not the server default.
            unit_price_currency: currency,
            // UI19d: cache-write rows carry their TTL for find_rate matching.
            cache_ttl: CACHE_WRITE_TTL[usageClass] ?? null,
            peak_unit_price_nano: peakValue.trim() ? perMillionToNano(peakValue) : null,
            priority: 1000,
            enabled: true,
            match_json: {},
            raw_json: { editor: "model_database" },
          },
          modelRates
        );
      }
      toast.success(t("modelMetadata.editor.saved"));
      onRatesChanged();
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : t("modelMetadata.editor.saveFailed")
      );
    } finally {
      setSaving(false);
    }
  };

  const deleteOverride = async () => {
    const manualRows = modelRates.filter(
      (rate) => rate.source === "manual" && rate.pricing_profile === profile
    );
    for (const row of manualRows) {
      await deleteBillingRateOptimistic(row.id);
    }
    setForm(EMPTY_FORM);
    toast.success(t("modelMetadata.editor.overrideDeleted"));
    onRatesChanged();
  };

  const symbol = currency === "CNY" ? "¥" : "$";

  return (
    <div className="flex h-full flex-col gap-4">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="truncate font-mono text-sm font-semibold">{model}</p>
          <p className="truncate text-xs text-muted-foreground">{profile}</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {hasManualOverride && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void deleteOverride()}
              aria-label={t("modelMetadata.editor.deleteOverride")}
            >
              <Trash2 data-icon className="text-destructive" />
            </Button>
          )}
        </div>
      </div>

      <div className="flex items-center justify-between gap-2">
        <Label htmlFor="price-currency">{t("modelMetadata.editor.currency")}</Label>
        <Select value={currency} onValueChange={(value) => changeCurrency(value as RateCurrency)}>
          <SelectTrigger id="price-currency" className="w-32">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="CNY">CNY (¥)</SelectItem>
            <SelectItem value="USD">USD ($)</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <p className="text-xs text-muted-foreground">
        {t("modelMetadata.editor.helper")}
      </p>

      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
        {VISIBLE_USAGE_CLASSES.map((item) => (
          <div key={item.id} className="rounded-lg border p-3">
            <div className="mb-2 flex items-center gap-2">
              <Label>{item.label}</Label>
              {item.required && (
                <span className="text-xs text-destructive">*</span>
              )}
            </div>
            <div className="grid grid-cols-2 gap-2">
              <div className="flex flex-col gap-1">
                <Label className="text-xs text-muted-foreground">
                  {t("modelMetadata.editor.offPeak")}
                </Label>
                <div className="relative">
                  <span className="absolute left-3 top-1/2 -translate-y-1/2 text-sm text-muted-foreground">
                    {symbol}
                  </span>
                  <Input
                    type="text"
                    inputMode="decimal"
                    className="pl-7"
                    value={form[item.id === "input_uncached" ? "input" : item.id === "cache_read" ? "cache" : item.id === "cache_write_5m" ? "cacheWrite5m" : item.id === "cache_write_1h" ? "cacheWrite1h" : "output"]}
                    onChange={(event) =>
                      setForm((previous) => ({
                        ...previous,
                        [item.id === "input_uncached"
                          ? "input"
                          : item.id === "cache_read"
                            ? "cache"
                            : item.id === "cache_write_5m"
                              ? "cacheWrite5m"
                              : item.id === "cache_write_1h"
                                ? "cacheWrite1h"
                                : "output"]: event.target.value,
                      }))
                    }
                  />
                </div>
              </div>
              <div className="flex flex-col gap-1">
                <Label className="text-xs text-muted-foreground">
                  {t("modelMetadata.editor.peak")}
                </Label>
                <div className="relative">
                  <span className="absolute left-3 top-1/2 -translate-y-1/2 text-sm text-muted-foreground">
                    {symbol}
                  </span>
                  <Input
                    type="text"
                    inputMode="decimal"
                    className="pl-7"
                    value={form[item.id === "input_uncached" ? "inputPeak" : item.id === "cache_read" ? "cachePeak" : item.id === "cache_write_5m" ? "cacheWrite5mPeak" : item.id === "cache_write_1h" ? "cacheWrite1hPeak" : "outputPeak"]}
                    onChange={(event) =>
                      setForm((previous) => ({
                        ...previous,
                        [item.id === "input_uncached"
                          ? "inputPeak"
                          : item.id === "cache_read"
                            ? "cachePeak"
                            : item.id === "cache_write_5m"
                              ? "cacheWrite5mPeak"
                              : item.id === "cache_write_1h"
                                ? "cacheWrite1hPeak"
                                : "outputPeak"]: event.target.value,
                      }))
                    }
                  />
                </div>
              </div>
            </div>
          </div>
        ))}
      </div>

      <div className="flex items-center justify-between gap-2 border-t pt-3">
        <p className="flex items-center gap-1 text-xs text-muted-foreground">
          <Pin className="size-3" />
          {t("modelMetadata.editor.peakHint")}
        </p>
        <Button size="sm" disabled={saving} onClick={() => void save()}>
          {saving && <RefreshCw data-icon className="animate-spin" />}
          {t("common.save")}
        </Button>
      </div>
    </div>
  );
}

/** Skeleton for the editor while rates load. */
export function PriceEditorSkeleton() {
  return (
    <div className="space-y-4">
      <Skeleton className="h-6 w-40" />
      <Skeleton className="h-9 w-full" />
      {Array.from({ length: 5 }).map((_, index) => (
        <Skeleton key={index} className="h-20 w-full rounded-lg" />
      ))}
    </div>
  );
}

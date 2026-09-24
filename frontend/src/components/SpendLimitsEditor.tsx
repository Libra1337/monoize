import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  SPEND_WINDOWS,
  parseSpendLimitRate,
  rederiveField,
  type SpendLimitDraft,
  type SpendWindowKey,
} from "@/lib/spend-limits";

const DRAFT_KEYS: Record<SpendWindowKey, keyof SpendLimitDraft> = {
  total_nano_usd: "total",
  hourly_nano_usd: "hourly",
  daily_nano_usd: "daily",
};

/**
 * ORGL-20: the shared three-window limit editor with a USD/CNY input toggle.
 * Stored windows stay nano-USD; a CNY entry converts through the live
 * cny_per_usd snapshot at save time (exact decimal arithmetic), and the editor
 * shows the rate it will use. Switching currency re-derives the draft fields
 * from the stored nano values when they are supplied.
 */
export function SpendLimitsEditor({
  idPrefix,
  draft,
  currency,
  onCurrencyChange,
  onChange,
  cnyPerUsd,
  storedNano,
  compact = false,
}: {
  idPrefix: string;
  draft: SpendLimitDraft;
  currency: "USD" | "CNY";
  onCurrencyChange: (currency: "USD" | "CNY") => void;
  onChange: (next: SpendLimitDraft) => void;
  cnyPerUsd: string | undefined;
  /** Stored nano-USD strings; used to re-derive the draft after a currency switch. */
  storedNano?: Partial<Record<keyof SpendLimitDraft, string | null | undefined>>;
  compact?: boolean;
}) {
  const { t } = useTranslation();
  const rateAvailable = parseSpendLimitRate(cnyPerUsd) !== undefined;

  const handleCurrencyChange = (next: "USD" | "CNY") => {
    if (next === currency) return;
    onCurrencyChange(next);
    // A currency switch must not lose edits or silently reinterpret blanks.
    // A non-empty input is the user's edit: convert it old-currency → nano →
    // next-currency and keep it. An empty input means "no explicit value":
    // fall back to the stored nano window (which renders empty only when the
    // stored limit is actually unlimited), instead of clearing it to a blank
    // that a later save would submit as an explicit unlimited.
    onChange({
      total: rederiveField(draft.total, storedNano?.total, next, currency, cnyPerUsd),
      hourly: rederiveField(draft.hourly, storedNano?.hourly, next, currency, cnyPerUsd),
      daily: rederiveField(draft.daily, storedNano?.daily, next, currency, cnyPerUsd),
    });
  };

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        {!compact && <span className="text-xs font-medium text-muted-foreground">{t("apiKeys.spendLimits")}</span>}
        <Tabs value={currency} onValueChange={(v) => handleCurrencyChange(v as "USD" | "CNY")}>
          <TabsList className="h-7">
            <TabsTrigger value="USD" className="px-2.5 py-0 text-xs">USD</TabsTrigger>
            <TabsTrigger
              value="CNY"
              className="px-2.5 py-0 text-xs"
              disabled={!rateAvailable}
            >
              CNY
            </TabsTrigger>
          </TabsList>
        </Tabs>
        {currency === "CNY" && rateAvailable && (
          <span className="text-xs text-muted-foreground">
            {t("spendLimitsEditor.rateHint", { rate: cnyPerUsd })}
          </span>
        )}
        {!rateAvailable && (
          <span className="text-xs text-warning">{t("spendLimitsEditor.rateUnavailable")}</span>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {SPEND_WINDOWS.map(({ key, labelKey }) => (
          <label key={key} htmlFor={`${idPrefix}-spend-${key}`} className="flex items-center gap-1 text-xs text-muted-foreground">
            {t(labelKey)}
            <Input
              id={`${idPrefix}-spend-${key}`}
              className="h-7 w-24 text-xs"
              placeholder="∞"
              inputMode="decimal"
              value={draft[DRAFT_KEYS[key]]}
              onChange={(e) => onChange({ ...draft, [DRAFT_KEYS[key]]: e.target.value })}
            />
          </label>
        ))}
      </div>
      <p className="text-xs text-muted-foreground">
        {currency === "CNY" ? t("spendLimitsEditor.cnyHint") : t("apiKeys.spendLimitsHint")}
      </p>
    </div>
  );
}

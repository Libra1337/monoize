import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";

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
import { updatePricingProfilePatternsOptimistic } from "@/lib/swr";
import type { PricingProfilePattern } from "@/lib/api";

/**
 * UI19: the match-rule editor as a first-class dialog. Order matters — the
 * first glob that matches a request's model name decides the billing profile.
 */
export function MatchPatternsDialog({
  open,
  onOpenChange,
  patterns,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  patterns: PricingProfilePattern[];
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<PricingProfilePattern[]>(patterns);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) {
      setDraft(patterns);
      setDirty(false);
    }
  }, [open, patterns]);

  const move = (index: number, direction: -1 | 1) => {
    setDraft((previous) => {
      const next = [...previous];
      const target = index + direction;
      if (target < 0 || target >= next.length) return previous;
      [next[index], next[target]] = [next[target], next[index]];
      return next;
    });
    setDirty(true);
  };

  const save = async () => {
    if (draft.some((pattern) => !pattern.pattern.trim() || !pattern.pricing_profile.trim())) {
      toast.error(t("modelMetadata.patterns.emptyError"));
      return;
    }
    setSaving(true);
    try {
      await updatePricingProfilePatternsOptimistic(draft, patterns);
      setDirty(false);
      toast.success(t("modelMetadata.patterns.saved"));
      onOpenChange(false);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : t("modelMetadata.patterns.saveFailed")
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-hidden rounded-2xl p-0 sm:max-w-2xl">
        <div className="flex max-h-[calc(100dvh-2rem)] flex-col p-5 sm:p-6">
          <DialogHeader className="shrink-0 pr-10">
            <DialogTitle>{t("modelMetadata.patterns.title")}</DialogTitle>
            <DialogDescription className="mt-2 text-pretty">
              {t("modelMetadata.patterns.description")}
            </DialogDescription>
          </DialogHeader>

          <div className="mt-4 min-h-0 flex-1 space-y-2 overflow-y-auto pr-1">
            {draft.length === 0 ? (
              <p className="py-6 text-center text-sm text-muted-foreground">
                {t("modelMetadata.patterns.empty")}
              </p>
            ) : (
              draft.map((pattern, index) => (
                <div
                  key={index}
                  className="grid grid-cols-[2rem_minmax(0,1fr)_minmax(0,12rem)_auto] items-center gap-2"
                >
                  <span className="text-center font-mono text-xs text-muted-foreground">
                    {index + 1}
                  </span>
                  <Input
                    className="font-mono"
                    value={pattern.pattern}
                    placeholder="claude-opus-*"
                    onChange={(event) =>
                      setDraft((previous) =>
                        previous.map((item, i) =>
                          i === index ? { ...item, pattern: event.target.value } : item
                        )
                      )
                    }
                  />
                  <Input
                    className="font-mono"
                    value={pattern.pricing_profile}
                    placeholder="anthropic"
                    onChange={(event) =>
                      setDraft((previous) =>
                        previous.map((item, i) =>
                          i === index ? { ...item, pricing_profile: event.target.value } : item
                        )
                      )
                    }
                  />
                  <div className="flex items-center gap-0.5">
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={index === 0}
                      onClick={() => move(index, -1)}
                      aria-label={t("modelMetadata.patterns.moveUp")}
                    >
                      <ArrowUp data-icon />
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={index === draft.length - 1}
                      onClick={() => move(index, 1)}
                      aria-label={t("modelMetadata.patterns.moveDown")}
                    >
                      <ArrowDown data-icon />
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => {
                        setDraft((previous) => previous.filter((_, i) => i !== index));
                        setDirty(true);
                      }}
                      aria-label={t("common.delete")}
                    >
                      <Trash2 data-icon className="text-destructive" />
                    </Button>
                  </div>
                </div>
              ))
            )}
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setDraft((previous) => [...previous, { pattern: "", pricing_profile: "" }]);
                setDirty(true);
              }}
            >
              <Plus data-icon />
              {t("modelMetadata.patterns.addRule")}
            </Button>
          </div>

          <DialogFooter className="mt-4 shrink-0 border-t pt-4">
            <Button variant="outline" onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button disabled={!dirty || saving} onClick={() => void save()}>
              {saving ? t("common.saving") : t("modelMetadata.patterns.save")}
            </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** UI24: labels for the pattern inputs, exported for tests and reuse. */
export const PATTERN_FIELD_LABELS = { pattern: "Pattern", profile: "Profile" } as const;

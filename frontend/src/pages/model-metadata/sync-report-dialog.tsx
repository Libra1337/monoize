import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export interface SyncReportData {
  /** "models_dev" or "catalog". */
  kind: "models_dev" | "catalog";
  upserted: number;
  skipped: number;
  deleted: number;
  /** UI26: model ids whose manual metadata kept them out of a models.dev sync. */
  retainedManualModels?: string[];
}

/**
 * UI26: post-sync report. The toolbar toast stays small; this dialog carries
 * the counts and the retained-model detail so a manual price set is never
 * silently ignored.
 */
export function SyncReportDialog({
  open,
  onOpenChange,
  report,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  report: SyncReportData | null;
}) {
  const { t } = useTranslation();
  if (!report) return null;

  const isModelsDev = report.kind === "models_dev";
  const retained = report.retainedManualModels ?? [];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-hidden rounded-2xl p-0 sm:max-w-md">
        <div className="flex max-h-[calc(100dvh-2rem)] flex-col p-5 sm:p-6">
          <DialogHeader className="shrink-0">
            <DialogTitle>
              {isModelsDev
                ? t("modelMetadata.sync.reportModelsDev")
                : t("modelMetadata.sync.reportCatalog")}
            </DialogTitle>
            <DialogDescription className="mt-2">
              {t("modelMetadata.sync.reportDescription")}
            </DialogDescription>
          </DialogHeader>

          <div className="mt-4 grid grid-cols-3 gap-3">
            <div className="rounded-lg border p-3 text-center">
              <p className="font-mono text-xl font-semibold text-success">
                {report.upserted}
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("modelMetadata.sync.upserted")}
              </p>
            </div>
            <div className="rounded-lg border p-3 text-center">
              <p className="font-mono text-xl font-semibold">{retained.length}</p>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("modelMetadata.sync.retained")}
              </p>
            </div>
            <div className="rounded-lg border p-3 text-center">
              <p className="font-mono text-xl font-semibold text-muted-foreground">
                {report.deleted}
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("modelMetadata.sync.removed")}
              </p>
            </div>
          </div>

          {isModelsDev && retained.length > 0 && (
            <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
              <p className="mb-2 text-xs font-medium text-muted-foreground">
                {t("modelMetadata.sync.retainedList")}
              </p>
              <ul className="space-y-1">
                {retained.map((model) => (
                  <li
                    key={model}
                    className="rounded-md bg-muted/50 px-2.5 py-1.5 font-mono text-xs"
                  >
                    {model}
                  </li>
                ))}
              </ul>
            </div>
          )}

          <div className="mt-4 flex justify-end border-t pt-4">
            <Button onClick={() => onOpenChange(false)}>{t("common.close")}</Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

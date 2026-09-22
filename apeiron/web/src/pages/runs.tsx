import { useState } from "react";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { ChevronDown, Download, History, XCircle } from "lucide-react";
import { api, assetContentUrl, type Run, type Step } from "@/lib/api";
import { useEventStream } from "@/lib/sse";
import { EmptyState, StatusDot } from "@/components/ui/page";
import { BalanceChip, TopUpButton, WorkHeader } from "@/components/app-shell";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { formatTime, nanoToUsd } from "@/lib/format";

function stepLabel(step: Step): string {
  const shotIndex = step.payload?.shot_index;
  return typeof shotIndex === "number" ? `${step.kind} #${shotIndex + 1}` : step.kind;
}

export function RunsPage() {
  const { t } = useTranslation();
  useEventStream(true);
  const { data, mutate, isLoading } = useSWR("runs", () => api.get<{ runs: Run[] }>("/runs"));
  const [expanded, setExpanded] = useState<string | null>(null);
  const [detail, setDetail] = useState<Record<string, Run>>({});

  async function toggle(runId: string) {
    if (expanded === runId) {
      setExpanded(null);
      return;
    }
    setExpanded(runId);
    if (!detail[runId]) {
      try {
        const run = await api.get<Run>(`/runs/${runId}`);
        setDetail((current) => ({ ...current, [runId]: run }));
      } catch (error) {
        toast.error(String(error));
      }
    }
  }

  const runs = data?.runs ?? [];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <WorkHeader
        title={t("runs.title")}
        description={t("runs.description")}
        actions={
          <>
            <BalanceChip />
            <TopUpButton />
          </>
        }
      />
      <div className="flex-1 overflow-y-auto p-6 pb-24 md:pb-6">
      {isLoading ? (
        <div className="space-y-2">
          {[0, 1, 2].map((index) => (
            <Skeleton key={index} className="h-16" />
          ))}
        </div>
      ) : runs.length === 0 ? (
        <EmptyState icon={History} title={t("runs.empty")} />
      ) : (
        <div className="space-y-2">
          {runs.map((run) => {
            const detailRun = detail[run.id];
            return (
              <div key={run.id} className="rounded-lg border bg-card">
                <button
                  className="flex w-full items-center gap-3 px-4 py-3 text-left"
                  onClick={() => toggle(run.id)}
                >
                  <StatusDot status={run.status} />
                  <span className="font-mono text-xs text-muted-foreground">{run.id.slice(0, 8)}</span>
                  <span className="text-sm font-medium">{t(`status.${run.status}`)}</span>
                  <span className="rounded-md border px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                    {run.kind}
                  </span>
                  <span className="ml-auto font-mono text-xs text-muted-foreground">
                    {run.spend_nano_usd ? nanoToUsd(run.spend_nano_usd) : ""} ·{" "}
                    {formatTime(run.created_at)}
                  </span>
                  <ChevronDown
                    className={cn(
                      "h-4 w-4 text-muted-foreground transition-transform",
                      expanded === run.id && "rotate-180",
                    )}
                  />
                </button>
                {expanded === run.id && detailRun ? (
                  <div className="space-y-4 border-t px-4 py-4">
                    <div className="space-y-1.5">
                      <div className="font-mono text-xs uppercase tracking-wider text-muted-foreground">
                        {t("runs.steps")}
                      </div>
                      {(detailRun.steps ?? []).map((step) => (
                        <div
                          key={step.id}
                          className="flex items-center gap-3 rounded-md border bg-background px-3 py-2 text-xs"
                        >
                          <StatusDot status={step.status} />
                          <span className="font-medium">{stepLabel(step)}</span>
                          <span className="text-muted-foreground">{t(`status.${step.status}`)}</span>
                          {step.charge_nano_usd ? (
                            <span className="ml-auto font-mono text-muted-foreground">
                              {nanoToUsd(step.charge_nano_usd)}
                              {step.refund_nano_usd ? " ↩" : ""}
                            </span>
                          ) : null}
                          {step.error ? (
                            <span className="max-w-72 truncate text-destructive">{step.error}</span>
                          ) : null}
                        </div>
                      ))}
                    </div>
                    {detailRun.output ? (
                      <div className="space-y-2">
                        <div className="font-mono text-xs uppercase tracking-wider text-muted-foreground">
                          {t("runs.output")}
                        </div>
                        <video
                          className="max-w-lg rounded-md border bg-muted"
                          controls
                          src={assetContentUrl(detailRun.output.asset_id)}
                        />
                        <Button variant="outline" size="sm" onClick={() => window.open(assetContentUrl(detailRun.output!.asset_id), "_blank")}>
                          <Download />
                          {t("common.download")}
                        </Button>
                      </div>
                    ) : (
                      <div className="text-xs text-muted-foreground">{t("runs.noOutput")}</div>
                    )}
                    {["queued", "running"].includes(run.status) ? (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={async () => {
                          try {
                            await api.post(`/runs/${run.id}/cancel`);
                            void mutate();
                          } catch (error) {
                            toast.error(String(error));
                          }
                        }}
                      >
                        <XCircle />
                        {t("runs.cancelRun")}
                      </Button>
                    ) : null}
                  </div>
                ) : null}
              </div>
            );
          })}
        </div>
      )}
    </div>
  </div>
  );
}

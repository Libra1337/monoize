import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearchParams } from "react-router-dom";
import { toast } from "sonner";
import useSWR from "swr";
import { CheckCircle2, CircleDashed, Download, Loader2, Sparkles, XCircle } from "lucide-react";
import { api, assetContentUrl, type Run } from "@/lib/api";
import { useEventStream, useRun } from "@/lib/sse";
import { PageWrapper } from "@/components/ui/motion";
import { PageHeader } from "@/components/ui/page";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input, Select, Textarea } from "@/components/ui/input";
import { Label, Switch } from "@/components/ui/controls";
import { Skeleton } from "@/components/ui/badge";
import { nanoToUsd } from "@/lib/format";
import { cn } from "@/lib/utils";

interface Estimate {
  seconds: number;
  shots: number;
  breakdown: { clips: string; tts: string; assemble: string };
  total_nano_usd: string;
}

const VOICES = ["alloy", "ash", "ballad", "coral", "echo", "sage", "shimmer", "verse"];

function StageRow({
  index,
  label,
  state,
}: {
  index: number;
  label: string;
  state: "done" | "active" | "todo" | "failed";
}) {
  return (
    <div className="flex items-center gap-3 py-2">
      {state === "done" ? (
        <CheckCircle2 className="h-4 w-4 text-success" />
      ) : state === "active" ? (
        <Loader2 className="h-4 w-4 animate-spin text-primary" />
      ) : state === "failed" ? (
        <XCircle className="h-4 w-4 text-destructive" />
      ) : (
        <CircleDashed className="h-4 w-4 text-muted-foreground/40" />
      )}
      <span className="font-mono text-xs text-muted-foreground">
        {String(index + 1).padStart(2, "0")}
      </span>
      <span
        className={cn(
          "text-sm",
          state === "todo" ? "text-muted-foreground/60" : "text-foreground",
        )}
      >
        {label}
      </span>
    </div>
  );
}

export function CreatePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  useEventStream(true);

  const [topic, setTopic] = useState(() => searchParams.get("topic") ?? "");
  const [style, setStyle] = useState("");
  const [voice, setVoice] = useState("alloy");
  const [duration, setDuration] = useState(() => {
    const raw = Number(searchParams.get("seconds"));
    return raw >= 5 && raw <= 300 ? raw : 30;
  });
  const [materialMode, setMaterialMode] = useState<"stock" | "ai_image">(() =>
    searchParams.get("mode") === "ai_image" ? "ai_image" : "stock",
  );
  const [subtitle, setSubtitle] = useState(true);
  const [runId, setRunId] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const { data: runData } = useRun(runId);
  const run = runData as Run | undefined;

  const { data: estimate } = useSWR(
    ["estimate", duration, materialMode] as const,
    ([, seconds, mode]) =>
      api.get<Estimate>(`/estimate/oneclick?seconds=${seconds}&mode=${mode}`),
  );

  const terminal = run ? ["succeeded", "failed", "canceled", "partial"].includes(run.status) : false;

  const stages = useMemo(() => {
    const steps = run?.steps ?? [];
    const stateOf = (kind: string, alt?: string) => {
      const matching = steps.filter((step) => step.kind === kind || (alt && step.kind === alt));
      if (matching.length === 0) return "todo" as const;
      if (matching.some((step) => step.status === "running")) return "active" as const;
      if (matching.every((step) => step.status === "succeeded")) return "done" as const;
      if (matching.every((step) => ["failed", "canceled", "skipped"].includes(step.status)))
        return "failed" as const;
      return "active" as const;
    };
    return [
      { label: t("create.stages.script"), state: stateOf("script") },
      { label: t("create.stages.storyboard"), state: stateOf("storyboard") },
      { label: t("create.stages.media"), state: stateOf("material", "image") },
      { label: t("create.stages.voice"), state: stateOf("tts") },
      { label: t("create.stages.subtitle"), state: stateOf("subtitle") },
      { label: t("create.stages.assemble"), state: stateOf("assemble") },
    ];
  }, [run, t]);

  async function submit() {
    if (!topic.trim()) {
      toast.error(t("create.topic"));
      return;
    }
    setSubmitting(true);
    try {
      const created = await api.post<{ project_id: string; run: Run }>("/runs/oneclick", {
        topic,
        style,
        voice,
        duration_target_secs: duration,
        material_mode: materialMode,
        subtitle,
      });
      setRunId(created.run.id);
      toast.success(t("canvas.runStarted"));
    } catch (error) {
      toast.error(String(error));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <PageWrapper className="gap-6">
      <PageHeader title={t("create.title")} description={t("create.description")} />
      <div className="grid gap-6 lg:grid-cols-[1.1fr_0.9fr]">
        <Card>
          <CardHeader>
            <CardTitle>{t("create.title")}</CardTitle>
            <CardDescription>
              {estimate
                ? `${t("create.estimateShots", { shots: estimate.shots })} · ${t("create.estimate")} ${nanoToUsd(estimate.total_nano_usd)}`
                : null}
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="topic">{t("create.topic")}</Label>
              <Textarea
                id="topic"
                rows={4}
                value={topic}
                placeholder={t("create.topicPlaceholder")}
                onChange={(event) => setTopic(event.target.value)}
              />
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="style">{t("create.style")}</Label>
                <Input
                  id="style"
                  value={style}
                  placeholder={t("create.stylePlaceholder")}
                  onChange={(event) => setStyle(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="voice">{t("create.voice")}</Label>
                <Select
                  id="voice"
                  value={voice}
                  onChange={(event) => setVoice(event.target.value)}
                >
                  {VOICES.map((option) => (
                    <option key={option} value={option}>
                      {option}
                    </option>
                  ))}
                </Select>
              </div>
              <div className="space-y-2">
                <Label htmlFor="duration">{t("create.duration")}</Label>
                <Input
                  id="duration"
                  type="number"
                  min={5}
                  max={300}
                  step={5}
                  value={duration}
                  onChange={(event) => setDuration(Number(event.target.value) || 30)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="material">{t("create.materialMode")}</Label>
                <Select
                  id="material"
                  value={materialMode}
                  onChange={(event) => setMaterialMode(event.target.value as "stock" | "ai_image")}
                >
                  <option value="stock">{t("create.stock")}</option>
                  <option value="ai_image">{t("create.aiImage")}</option>
                </Select>
              </div>
            </div>
            <div className="flex items-center justify-between rounded-md border p-3">
              <Label htmlFor="subtitle-switch">{t("create.subtitle")}</Label>
              <Switch
                id="subtitle-switch"
                checked={subtitle}
                onCheckedChange={setSubtitle}
              />
            </div>
            <div className="flex items-center justify-between gap-4 pt-2">
              <div className="text-sm text-muted-foreground">
                {estimate ? (
                  <span className="font-mono">
                    {t("create.estimate")}: {nanoToUsd(estimate.total_nano_usd)}
                  </span>
                ) : (
                  <Skeleton className="h-4 w-28" />
                )}
              </div>
              <Button variant="primary" onClick={submit} disabled={submitting || (!!runId && !terminal)}>
                {submitting ? <Loader2 className="animate-spin" /> : <Sparkles />}
                {submitting ? t("create.submitting") : t("create.submit")}
              </Button>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>
              {!run
                ? t("create.title")
                : terminal
                  ? run.status === "succeeded"
                    ? t("create.doneTitle")
                    : t("create.failedTitle")
                  : t("create.running")}
            </CardTitle>
            {run ? (
              <CardDescription className="font-mono text-xs">
                {run.id}
              </CardDescription>
            ) : null}
          </CardHeader>
          <CardContent className="space-y-2">
            {run ? (
              <>
                {stages.map((stage, index) => (
                  <StageRow key={stage.label} index={index} label={stage.label} state={stage.state} />
                ))}
                {run.error ? (
                  <p className="rounded-md border border-destructive/40 bg-destructive/10 p-3 text-xs text-destructive">
                    {run.error}
                  </p>
                ) : null}
                {terminal && run.status === "succeeded" && run.output ? (
                  <div className="space-y-3 pt-2">
                    <video
                      className="aspect-video w-full rounded-md border bg-muted"
                      controls
                      src={assetContentUrl(run.output.asset_id)}
                    />
                    <div className="flex gap-2">
                      <Button variant="outline" size="sm" onClick={() => window.open(assetContentUrl(run.output!.asset_id), "_blank")}>
                        <Download />
                        {t("common.download")}
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => run.project_id && navigate(`/canvas/${run.project_id}`)}
                      >
                        {t("common.openInCanvas")}
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => setRunId(null)}>
                        {t("create.newFilm")}
                      </Button>
                    </div>
                  </div>
                ) : null}
                {terminal && run.status !== "succeeded" ? (
                  <Button variant="outline" size="sm" onClick={() => setRunId(null)}>
                    {t("create.newFilm")}
                  </Button>
                ) : null}
              </>
            ) : (
              <p className="py-8 text-center text-sm text-muted-foreground">
                {t("create.description")}
              </p>
            )}
          </CardContent>
        </Card>
      </div>
    </PageWrapper>
  );
}

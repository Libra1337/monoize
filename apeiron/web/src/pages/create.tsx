import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearchParams } from "react-router-dom";
import { toast } from "sonner";
import useSWR from "swr";
import { CheckCircle2, Download, Film, Loader2, Sparkles, XCircle } from "lucide-react";
import { api, assetContentUrl, type Run, type Step } from "@/lib/api";
import { useEventStream, useRun } from "@/lib/sse";
import { BalanceChip, TopUpButton, WorkHeader } from "@/components/app-shell";
import { Button } from "@/components/ui/button";
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
const ASPECTS = ["16:9", "9:16", "1:1"] as const;
const STYLES = ["cinematic", "documentary", "animation", "noir"] as const;

/** Visual ratio glyph: a bordered rectangle in the aspect's proportion. */
function RatioGlyph({ aspect }: { aspect: string }) {
  const size =
    aspect === "16:9"
      ? "h-3 w-[21px]"
      : aspect === "9:16"
        ? "h-[21px] w-3"
        : "h-[18px] w-[18px]";
  return <span aria-hidden className={cn("inline-block rounded-[2px] border-2 border-current", size)} />;
}

function shotProgress(steps: Step[] | undefined, count: number) {
  const media = (steps ?? [])
    .filter((step) => step.kind === "material" || step.kind === "image")
    .sort(
      (a, b) =>
        Number(a.payload?.shot_index ?? 0) - Number(b.payload?.shot_index ?? 0),
    );
  const tiles = Array.from({ length: count }, (_, index) => {
    const step = media.find((s) => Number(s.payload?.shot_index) === index);
    return {
      index,
      status: step?.status ?? ("pending" as const),
      narration: String((step?.payload?.shot as Record<string, unknown> | undefined)?.narration ?? ""),
      assetId: (step?.result?.asset_id as string) ?? null,
      mime: (step?.result?.mime_type as string) ?? null,
    };
  });
  const done = tiles.filter((tile) => tile.status === "succeeded").length;
  return { tiles, done };
}

/**
 * Professional creation layout: parameter panel left, a persistent preview
 * monitor (fixed aspect frame) center — storyboard tiles fill the frame
 * while rendering, the finished film plays inside it.
 */
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
  const [aspect, setAspect] = useState<(typeof ASPECTS)[number]>("16:9");
  const [subtitle, setSubtitle] = useState(true);
  const [runId, setRunId] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [continuing, setContinuing] = useState(false);
  // AP-AG1: stage-gated mode — run stops after storyboard for review.
  const [stageGate, setStageGate] = useState(true);
  const [reviewProjectId, setReviewProjectId] = useState<string | null>(null);
  const [editedShots, setEditedShots] = useState<
    Record<number, { description: string; keywords: string; duration_secs?: number }>
  >({});

  const { data: runData } = useRun(runId);
  const run = runData as Run | undefined;
  const terminal = run ? ["succeeded", "failed", "canceled", "partial"].includes(run.status) : false;

  const { data: estimate } = useSWR(
    ["estimate", duration, materialMode] as const,
    ([, seconds, mode]) => api.get<Estimate>(`/estimate/oneclick?seconds=${seconds}&mode=${mode}`),
  );
  const shotCount = estimate?.shots ?? Math.ceil(duration / 5);
  const { tiles, done } = shotProgress(run?.steps, shotCount);

  /** Storyboard shots from the run, merged with local edits (AP-AG1). */
  const boardShots = useMemo(() => {
    const board = (run?.steps ?? []).find((step) => step.kind === "storyboard");
    const shots = (board?.result?.shots as Array<Record<string, unknown>>) ?? [];
    return shots.map((shot, index) => {
      const edit = editedShots[index];
      return {
        index,
        description: edit?.description ?? String(shot.description ?? ""),
        keywords: edit?.keywords ?? String(shot.keywords ?? ""),
        narration: String(shot.narration ?? ""),
        duration_secs: edit?.duration_secs ?? Number(shot.duration_secs ?? 5),
        materialCandidates: shot.material_candidates as number | null | undefined,
      };
    });
  }, [run, editedShots]);

  const runStopped =
    terminal &&
    (run?.params as Record<string, unknown> | undefined)?.stop_after === "storyboard";

  const frameClass =
    aspect === "16:9"
      ? "aspect-video"
      : aspect === "9:16"
        ? "aspect-[9/16] max-h-full"
        : "aspect-square max-h-full";

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
        aspect,
        stop_after: stageGate ? "storyboard" : null,
      });
      if (stageGate) setReviewProjectId(created.project_id);
      setRunId(created.run.id);
      toast.success(t("canvas.runStarted"));
    } catch (error) {
      toast.error(String(error));
    } finally {
      setSubmitting(false);
    }
  }

  async function continueProduction() {
    if (!reviewProjectId) return;
    setContinuing(true);
    try {
      const project = await api.get<import("@/lib/api").Project>(
        `/projects/${reviewProjectId}`,
      );
      const graph = { ...project.graph, version: undefined };
      const board = (graph.nodes ?? []).find((node) => node.id === "board");
      // Persist per-shot edits into the storyboard node's fan-out payload:
      // material steps read shot keywords/description at dispatch.
      if (board) {
        board.params = {
          ...(board.params ?? {}),
          shot_overrides: editedShots,
        };
      }
      await api.put(`/projects/${reviewProjectId}/graph`, {
        version: project.version + 1,
        graph,
      });
      const started = await api.post<Run>("/runs/continue", {
        project_id: reviewProjectId,
      });
      setRunId(started.id);
      toast.success(t("canvas.runStarted"));
    } catch (error) {
      toast.error(String(error));
    } finally {
      setContinuing(false);
    }
  }

  return (
    <>
      <WorkHeader
        title={t("create.title")}
        description={t("create.description")}
        actions={
          <>
            <BalanceChip />
            <TopUpButton />
          </>
        }
      />
      <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
        {/* Parameter panel */}
        <aside className="w-full shrink-0 space-y-6 overflow-y-auto border-b p-5 pb-24 lg:w-[320px] lg:border-b-0 lg:border-r lg:pb-5">
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

          <div className="space-y-2">
            <Label>{t("create.style")}</Label>
            <div className="flex flex-wrap gap-1.5">
              {STYLES.map((preset) => (
                <button
                  key={preset}
                  type="button"
                  aria-pressed={style === preset}
                  onClick={() => setStyle(style === preset ? "" : preset)}
                  className={
                    style === preset
                      ? "rounded-md bg-accent px-2.5 py-1 text-xs font-medium text-accent-foreground"
                      : "rounded-md border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
                  }
                >
                  {t(`create.styles.${preset}`)}
                </button>
              ))}
            </div>
            <Input
              aria-label={t("create.style")}
              value={style}
              placeholder={t("create.stylePlaceholder")}
              onChange={(event) => setStyle(event.target.value)}
            />
          </div>

          <div className="space-y-2">
            <Label>{t("create.aspect")}</Label>
            <div className="grid grid-cols-3 gap-1.5">
              {ASPECTS.map((option) => (
                <button
                  key={option}
                  type="button"
                  aria-pressed={aspect === option}
                  onClick={() => setAspect(option)}
                  className={
                    aspect === option
                      ? "flex flex-col items-center gap-1.5 rounded-md border border-primary/60 bg-accent py-2.5 text-xs font-medium text-accent-foreground"
                      : "flex flex-col items-center gap-1.5 rounded-md border py-2.5 text-xs text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
                  }
                >
                  <RatioGlyph aspect={option} />
                  {option}
                </button>
              ))}
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-2">
              <Label htmlFor="voice">{t("create.voice")}</Label>
              <Select id="voice" value={voice} onChange={(event) => setVoice(event.target.value)}>
                {VOICES.map((option) => (
                  <option key={option}>{option}</option>
                ))}
              </Select>
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

          <div className="grid grid-cols-2 items-end gap-3">
            <div className="space-y-2">
              <Label htmlFor="duration">{t("create.duration")}</Label>
              <div className="flex gap-1">
                {[15, 30, 60].map((option) => (
                  <button
                    key={option}
                    type="button"
                    aria-pressed={duration === option}
                    onClick={() => setDuration(option)}
                    className={
                      duration === option
                        ? "h-8 flex-1 rounded-md bg-accent font-mono text-xs font-medium text-accent-foreground"
                        : "h-8 flex-1 rounded-md border font-mono text-xs text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
                    }
                  >
                    {option}s
                  </button>
                ))}
              </div>
            </div>
            <div className="flex items-center justify-between rounded-md border px-3 py-1.5">
              <Label htmlFor="subtitle-switch" className="text-xs">{t("create.subtitle")}</Label>
              <Switch id="subtitle-switch" checked={subtitle} onCheckedChange={setSubtitle} />
            </div>

            <div className="flex items-center justify-between rounded-md border px-3 py-1.5">
              <Label htmlFor="gate-switch" className="text-xs">{t("create.stageGate")}</Label>
              <Switch id="gate-switch" checked={stageGate} onCheckedChange={setStageGate} />
            </div>
          </div>

          <div className="sticky bottom-0 flex items-center justify-between gap-2 border-t bg-background pt-4">
            <span className="font-mono text-xs text-muted-foreground">
              {estimate ? (
                <>
                  {t("create.estimateShots", { shots: estimate.shots })} ·{" "}
                  {nanoToUsd(estimate.total_nano_usd)}
                </>
              ) : (
                <Skeleton className="h-4 w-24" />
              )}
            </span>
            <Button variant="primary" onClick={submit} disabled={submitting || (!!runId && !terminal)}>
              {submitting ? <Loader2 className="animate-spin" /> : <Sparkles />}
              {submitting ? t("create.submitting") : t("create.submit")}
            </Button>
          </div>
        </aside>

        {/* Preview monitor */}
        <section className="flex min-h-0 min-w-0 flex-1 flex-col items-center gap-4 overflow-y-auto p-6 pb-24 lg:pb-6">
          <div className={cn("relative w-full max-w-3xl", aspect === "16:9" ? "" : "self-center")}>
            <div
              className={cn(
                "relative flex w-full items-center justify-center overflow-hidden rounded-lg border bg-muted/30",
                frameClass,
              )}
            >
              {/* AP-AG1: storyboard review — edit shots before continuing */}
              {runStopped && boardShots.length > 0 ? (
                <div className="flex h-full w-full flex-col">
                  <div className="border-b px-4 py-2 text-xs font-medium text-muted-foreground">
                    {t("create.reviewHint")}
                  </div>
                  <div className="grid flex-1 content-start gap-2 overflow-y-auto p-4 sm:grid-cols-2">
                    {boardShots.map((shot) => (
                      <div key={shot.index} className="space-y-1.5 rounded-md border bg-card p-3">
                        <div className="flex items-center gap-2 font-mono text-[10px] text-muted-foreground">
                          <span>{String(shot.index + 1).padStart(2, "0")}</span>
                          <input
                            aria-label={`duration-${shot.index}`}
                            type="number"
                            min={2}
                            max={30}
                            value={shot.duration_secs}
                            onChange={(event) =>
                              setEditedShots((current) => ({
                                ...current,
                                [shot.index]: {
                                  description: shot.description,
                                  keywords: shot.keywords,
                                  duration_secs: Math.max(2, Math.min(30, Number(event.target.value) || 5)),
                                },
                              }))
                            }
                            className="h-5 w-11 rounded border bg-transparent px-1 font-mono text-[10px] focus-visible:outline-none"
                          />
                          <span>s</span>
                        </div>
                        <input
                          aria-label={`description-${shot.index}`}
                          className="w-full rounded border bg-transparent px-2 py-1 text-xs focus-visible:outline-none"
                          value={shot.description}
                          placeholder={t("create.shotDescription")}
                          onChange={(event) =>
                            setEditedShots((current) => ({
                              ...current,
                              [shot.index]: {
                                description: event.target.value,
                                keywords: shot.keywords,
                              },
                            }))
                          }
                        />
                        <input
                          aria-label={`keywords-${shot.index}`}
                          className="w-full rounded border bg-transparent px-2 py-1 font-mono text-[11px] text-muted-foreground focus-visible:outline-none"
                          value={shot.keywords}
                          placeholder={t("create.shotKeywords")}
                          onChange={(event) =>
                            setEditedShots((current) => ({
                              ...current,
                              [shot.index]: {
                                description: shot.description,
                                keywords: event.target.value,
                              },
                            }))
                          }
                        />
                        <p className="line-clamp-2 text-[11px] leading-relaxed text-muted-foreground/80">
                          {shot.narration}
                        </p>
                        {shot.materialCandidates === 0 ? (
                          <span className="inline-flex items-center rounded border border-warning/50 bg-warning-soft px-1.5 py-0.5 text-[10px] text-warning-foreground">
                            {t("create.noMaterial")}
                          </span>
                        ) : typeof shot.materialCandidates === "number" ? (
                          <span className="font-mono text-[10px] text-muted-foreground/60">
                            ~{shot.materialCandidates} clips
                          </span>
                        ) : null}
                      </div>
                    ))}
                  </div>
                </div>
              ) : terminal && run?.status === "succeeded" && run.output ? (
                <video
                  className="h-full w-full"
                  controls
                  autoPlay
                  src={assetContentUrl(run.output.asset_id)}
                />
              ) : run ? (
                <div className="flex h-full w-full flex-col">
                  {/* Render progress bar */}
                  <div className="h-1 w-full bg-muted">
                    <div
                      className="h-full bg-primary transition-all"
                      style={{ width: `${Math.round((done / Math.max(shotCount, 1)) * 100)}%` }}
                    />
                  </div>
                  {/* Storyboard tiles */}
                  <div className="grid flex-1 grid-cols-3 content-start gap-2 overflow-y-auto p-4 sm:grid-cols-4">
                    {tiles.map((tile) => (
                      <div
                        key={tile.index}
                        className="flex aspect-video flex-col items-center justify-center gap-1 rounded-md border bg-card p-1.5 text-center"
                      >
                        {tile.status === "succeeded" ? (
                          tile.assetId ? (
                            <img
                              src={assetContentUrl(tile.assetId)}
                              alt=""
                              className="h-full w-full rounded object-cover"
                            />
                          ) : (
                            <CheckCircle2 className="h-4 w-4 text-success" />
                          )
                        ) : tile.status === "running" ? (
                          <Loader2 className="h-4 w-4 animate-spin text-primary" />
                        ) : tile.status === "failed" || tile.status === "canceled" ? (
                          <XCircle className="h-4 w-4 text-destructive" />
                        ) : (
                          <span className="font-mono text-xs text-muted-foreground/50">
                            {String(tile.index + 1).padStart(2, "0")}
                          </span>
                        )}
                        {tile.status !== "succeeded" || !tile.assetId ? (
                          <span className="line-clamp-2 text-[10px] leading-tight text-muted-foreground/70">
                            {tile.narration}
                          </span>
                        ) : null}
                      </div>
                    ))}
                  </div>
                </div>
              ) : (
                /* Empty state inside the frame */
                <div className="flex flex-col items-center gap-3 text-muted-foreground/60">
                  <Film className="h-8 w-8" />
                  <p className="max-w-xs px-6 text-center text-xs leading-relaxed">
                    {t("create.stageEmpty")}
                  </p>
                </div>
              )}
            </div>

            {/* Monitor action row */}
            <div className="mt-3 flex items-center justify-center gap-2">
              {run?.error ? (
                <p className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-1.5 text-xs text-destructive">
                  {run.error}
                </p>
              ) : null}
              {terminal && run?.status === "succeeded" && run.output ? (
                <>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => window.open(assetContentUrl(run.output!.asset_id), "_blank")}
                  >
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
                </>
              ) : runStopped ? (
                <>
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={continueProduction}
                    disabled={continuing}
                  >
                    {continuing ? <Loader2 className="animate-spin" /> : <Sparkles />}
                    {t("create.continue")}
                  </Button>
                  <Button variant="ghost" size="sm" onClick={() => setRunId(null)}>
                    {t("create.newFilm")}
                  </Button>
                </>
              ) : terminal ? (
                <Button variant="outline" size="sm" onClick={() => setRunId(null)}>
                  {t("create.newFilm")}
                </Button>
              ) : null}
            </div>
          </div>
        </section>
      </div>
    </>
  );
}

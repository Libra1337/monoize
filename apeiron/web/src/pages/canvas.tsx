import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useParams } from "react-router-dom";
import { toast } from "sonner";
import useSWR from "swr";
import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  addEdge,
  useEdgesState,
  useNodesState,
  type Connection,
  type Edge,
  type Node,
  type NodeProps,
  type ReactFlowInstance,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { ArrowLeft, Loader2, MessageSquare, PanelLeft, Play, Send, Trash2 } from "lucide-react";
import { api, assetContentUrl, type Graph, type Project, type Run, type Step } from "@/lib/api";
import { useEventStream, useRun } from "@/lib/sse";
import { NODE_META } from "@/lib/nodes";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input, Select, Textarea } from "@/components/ui/input";
import { Label } from "@/components/ui/controls";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

type FlowNode = Node<Record<string, unknown>, string>;

interface CanvasNodeData {
  kind: string;
  params: Record<string, unknown>;
  step?: Step | null;
  [key: string]: unknown;
}

function statusBorder(status: string | undefined): string {
  switch (status) {
    case "running":
      return "border-primary/60";
    case "succeeded":
      return "border-success/60";
    case "failed":
      return "border-destructive/60";
    case "canceled":
    case "skipped":
      return "border-muted-foreground/30 border-dashed";
    default:
      return "border-border";
  }
}

function ResultPreview({ step }: { step?: Step | null }) {
  if (!step?.result?.asset_id) return null;
  const kind = step.result.mime_type as string;
  if (typeof kind === "string" && kind.startsWith("image/")) {
    return (
      <img
        src={assetContentUrl(String(step.result.asset_id))}
        alt=""
        className="mt-2 aspect-video w-full rounded-md border object-cover"
      />
    );
  }
  if (typeof kind === "string" && kind.startsWith("video/")) {
    return (
      <video
        src={assetContentUrl(String(step.result.asset_id))}
        controls
        className="mt-2 aspect-video w-full rounded-md border bg-muted"
      />
    );
  }
  if (typeof kind === "string" && kind.startsWith("audio/")) {
    return (
      <audio
        src={assetContentUrl(String(step.result.asset_id))}
        controls
        className="mt-2 w-full"
      />
    );
  }
  return null;
}

function CanvasNodeCard({ data, selected }: NodeProps<FlowNode>) {
  const nodeData = data as CanvasNodeData;
  const meta = NODE_META[nodeData.kind];
  if (!meta) return null;
  const Icon = meta.icon;
  if (nodeData.kind === "note") {
    return (
      <Card className={cn("w-56 border-dashed p-3", selected && "ring-1 ring-ring")}>
        <div className="text-xs leading-relaxed text-muted-foreground">
          {String(nodeData.params.text ?? "")}
        </div>
      </Card>
    );
  }
  const step = nodeData.step;
  const inputPorts = meta.inputPorts;
  return (
    <Card
      className={cn(
        "relative w-64 p-3",
        statusBorder(step?.status),
        selected && "ring-1 ring-ring",
      )}
    >
      {inputPorts.map((port, index) => (
        <Handle
          key={port}
          type="target"
          position={Position.Left}
          id={port}
          style={{ top: `${((index + 1) * 100) / (inputPorts.length + 1)}%` }}
        />
      ))}
      {meta.outputPort ? (
        <Handle type="source" position={Position.Right} id={meta.outputPort} />
      ) : null}
      <div className="flex items-center gap-2">
        <span className="flex size-6 items-center justify-center rounded-md bg-muted">
          <Icon className="h-3.5 w-3.5 text-muted-foreground" />
        </span>
        <span className="text-xs font-semibold">{meta.kind}</span>
        {step?.status === "running" ? (
          <Loader2 className="ml-auto h-3.5 w-3.5 animate-spin text-primary" />
        ) : null}
      </div>
      {nodeData.kind === "script" ? (
        <div className="mt-2 line-clamp-3 text-xs leading-relaxed text-muted-foreground">
          {String(nodeData.params.prompt ?? "")}
        </div>
      ) : null}
      {step?.result?.text ? (
        <div className="mt-2 line-clamp-3 text-xs leading-relaxed text-muted-foreground">
          {String(step.result.text).slice(0, 160)}
        </div>
      ) : null}
      {Array.isArray(step?.result?.shots) ? (
        <div className="mt-2 font-mono text-[10px] text-muted-foreground">
          {(step?.result?.shots as unknown[]).length} shots
        </div>
      ) : null}
      <ResultPreview step={step} />
      {step?.error ? (
        <div className="mt-2 line-clamp-2 text-[10px] text-destructive">{step.error}</div>
      ) : null}
    </Card>
  );
}

const nodeTypes = { apeiron: CanvasNodeCard };

export function CanvasPage() {
  const { projectId } = useParams<{ projectId: string }>();
  const { t } = useTranslation();
  const navigate = useNavigate();
  useEventStream(true);

  const { data: project, mutate: reloadProject } = useSWR(
    projectId ? (["project", projectId] as const) : null,
    ([, id]) => api.get<Project>(`/projects/${id}`),
  );

  const [nodes, setNodes, onNodesChange] = useNodesState<FlowNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const [version, setVersion] = useState(1);
  const [saveState, setSaveState] = useState<"idle" | "pending" | "saved">("idle");
  const [inspectId, setInspectId] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(true);
  const [chatOpen, setChatOpen] = useState(false);
  const [chatInput, setChatInput] = useState("");
  const [chatSending, setChatSending] = useState(false);
  const [chatMessages, setChatMessages] = useState<Array<{ role: "user" | "assistant"; text: string }>>([]);
  const [runId, setRunId] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const instanceRef = useRef<ReactFlowInstance<FlowNode> | null>(null);
  const dirtyRef = useRef(false);
  const loadedRef = useRef<string | null>(null);

  // Load the project graph once per project revision from the server.
  useEffect(() => {
    if (!project || loadedRef.current === project.id + ":" + project.version) return;
    loadedRef.current = project.id + ":" + project.version;
    const graph = project.graph;
    setNodes(
      (graph.nodes ?? []).map((node) => ({
        id: node.id,
        type: "apeiron",
        position: { x: node.x, y: node.y },
        data: { kind: node.kind, params: node.params ?? {} },
      })),
    );
    setEdges(
      (graph.edges ?? []).map((edge) => ({
        id: edge.id,
        source: edge.source,
        sourceHandle: edge.sourcePort,
        target: edge.target,
        targetHandle: edge.targetPort,
      })),
    );
    setVersion(project.version);
  }, [project, setNodes, setEdges]);

  // Track the newest run for this project so node cards surface step state.
  const { data: runs } = useSWR("runs", () => api.get<{ runs: Run[] }>("/runs"));
  useEffect(() => {
    const mine = (runs?.runs ?? []).filter((run) => run.project_id === projectId);
    const active = mine.find((run) => ["queued", "running"].includes(run.status));
    const latest = active ?? mine[0];
    if (latest) setRunId(latest.id);
  }, [runs, projectId]);
  const { data: runData } = useRun(runId);
  const run = runData as Run | undefined;
  const terminal = run ? ["succeeded", "failed", "canceled", "partial"].includes(run.status) : false;
  useEffect(() => {
    if (terminal) setRunning(false);
  }, [terminal]);

  // Mirror step state into node cards.
  const stepsByNode = useMemo(() => {
    const map = new Map<string, Step>();
    for (const step of run?.steps ?? []) {
      const existing = map.get(step.node_id);
      if (!existing || existing.updated_at <= step.updated_at) map.set(step.node_id, step);
    }
    return map;
  }, [run]);
  useEffect(() => {
    setNodes((current) =>
      current.map((node) => ({
        ...node,
        data: { ...node.data, step: stepsByNode.get(node.id) ?? null },
      })),
    );
  }, [stepsByNode, setNodes]);

  const buildGraph = useCallback(
    (): Graph => ({
      nodes: nodes.map((node) => {
        const data = node.data as CanvasNodeData;
        return {
          id: node.id,
          kind: data.kind,
          x: node.position.x,
          y: node.position.y,
          params: data.params ?? {},
        };
      }),
      edges: edges.map((edge) => ({
        id: edge.id,
        source: edge.source,
        sourcePort: edge.sourceHandle ?? "",
        target: edge.target,
        targetPort: edge.targetHandle ?? "",
      })),
    }),
    [nodes, edges],
  );

  // Debounced autosave (AP-U5).
  const saveTimer = useRef<number | null>(null);
  const scheduleSave = useCallback(() => {
    if (!projectId) return;
    dirtyRef.current = true;
    setSaveState("pending");
    if (saveTimer.current) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(async () => {
      try {
        const result = await api.put<{ version: number }>(`/projects/${projectId}/graph`, {
          version: version + 1,
          graph: buildGraph(),
        });
        setVersion(result.version);
        setSaveState("saved");
        dirtyRef.current = false;
      } catch (error) {
        const response = error as { status?: number };
        if (response.status === 409) {
          toast.error(t("canvas.versionConflict"));
          loadedRef.current = null;
          void reloadProject();
        } else {
          toast.error(String(error));
        }
        setSaveState("idle");
      }
    }, 800);
  }, [projectId, version, buildGraph, reloadProject, t]);

  useEffect(() => {
    return () => {
      if (saveTimer.current) window.clearTimeout(saveTimer.current);
    };
  }, []);

  const onConnect = useCallback(
    (connection: Connection) => {
      const source = nodes.find((node) => node.id === connection.source);
      const target = nodes.find((node) => node.id === connection.target);
      if (!source || !target) return;
      const sourceKind = (source.data as CanvasNodeData).kind;
      const targetKind = (target.data as CanvasNodeData).kind;
      const sourcePort = NODE_META[sourceKind]?.outputPort ?? null;
      if (!sourcePort) return;
      const targetPort =
        (connection.targetHandle as string | null) ?? NODE_META[targetKind]?.inputPorts[0] ?? null;
      if (!targetPort) return;
      const compatible =
        targetPort === "text"
          ? sourcePort === "text" || sourcePort === "shots"
          : targetPort === "clips"
            ? sourcePort === "image" || sourcePort === "video"
            : targetPort === sourcePort;
      if (!compatible) {
        toast.error(`${sourcePort} → ${targetPort}: incompatible`);
        return;
      }
      setEdges((current) =>
        addEdge(
          {
            ...connection,
            id: `e-${connection.source}-${sourcePort}-${connection.target}-${targetPort}-${Date.now()}`,
            sourceHandle: sourcePort,
            targetHandle: targetPort,
          },
          current,
        ),
      );
      scheduleSave();
    },
    [nodes, setEdges, scheduleSave],
  );

  function addNode(kind: string) {
    const id = `${kind}-${Math.random().toString(36).slice(2, 8)}`;
    const meta = NODE_META[kind];
    const defaults: Record<string, unknown> =
      kind === "storyboard"
        ? { count: 6 }
        : kind === "image"
          ? { size: "1280x720" }
          : kind === "video"
            ? { seconds: "5", size: "1280x720" }
            : kind === "tts"
              ? { voice: "alloy", speed: 1 }
              : kind === "assemble"
                ? { resolution: "1280x720", fps: 30 }
                : {};
    setNodes((current) => [
      ...current,
      {
        id,
        type: "apeiron",
        position: {
          x: 200 + Math.random() * 260,
          y: 160 + Math.random() * 220,
        },
        data: { kind, params: defaults, _label: meta?.kind },
      },
    ]);
    scheduleSave();
  }

  async function runGraph() {
    if (!projectId) return;
    if (dirtyRef.current || saveState === "pending") {
      // flush pending edits first so the run sees the drawn graph
      await api
        .put<{ version: number }>(`/projects/${projectId}/graph`, {
          version: version + 1,
          graph: buildGraph(),
        })
        .then((result) => setVersion(result.version))
        .catch(() => reloadProject());
      dirtyRef.current = false;
    }
    setRunning(true);
    try {
      const created = await api.post<Run>(`/projects/${projectId}/run`, {});
      setRunId(created.id);
      toast.success(t("canvas.runStarted"));
    } catch (error) {
      toast.error(String(error));
      setRunning(false);
    }
  }

  const inspectNode = nodes.find((node) => node.id === inspectId);
  const inspectData = inspectNode ? (inspectNode.data as CanvasNodeData) : null;
  const inspectMeta = inspectData ? NODE_META[inspectData.kind] : null;

  function updateParam(key: string, value: unknown) {
    if (!inspectId) return;
    setNodes((current) =>
      current.map((node) =>
        node.id === inspectId
          ? {
              ...node,
              data: {
                ...node.data,
                params: { ...(node.data as CanvasNodeData).params, [key]: value },
              },
            }
          : node,
      ),
    );
    scheduleSave();
  }

  if (!project) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <Skeleton className="h-8 w-64" />
      </div>
    );
  }

  async function sendAgentMessage() {
    const message = chatInput.trim();
    if (!message || !projectId || chatSending) return;
    setChatMessages((current) => [...current, { role: "user", text: message }]);
    setChatInput("");
    setChatSending(true);
    try {
      const response = await api.post<{ message: string; applied: Array<Record<string, unknown>> }>(
        `/projects/${projectId}/agent`,
        { message },
      );
      const applied = (response.applied ?? []).map((call) => String(call.tool ?? "?")).join(", ");
      setChatMessages((current) => [
        ...current,
        {
          role: "assistant",
          text: response.message || (applied ? `✓ ${applied}` : "…"),
        },
      ]);
      if ((response.applied ?? []).length > 0) {
        dirtyRef.current = false;
        loadedRef.current = null;
        void reloadProject();
      }
    } catch (error) {
      setChatMessages((current) => [
        ...current,
        { role: "assistant", text: `⚠ ${String(error)}` },
      ]);
    } finally {
      setChatSending(false);
    }
  }

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      {/* Floating toolbar (ComfyUI-style) over the full-bleed canvas */}
      <div className="pointer-events-none absolute inset-x-3 top-3 z-20 flex items-center gap-2">
        <div className="pointer-events-auto flex min-w-0 items-center gap-2 rounded-lg border bg-card/95 px-2.5 py-1.5 shadow-sm backdrop-blur">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => navigate("/projects")}
            aria-label="back"
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <span className="truncate text-sm font-medium">{project.title}</span>
          <span className="font-mono text-xs text-muted-foreground">
            {saveState === "pending" ? t("canvas.savePending") : `v${version}`}
          </span>
        </div>
        <div className="pointer-events-auto ml-auto flex items-center gap-1.5 rounded-lg border bg-card/95 px-2 py-1.5 shadow-sm backdrop-blur">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => setPaletteOpen((open) => !open)}
            aria-label={t("canvas.palette")}
          >
            <PanelLeft className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => setChatOpen((open) => !open)}
            aria-label={t("canvas.agent")}
          >
            <MessageSquare className="h-4 w-4" />
          </Button>
          <Button variant="primary" size="sm" className="h-7" onClick={runGraph} disabled={running}>
            {running ? <Loader2 className="animate-spin" /> : <Play />}
            {running ? t("canvas.running") : t("canvas.run")}
          </Button>
        </div>
      </div>

      {/* Node palette */}
      {paletteOpen ? (
        <aside className="absolute left-3 top-16 z-20 flex w-44 flex-col gap-1 rounded-lg border bg-card/95 p-2 shadow-sm backdrop-blur">
          <div className="px-2 pb-1 font-mono text-xs uppercase tracking-wider text-muted-foreground">
            {t("canvas.palette")}
          </div>
          {Object.entries(NODE_META).map(([kind, meta]) => (
            <button
              key={kind}
              onClick={() => addNode(kind)}
              className="flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-muted-foreground transition-colors hover:bg-accent/60 hover:text-foreground"
            >
              <meta.icon className="h-3.5 w-3.5" />
              {meta.kind}
            </button>
          ))}
          <div className="px-2 pt-2 text-[10px] leading-relaxed text-muted-foreground/70">
            {t("canvas.hint")}
          </div>
        </aside>
      ) : null}

      {/* Full-bleed canvas */}
      <div className="relative min-h-0 flex-1 overflow-hidden bg-background">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={(changes) => {
            onNodesChange(changes);
            if (changes.some((change) => change.type !== "select")) scheduleSave();
          }}
          onEdgesChange={(changes) => {
            onEdgesChange(changes);
            if (changes.some((change) => change.type !== "select")) scheduleSave();
          }}
          onConnect={onConnect}
          onInit={(instance) => {
            instanceRef.current = instance;
          }}
          onNodeDoubleClick={(_, node) => setInspectId(node.id)}
          onNodesDelete={() => scheduleSave()}
          fitView
          proOptions={{ hideAttribution: true }}
        >
          <Background variant={BackgroundVariant.Dots} gap={20} className="bg-muted/40" />
          <Controls showInteractive={false} />
          <MiniMap pannable zoomable />
        </ReactFlow>
        {running && !terminal ? (
          <div className="absolute left-1/2 top-16 z-10 -translate-x-1/2 rounded-full border bg-card px-3 py-1 font-mono text-xs text-primary shadow-sm">
            {t("canvas.running")}
          </div>
        ) : null}
      </div>

      {/* AP-AG2: conversational graph agent panel */}
      {chatOpen ? (
        <aside className="absolute bottom-3 right-3 top-16 z-20 flex w-80 flex-col rounded-lg border bg-card/95 shadow-sm backdrop-blur">
          <div className="flex items-center justify-between border-b px-3 py-2">
            <span className="text-xs font-medium">{t("canvas.agent")}</span>
          </div>
          <div className="flex-1 space-y-2 overflow-y-auto p-3">
            {chatMessages.length === 0 ? (
              <p className="text-[11px] leading-relaxed text-muted-foreground/70">
                {t("canvas.agentHint")}
              </p>
            ) : (
              chatMessages.map((entry, index) => (
                <div
                  key={index}
                  className={
                    entry.role === "user"
                      ? "ml-6 rounded-md bg-primary/10 px-2.5 py-1.5 text-xs"
                      : "mr-6 rounded-md border px-2.5 py-1.5 text-xs"
                  }
                >
                  {entry.text}
                </div>
              ))
            )}
            {chatSending ? (
              <div className="mr-6 flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-xs text-muted-foreground">
                <Loader2 className="h-3 w-3 animate-spin" />
              </div>
            ) : null}
          </div>
          <form
            className="flex gap-1.5 border-t p-2"
            onSubmit={(event) => {
              event.preventDefault();
              void sendAgentMessage();
            }}
          >
            <input
              value={chatInput}
              onChange={(event) => setChatInput(event.target.value)}
              placeholder={t("canvas.agentPlaceholder")}
              className="h-8 w-full rounded-md border bg-transparent px-2 text-xs focus-visible:outline-none"
            />
            <Button type="submit" variant="primary" size="sm" className="h-8" disabled={chatSending}>
              <Send className="h-3.5 w-3.5" />
            </Button>
          </form>
        </aside>
      ) : null}

      <Dialog open={!!inspectId} onOpenChange={(open) => !open && setInspectId(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t("canvas.inspector")} · {inspectMeta?.kind ?? ""}
            </DialogTitle>
          </DialogHeader>
          {inspectData && inspectMeta ? (
            <div className="space-y-4">
              {inspectMeta.fields.map((field) => (
                <div key={field.key} className="space-y-2">
                  <Label htmlFor={`param-${field.key}`}>{field.key}</Label>
                  {field.type === "textarea" ? (
                    <Textarea
                      id={`param-${field.key}`}
                      rows={4}
                      value={String(inspectData.params[field.key] ?? "")}
                      onChange={(event) => updateParam(field.key, event.target.value)}
                    />
                  ) : field.type === "select" ? (
                    <Select
                      id={`param-${field.key}`}
                      value={String(inspectData.params[field.key] ?? field.options?.[0] ?? "")}
                      onChange={(event) => updateParam(field.key, event.target.value)}
                    >
                      {(field.options ?? []).map((option) => (
                        <option key={option}>{option}</option>
                      ))}
                    </Select>
                  ) : field.type === "size" ? (
                    <Select
                      id={`param-${field.key}`}
                      value={String(inspectData.params[field.key] ?? "1280x720")}
                      onChange={(event) => updateParam(field.key, event.target.value)}
                    >
                      {["1280x720", "1920x1080", "720x1280", "1080x1920"].map((option) => (
                        <option key={option}>{option}</option>
                      ))}
                    </Select>
                  ) : (
                    <Input
                      id={`param-${field.key}`}
                      type="number"
                      min={field.min}
                      max={field.max}
                      value={String(inspectData.params[field.key] ?? "")}
                      onChange={(event) => updateParam(field.key, Number(event.target.value))}
                    />
                  )}
                </div>
              ))}
              {inspectData.kind === "note" ? (
                <div className="space-y-2">
                  <Label htmlFor="param-text">text</Label>
                  <Textarea
                    id="param-text"
                    rows={3}
                    value={String(inspectData.params.text ?? "")}
                    onChange={(event) => updateParam("text", event.target.value)}
                  />
                </div>
              ) : null}
              {inspectData.step?.result?.text ? (
                <div className="space-y-2">
                  <Label>result</Label>
                  <pre className="max-h-40 overflow-y-auto rounded-md border bg-muted/40 p-2 font-mono text-xs whitespace-pre-wrap">
                    {String(inspectData.step.result.text).slice(0, 2000)}
                  </pre>
                </div>
              ) : null}
            </div>
          ) : null}
          <DialogFooter>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => {
                if (!inspectId) return;
                setNodes((current) => current.filter((node) => node.id !== inspectId));
                setEdges((current) =>
                  current.filter((edge) => edge.source !== inspectId && edge.target !== inspectId),
                );
                setInspectId(null);
                scheduleSave();
              }}
            >
              <Trash2 />
              {t("canvas.deleteNode")}
            </Button>
            <Button variant="outline" onClick={() => setInspectId(null)}>
              {t("common.close")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

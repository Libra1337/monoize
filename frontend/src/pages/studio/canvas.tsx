import { useCallback, useMemo, useState } from "react";
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  addEdge,
  useEdgesState,
  useNodesState,
  type Connection,
  type Node,
  type NodeProps,
  Handle,
  Position,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useTranslation } from "react-i18next";
import useSWRSubscription from "swr/subscription";
import { toast } from "sonner";
import {
  Clapperboard,
  Film,
  Image as ImageIcon,
  Loader2,
  NotebookPen,
  Send,
  SquarePen,
  Video,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { PageWrapper } from "@/components/ui/motion";
import { cn } from "@/lib/utils";
import {
  parseGraph,
  studioApi,
  type StudioGraphNode,
  type StudioProject,
} from "@/lib/studio-api";

const statusBorder: Record<string, string> = {
  queued: "border-muted-foreground/30",
  running: "border-primary/60",
  succeeded: "border-emerald-500/60",
  failed: "border-destructive/60",
};

function nodeStatus(data: Record<string, unknown>): string {
  const status = data.status;
  return typeof status === "string" ? status : "idle";
}

function ScriptNodeCard({ data }: NodeProps) {
  const { t } = useTranslation();
  const stage = typeof data.stage === "string" ? data.stage : "idle";
  return (
    <Card className={cn("w-72 border-2", statusBorder[nodeStatus(data)] ?? "border-border")}>
      <CardContent className="space-y-2 p-3">
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-2 text-sm font-medium">
            <SquarePen className="size-4 text-primary" />
            {t("studio.node.script")}
          </span>
          <Badge variant="secondary">{t(`studio.stage.${stage}`, { defaultValue: stage })}</Badge>
        </div>
        <Handle type="source" position={Position.Right} />
        <p className="max-h-40 overflow-y-auto whitespace-pre-wrap text-xs leading-relaxed text-muted-foreground">
          {typeof data.text === "string" && data.text.trim()
            ? data.text
            : t("studio.node.scriptEmpty")}
        </p>
      </CardContent>
    </Card>
  );
}

function ShotNodeCard({ data }: NodeProps) {
  const { t } = useTranslation();
  const description = typeof data.description === "string" ? data.description : "";
  const camera = typeof data.camera === "string" ? data.camera : "";
  const duration = typeof data.duration_sec === "number" ? `${data.duration_sec}s` : "";
  const status = nodeStatus(data);
  return (
    <Card className={cn("w-64 border-2", statusBorder[status] ?? "border-border")}>
      <CardContent className="space-y-2 p-3">
        <Handle type="target" position={Position.Left} />
        <Handle type="source" position={Position.Right} />
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-2 text-sm font-medium">
            <Film className="size-4 text-primary" />
            {t("studio.node.shot")} #{typeof data.index === "number" ? data.index : "?"}
          </span>
          <Badge variant="secondary">
            {t(`studio.status.${status}`, { defaultValue: status })}
          </Badge>
        </div>
        <p className="line-clamp-3 text-xs text-muted-foreground">{description}</p>
        <div className="flex flex-wrap gap-1 text-[10px] text-muted-foreground">
          {camera && <span>{camera}</span>}
          {duration && <span>· {duration}</span>}
        </div>
      </CardContent>
    </Card>
  );
}

function MediaNodeCard({ data, kind }: NodeProps & { kind: "image" | "video" }) {
  const { t } = useTranslation();
  const status = nodeStatus(data);
  const b64 = typeof data.b64 === "string" ? data.b64 : null;
  const assetId = typeof data.asset_id === "string" ? data.asset_id : null;
  const prompt = typeof data.prompt === "string" ? data.prompt : "";
  const Icon = kind === "image" ? ImageIcon : Video;
  return (
    <Card className={cn("w-60 border-2", statusBorder[status] ?? "border-border")}>
      <CardContent className="space-y-2 p-3">
        <Handle type="target" position={Position.Left} />
        <Handle type="source" position={Position.Right} />
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-2 text-sm font-medium">
            <Icon className="size-4 text-primary" />
            {t(`studio.node.${kind}`)}
          </span>
          <Badge variant="secondary">
            {t(`studio.status.${status}`, { defaultValue: status })}
          </Badge>
        </div>
        <div className="flex aspect-video items-center justify-center overflow-hidden rounded-md bg-muted">
          {b64 ? (
            <img
              src={`data:image/png;base64,${b64}`}
              alt={prompt}
              className="size-full object-cover"
            />
          ) : assetId && status === "succeeded" && kind === "video" ? (
            <video src={studioApi.assetUrl(assetId)} controls className="size-full object-cover" />
          ) : (
            <Loader2 className="size-5 animate-spin text-muted-foreground" />
          )}
        </div>
        <p className="line-clamp-2 text-[10px] text-muted-foreground">{prompt}</p>
      </CardContent>
    </Card>
  );
}

function ImageNodeCard(props: NodeProps) {
  return <MediaNodeCard {...props} kind="image" />;
}
function VideoNodeCard(props: NodeProps) {
  return <MediaNodeCard {...props} kind="video" />;
}

function NoteNodeCard({ data }: NodeProps) {
  const { t } = useTranslation();
  return (
    <Card className="w-56 border-dashed">
      <CardContent className="space-y-1 p-3">
        <span className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
          <NotebookPen className="size-3.5" />
          {t("studio.node.note")}
        </span>
        <p className="text-xs">{typeof data.text === "string" ? data.text : ""}</p>
      </CardContent>
    </Card>
  );
}

const nodeTypes = {
  script: ScriptNodeCard,
  shot: ShotNodeCard,
  image: ImageNodeCard,
  video: VideoNodeCard,
  note: NoteNodeCard,
};

function toFlowNodes(graphNodes: StudioGraphNode[]): Node[] {
  return graphNodes.map((node) => ({
    id: node.id,
    type: node.kind,
    position: node.position,
    data: node.data,
  }));
}

type AgentMessage = { role: "user" | "agent"; text: string };

export function StudioCanvasPage({
  project,
  onProjectChanged,
}: {
  project: StudioProject;
  onProjectChanged?: () => void;
}) {
  const { t } = useTranslation();
  const initial = useMemo(() => parseGraph(project.graph_json), [project.graph_json]);
  const [nodes, , onNodesChange] = useNodesState(toFlowNodes(initial.nodes));
  const [edges, setEdges, onEdgesChange] = useEdgesState(
    initial.edges.map((edge) => ({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      animated: edge.kind === "image_to_video",
    })),
  );
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);

  useSWRSubscription<string, Error>("studio-events", (_key: string, { next }: { next: () => void }) => {
    const source = new EventSource("/api/dashboard/studio/events");
    const onGraphPatch = () => next();
    const onRunUpdate = (event: MessageEvent) => {
      try {
        const payload = JSON.parse(event.data) as { status?: string; error?: string };
        if (payload.status === "failed" && payload.error) {
          toast.error(t("studio.toast.runFailed"), { description: payload.error });
        } else if (payload.status === "succeeded") {
          toast.success(t("studio.toast.runSucceeded"));
        }
      } catch {
        /* ignore malformed frames */
      }
      next();
    };
    source.addEventListener("graph_patch", onGraphPatch);
    source.addEventListener("run_update", onRunUpdate);
    source.onerror = () => {
      /* SWR reconnect handles retry */
    };
    return () => {
      source.removeEventListener("graph_patch", onGraphPatch);
      source.removeEventListener("run_update", onRunUpdate);
      source.close();
    };
  });

  const onConnect = useCallback(
    (connection: Connection) =>
      setEdges((current) => addEdge({ ...connection, animated: false }, current)),
    [setEdges],
  );

  const handleWorkOrder = useCallback(
    async (kind: "image" | "video") => {
      const prompt = window.prompt(
        t(kind === "image" ? "studio.workOrder.imagePrompt" : "studio.workOrder.videoPrompt"),
      );
      if (!prompt?.trim()) return;
      try {
        await studioApi.workOrder({ kind, prompt: prompt.trim(), project_id: project.id });
        toast.success(t("studio.toast.queued"));
      } catch (error) {
        toast.error(String(error));
      }
    },
    [project.id, t],
  );

  const submitAgent = useCallback(
    async (text: string) => {
      setSending(true);
      setMessages((prev) => [...prev, { role: "user", text }]);
      try {
        await studioApi.submitAgent(project.id, text);
        setMessages((prev) => [...prev, { role: "agent", text: t("studio.toast.agentQueued") }]);
        onProjectChanged?.();
      } catch (error) {
        toast.error(String(error));
      } finally {
        setSending(false);
      }
    },
    [project.id, t, onProjectChanged],
  );

  return (
    <PageWrapper className="flex h-full min-h-0 flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <Clapperboard className="size-5 shrink-0 text-primary" />
          <h1 className="truncate font-display text-lg font-semibold">{project.title}</h1>
          <Badge variant="outline">v{project.version}</Badge>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={() => handleWorkOrder("image")}>
            <ImageIcon className="size-4" />
            <span className="hidden sm:inline">{t("studio.workOrder.image")}</span>
          </Button>
          <Button variant="outline" size="sm" onClick={() => handleWorkOrder("video")}>
            <Video className="size-4" />
            <span className="hidden sm:inline">{t("studio.workOrder.video")}</span>
          </Button>
        </div>
      </div>
      <div className="relative min-h-0 flex-1 overflow-hidden rounded-lg border bg-background">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          nodeTypes={nodeTypes}
          fitView
          proOptions={{ hideAttribution: true }}
        >
          <Background variant={BackgroundVariant.Dots} gap={20} className="bg-muted/40" />
          <Controls />
          <MiniMap pannable zoomable />
        </ReactFlow>
      </div>
      <Card>
        <CardContent className="flex flex-col gap-2 p-3 sm:flex-row sm:items-center">
          <div className="max-h-28 flex-1 space-y-1 overflow-y-auto">
            {messages.length === 0 && (
              <p className="text-xs text-muted-foreground">{t("studio.agent.hint")}</p>
            )}
            {messages.map((message, index) => (
              <p
                key={index}
                className={cn(
                  "text-xs",
                  message.role === "user" ? "font-medium" : "text-muted-foreground",
                )}
              >
                {message.text}
              </p>
            ))}
          </div>
          <Separator orientation="vertical" className="hidden h-8 sm:block" />
          <form
            className="flex w-full items-center gap-2 sm:max-w-md"
            onSubmit={async (event) => {
              event.preventDefault();
              const text = draft.trim();
              if (!text || sending) return;
              setDraft("");
              await submitAgent(text);
            }}
          >
            <Input
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              placeholder={t("studio.agent.placeholder")}
              className="flex-1"
            />
            <Button type="submit" size="icon" disabled={sending || !draft.trim()}>
              {sending ? <Loader2 className="size-4 animate-spin" /> : <Send className="size-4" />}
            </Button>
          </form>
        </CardContent>
      </Card>
    </PageWrapper>
  );
}

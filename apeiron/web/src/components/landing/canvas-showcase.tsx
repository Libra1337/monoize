import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import {
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  ReactFlow,
  type Edge,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { ArrowRight, Clapperboard, FileText, ImageIcon, Layers, Mic } from "lucide-react";
import { ScrollReveal, SectionKicker } from "@/components/ui/motion";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";

type MiniNode = Node<Record<string, unknown>, string>;

function MiniNodeCard({ data }: NodeProps<MiniNode>) {
  const label = String(data.label ?? "");
  const Icon = data.icon as typeof FileText | undefined;
  return (
    <Card className="w-40 p-3">
      <div className="flex items-center gap-2">
        <span className="flex size-5 items-center justify-center rounded bg-muted">
          {Icon ? <Icon className="h-3 w-3 text-muted-foreground" /> : null}
        </span>
        <span className="font-mono text-xs font-medium">{label}</span>
        {data.active ? (
          <span className="ml-auto size-1.5 animate-pulse rounded-full bg-primary" />
        ) : null}
      </div>
    </Card>
  );
}

const nodeTypes = { mini: MiniNodeCard };

/** Interactive miniature of the real workbench — same engine, same cards. */
export function CanvasShowcase() {
  const { t } = useTranslation();
  const nodes: MiniNode[] = useMemo(
    () => [
      { id: "script", type: "mini", position: { x: 0, y: 120 }, data: { label: "script", icon: FileText, active: true } },
      { id: "board", type: "mini", position: { x: 180, y: 120 }, data: { label: "storyboard", icon: Layers } },
      { id: "image", type: "mini", position: { x: 360, y: 40 }, data: { label: "image", icon: ImageIcon } },
      { id: "voice", type: "mini", position: { x: 360, y: 200 }, data: { label: "tts", icon: Mic } },
      { id: "final", type: "mini", position: { x: 540, y: 120 }, data: { label: "assemble", icon: Clapperboard, active: true } },
    ],
    [],
  );
  const edges: Edge[] = useMemo(
    () => [
      { id: "e1", source: "script", sourceHandle: "text", target: "board", targetHandle: "text" },
      { id: "e2", source: "board", sourceHandle: "shots", target: "image", targetHandle: "text" },
      { id: "e3", source: "script", sourceHandle: "text", target: "voice", targetHandle: "text" },
      { id: "e4", source: "image", sourceHandle: "image", target: "final", targetHandle: "clips" },
      { id: "e5", source: "voice", sourceHandle: "audio", target: "final", targetHandle: "audio" },
    ],
    [],
  );

  return (
    <section>
      <div className="mx-auto grid max-w-5xl items-center gap-10 px-4 py-20 sm:px-6 lg:grid-cols-[0.9fr_1.1fr]">
        <ScrollReveal className="space-y-4">
          <SectionKicker index="03" label={t("landing.canvasTitle")} />
          <h2 className="font-display text-2xl font-semibold tracking-tight text-balance sm:text-3xl">
            {t("landing.canvasTitle")}
          </h2>
          <p className="leading-relaxed text-muted-foreground text-pretty">
            {t("landing.canvasBody")}
          </p>
          <div>
            <Link to="/projects">
              <Button variant="outline">
                {t("landing.canvasCta")}
                <ArrowRight />
              </Button>
            </Link>
          </div>
        </ScrollReveal>
        <ScrollReveal delay={0.1}>
          <div className="h-80 overflow-hidden rounded-lg border bg-background">
            <ReactFlow
              nodes={nodes}
              edges={edges}
              nodeTypes={nodeTypes}
              fitView
              fitViewOptions={{ padding: 0.25 }}
              minZoom={0.5}
              maxZoom={1.5}
              proOptions={{ hideAttribution: true }}
              nodesConnectable={false}
              edgesFocusable={false}
            >
              <Background variant={BackgroundVariant.Dots} gap={20} className="bg-muted/40" />
              <Controls showInteractive={false} position="bottom-left" />
              <MiniMap pannable position="bottom-right" />
            </ReactFlow>
          </div>
        </ScrollReveal>
      </div>
    </section>
  );
}

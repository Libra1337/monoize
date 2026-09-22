import type { Graph } from "@/lib/api";
import { NODE_META } from "@/lib/nodes";

/**
 * Renders a template's real graph as a miniature dot-and-line diagram —
 * honest data-driven preview, no fake media.
 */
export function GraphPreview({ graph }: { graph: Graph }) {
  const nodes = graph.nodes.filter((node) => node.kind !== "note");
  if (nodes.length === 0) return null;
  const xs = nodes.map((node) => node.x);
  const ys = nodes.map((node) => node.y);
  const minX = Math.min(...xs) - 60;
  const maxX = Math.max(...xs) + 60;
  const minY = Math.min(...ys) - 40;
  const maxY = Math.max(...ys) + 40;
  const width = Math.max(maxX - minX, 1);
  const height = Math.max(maxY - minY, 1);
  const project = (node: { x: number; y: number }) => ({
    cx: node.x - minX,
    cy: node.y - minY,
  });
  return (
    <svg
      viewBox={`0 0 ${width} ${height}`}
      className="h-24 w-full"
      role="img"
      aria-label="workflow graph preview"
    >
      {graph.edges.map((edge) => {
        const source = graph.nodes.find((node) => node.id === edge.source);
        const target = graph.nodes.find((node) => node.id === edge.target);
        if (!source || !target) return null;
        const a = project(source);
        const b = project(target);
        return (
          <line
            key={edge.id}
            x1={a.cx}
            y1={a.cy}
            x2={b.cx}
            y2={b.cy}
            stroke="hsl(var(--muted-foreground) / 0.35)"
            strokeWidth="1.5"
          />
        );
      })}
      {nodes.map((node) => {
        const point = project(node);
        const highlighted = NODE_META[node.kind]?.outputPort != null;
        return (
          <circle
            key={node.id}
            cx={point.cx}
            cy={point.cy}
            r={highlighted ? 7 : 5}
            fill={node.kind === "assemble" ? "hsl(var(--primary))" : "hsl(var(--card))"}
            stroke={
              node.kind === "assemble"
                ? "hsl(var(--primary))"
                : "hsl(var(--muted-foreground) / 0.6)"
            }
            strokeWidth="1.5"
          />
        );
      })}
    </svg>
  );
}

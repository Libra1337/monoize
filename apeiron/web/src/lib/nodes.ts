import type { LucideIcon } from "lucide-react";
import {
  AlignLeft,
  CircleDot,
  Clapperboard,
  FileText,
  Film,
  ImageIcon,
  Layers,
  Mic,
  StickyNote,
} from "lucide-react";

export interface NodeMeta {
  kind: string;
  icon: LucideIcon;
  outputPort: string | null;
  inputPorts: string[];
  /** Params rendered in the inspector, in order. */
  fields: NodeField[];
}

export interface NodeField {
  key: string;
  type: "text" | "textarea" | "number" | "select" | "size";
  options?: string[];
  min?: number;
  max?: number;
}

export const NODE_META: Record<string, NodeMeta> = {
  note: {
    kind: "note",
    icon: StickyNote,
    outputPort: null,
    inputPorts: [],
    fields: [{ key: "text", type: "textarea" }],
  },
  script: {
    kind: "script",
    icon: FileText,
    outputPort: "text",
    inputPorts: [],
    fields: [
      { key: "prompt", type: "textarea" },
      { key: "system", type: "textarea" },
    ],
  },
  storyboard: {
    kind: "storyboard",
    icon: Layers,
    outputPort: "shots",
    inputPorts: ["text"],
    fields: [{ key: "count", type: "number", min: 1, max: 24 }],
  },
  image: {
    kind: "image",
    icon: ImageIcon,
    outputPort: "image",
    inputPorts: ["text"],
    fields: [
      { key: "prompt", type: "textarea" },
      { key: "size", type: "size" },
      { key: "style", type: "text" },
    ],
  },
  video: {
    kind: "video",
    icon: Film,
    outputPort: "video",
    inputPorts: ["text", "image"],
    fields: [
      { key: "prompt", type: "textarea" },
      { key: "seconds", type: "select", options: ["4", "5", "8", "10"] },
      { key: "size", type: "size" },
    ],
  },
  tts: {
    kind: "tts",
    icon: Mic,
    outputPort: "audio",
    inputPorts: ["text"],
    fields: [
      {
        key: "voice",
        type: "select",
        options: ["alloy", "ash", "ballad", "coral", "echo", "sage", "shimmer", "verse"],
      },
      { key: "speed", type: "number", min: 0.5, max: 2 },
    ],
  },
  subtitle: {
    kind: "subtitle",
    icon: AlignLeft,
    outputPort: "subtitle",
    inputPorts: ["text", "audio"],
    fields: [],
  },
  material: {
    kind: "material",
    icon: CircleDot,
    outputPort: "video",
    inputPorts: ["text"],
    fields: [
      { key: "query", type: "text" },
      { key: "url", type: "text" },
    ],
  },
  assemble: {
    kind: "assemble",
    icon: Clapperboard,
    outputPort: "video",
    inputPorts: ["clips", "audio", "subtitle"],
    fields: [
      { key: "resolution", type: "select", options: ["1280x720", "1920x1080", "720x1280", "1080x1920"] },
      { key: "fps", type: "number", min: 12, max: 60 },
    ],
  },
};

export const NODE_KINDS = Object.keys(NODE_META);

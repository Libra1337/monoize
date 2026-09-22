export class ApiError extends Error {
  code: string;
  status: number;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api${path}`, {
    credentials: "include",
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {}),
    },
    ...init,
  });
  if (response.status === 401) {
    window.dispatchEvent(new CustomEvent("apeiron:unauthorized"));
  }
  const payload = await response.json().catch(() => null);
  const code = payload?.error?.code ?? "unknown";
  const message = payload?.error?.message ?? response.statusText;
  if (!response.ok) {
    throw new ApiError(response.status, code, message);
  }
  return payload as T;
}

export const api = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: "POST", body: body === undefined ? undefined : JSON.stringify(body) }),
  put: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: "PUT", body: JSON.stringify(body) }),
  delete: <T>(path: string) => request<T>(path, { method: "DELETE" }),
};

export interface Me {
  user: {
    id: string;
    username: string;
    display_name: string;
    role: string;
  };
  balance_nano_usd: string;
  balance_unlimited: boolean;
  platform_url: string;
}

export interface ProjectSummary {
  id: string;
  title: string;
  version: number;
  template_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface GraphNode {
  id: string;
  kind: string;
  x: number;
  y: number;
  params: Record<string, unknown>;
}

export interface GraphEdge {
  id: string;
  source: string;
  sourcePort: string;
  target: string;
  targetPort: string;
}

export interface Graph {
  version?: number;
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface Project extends ProjectSummary {
  graph: Graph;
}

export interface Step {
  id: string;
  run_id: string;
  node_id: string;
  kind: string;
  status: string;
  payload: Record<string, unknown>;
  result: Record<string, unknown> | null;
  error: string | null;
  attempts: number;
  charge_nano_usd: string | null;
  refund_nano_usd: string | null;
  updated_at: string;
}

export interface Run {
  id: string;
  user_id?: string;
  project_id: string | null;
  kind: string;
  status: string;
  error: string | null;
  params: Record<string, unknown>;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
  steps?: Step[];
  output?: { asset_id: string; mime_type: string; bytes: number } | null;
  spend_nano_usd?: string | null;
}

export interface Asset {
  id: string;
  run_id: string | null;
  step_id: string | null;
  kind: string;
  mime_type: string;
  bytes: number;
  meta: Record<string, unknown>;
  created_at: string;
}

export interface Template {
  id: string;
  source: string;
  name: string;
  description: string;
  graph: Graph;
  params: Record<string, unknown>;
}

export interface Provider {
  id: string;
  kind: string;
  upstream_kind: string;
  name: string;
  base_url: string;
  model: string | null;
  params: Record<string, unknown>;
  enabled: boolean;
  weight: number;
  updated_at: string;
  api_key_set: boolean;
}

export function assetContentUrl(id: string): string {
  return `/api/assets/${id}/content`;
}

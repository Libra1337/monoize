// Studio canvas creation workbench client (studio-workflow.spec.md ST-U1/U2).

const API_BASE = "/api/dashboard/studio";
const ADMIN_BASE = "/api/dashboard/admin/studio";

export type StudioGraph = {
  nodes: StudioGraphNode[];
  edges: StudioGraphEdge[];
};

export type StudioGraphNode = {
  id: string;
  kind: "script" | "shot" | "image" | "video" | "note";
  position: { x: number; y: number };
  data: Record<string, unknown>;
};

export type StudioGraphEdge = {
  id: string;
  source: string;
  target: string;
  kind: "contains" | "consistency" | "image_to_video" | "probe";
};

export interface StudioProject {
  id: string;
  user_id: string;
  title: string;
  graph_json: string;
  version: number;
  template_id?: string | null;
  created_at: string;
  updated_at: string;
}

export interface StudioRun {
  id: string;
  user_id: string;
  project_id?: string | null;
  kind: string;
  status: string;
  error?: string | null;
  created_at: string;
  updated_at: string;
  finished_at?: string | null;
}

export interface StudioStep {
  id: string;
  run_id: string;
  kind: string;
  status: string;
  node_id?: string | null;
  error?: string | null;
  charge_nano_usd: number;
  refund_nano_usd: number;
  created_at: string;
}

export interface StudioAsset {
  id: string;
  user_id: string;
  run_id: string;
  step_id: string;
  kind: string;
  upstream_kind: string;
  mime_type: string;
  status: string;
  created_at: string;
}

export interface StudioTemplate {
  id: string;
  source: string;
  name: string;
  description: string;
  graph_json: string;
  params_json: string;
  price_nano_usd_map: string;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface StudioSettingsDto {
  agent_model: string;
  default_video_upstream: string;
  image_model: string;
}

async function jsonFetch<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, {
    ...init,
    headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
  });
  if (!response.ok) {
    let message = `${response.status}`;
    try {
      const body = await response.json();
      message = body?.error ?? body?.message ?? message;
    } catch {
      /* keep status text */
    }
    throw new Error(message);
  }
  return response.json() as Promise<T>;
}

export const studioApi = {
  listProjects: () => jsonFetch<{ projects: StudioProject[] }>(`${API_BASE}/projects`),
  createProject: (body: { title: string; template_id?: string }) =>
    jsonFetch<{ project: StudioProject }>(`${API_BASE}/projects`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  getProject: (id: string) => jsonFetch<{ project: StudioProject }>(`${API_BASE}/projects/${id}`),
  saveGraph: (id: string, version: number, graph: StudioGraph) =>
    jsonFetch<{ saved: boolean; project: StudioProject }>(`${API_BASE}/projects/${id}/graph`, {
      method: "PUT",
      body: JSON.stringify({ version, graph }),
    }),
  deleteProject: (id: string) =>
    jsonFetch<{ deleted: boolean }>(`${API_BASE}/projects/${id}`, { method: "DELETE" }),
  submitAgent: (id: string, message: string) =>
    jsonFetch<{ run_id: string }>(`${API_BASE}/projects/${id}/agent`, {
      method: "POST",
      body: JSON.stringify({ message }),
    }),
  listRuns: (limit = 50) => jsonFetch<{ runs: StudioRun[] }>(`${API_BASE}/runs?limit=${limit}`),
  getRun: (id: string) =>
    jsonFetch<{ run: StudioRun; steps: StudioStep[] }>(`${API_BASE}/runs/${id}`),
  cancelRun: (id: string) =>
    jsonFetch<{ canceled: boolean }>(`${API_BASE}/runs/${id}/cancel`, { method: "POST" }),
  workOrder: (body: {
    kind: "image" | "video";
    prompt: string;
    project_id?: string;
    seconds?: string;
    size?: string;
  }) =>
    jsonFetch<{ run_id: string }>(`${API_BASE}/work-order`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  listAssets: (kind?: string) =>
    jsonFetch<{ assets: StudioAsset[] }>(
      `${API_BASE}/assets${kind ? `?kind=${encodeURIComponent(kind)}` : ""}`,
    ),
  assetUrl: (id: string) => `${API_BASE}/assets/${id}/content`,
  upload: async (file: File) => {
    const form = new FormData();
    form.append("file", file);
    const response = await fetch(`${API_BASE}/uploads`, { method: "POST", body: form });
    if (!response.ok) throw new Error(String(response.status));
    return response.json() as Promise<{ data_url: string; bytes: number }>;
  },
  listTemplates: () => jsonFetch<{ templates: StudioTemplate[] }>(`${API_BASE}/templates`),
  adminListTemplates: () => jsonFetch<{ templates: StudioTemplate[] }>(`${ADMIN_BASE}/templates`),
  adminCreateTemplate: (body: Record<string, unknown>) =>
    jsonFetch<{ id: string }>(`${ADMIN_BASE}/templates`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  adminUpdateTemplate: (id: string, body: Record<string, unknown>) =>
    jsonFetch<{ updated: boolean }>(`${ADMIN_BASE}/templates/${id}`, {
      method: "PUT",
      body: JSON.stringify(body),
    }),
  adminDeleteTemplate: (id: string) =>
    jsonFetch<{ deleted: boolean }>(`${ADMIN_BASE}/templates/${id}`, { method: "DELETE" }),
  adminListRuns: (limit = 100) => jsonFetch<{ runs: StudioRun[] }>(`${ADMIN_BASE}/runs?limit=${limit}`),
  adminCancelRun: (id: string) =>
    jsonFetch<{ canceled: boolean }>(`${ADMIN_BASE}/runs/${id}/cancel`, { method: "POST" }),
  adminGetSettings: () => jsonFetch<{ settings: StudioSettingsDto }>(`${ADMIN_BASE}/settings`),
  adminUpdateSettings: (body: Partial<StudioSettingsDto>) =>
    jsonFetch<{ settings: StudioSettingsDto }>(`${ADMIN_BASE}/settings`, {
      method: "PUT",
      body: JSON.stringify(body),
    }),
};

export function parseGraph(raw: string): StudioGraph {
  try {
    const parsed = JSON.parse(raw) as StudioGraph;
    if (Array.isArray(parsed.nodes) && Array.isArray(parsed.edges)) return parsed;
  } catch {
    /* fall through */
  }
  return { nodes: [], edges: [] };
}

import { useEffect, useRef } from "react";
import useSWR, { mutate as globalMutate } from "swr";

export interface StreamEvent {
  kind: string;
  payload: Record<string, unknown>;
}

/**
 * Subscribes to the per-user SSE stream and revalidates the relevant SWR
 * keys per event kind (run_update / step_update / job_update / balance_update).
 */
export function useEventStream(enabled: boolean) {
  const sourceRef = useRef<EventSource | null>(null);

  useEffect(() => {
    if (!enabled) return;
    const source = new EventSource("/api/events", { withCredentials: true });
    sourceRef.current = source;
    source.onmessage = (message) => {
      try {
        const event = JSON.parse(message.data) as StreamEvent;
        if (event.kind === "run_update") {
          const runId = (event.payload as { id?: string }).id;
          if (runId) void globalMutate(["run", runId]);
          void globalMutate("runs");
        } else if (event.kind === "step_update" || event.kind === "job_update") {
          const runId = (event.payload as { run_id?: string }).run_id;
          if (runId) void globalMutate(["run", runId]);
        } else if (event.kind === "balance_update") {
          void globalMutate("me");
        } else if (event.kind === "graph_patch") {
          void globalMutate([
            "project",
            (event.payload as { project_id?: string }).project_id,
          ]);
        }
      } catch {
        // ignore malformed frames
      }
    };
    return () => {
      source.close();
      sourceRef.current = null;
    };
  }, [enabled]);
}

export function useRun(runId: string | null) {
  return useSWR(
    runId ? (["run", runId] as const) : null,
    ([, id]) =>
      fetch(`/api/runs/${id}`, { credentials: "include" }).then(async (response) => {
        const payload = await response.json();
        if (!response.ok) throw new Error(payload?.error?.message ?? "run fetch failed");
        return payload;
      }),
    { refreshInterval: 4000 },
  );
}

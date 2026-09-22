import useSWR from "swr";
import { useAuth } from "@/auth";

interface Healthz {
  ok: boolean;
  platform_url: string;
}

/**
 * Signed-in visitors create inside Apeiron; anonymous visitors are sent
 * through the platform handoff entry (`/studio-entry`), which bounces to the
 * platform login and back with a token.
 */
export function useCreateTarget(): string {
  const { me } = useAuth();
  const { data } = useSWR<Healthz>("healthz", () =>
    fetch("/api/healthz").then((response) => response.json()),
  );
  if (me === null && data?.platform_url) {
    return `${data.platform_url}/studio-entry`;
  }
  return "/create";
}

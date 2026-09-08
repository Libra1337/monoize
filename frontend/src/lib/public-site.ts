export type PublicApiBaseUrlResolution =
  | { baseUrl: string; error: null }
  | { baseUrl: null; error: "public_api_base_url_required" };

export function resolvePublicApiBaseUrl(
  configuredBaseUrl: string,
  browserOrigin: string,
): PublicApiBaseUrlResolution {
  const configured = configuredBaseUrl.trim();
  if (configured) {
    return { baseUrl: configured.replace(/\/+$/, ""), error: null };
  }

  const origin = new URL(browserOrigin);
  if (origin.protocol !== "https:") {
    return { baseUrl: null, error: "public_api_base_url_required" };
  }
  return { baseUrl: `${origin.origin}/v1`, error: null };
}

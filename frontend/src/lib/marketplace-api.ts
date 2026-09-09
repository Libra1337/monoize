export interface MarketplaceRateRange {
  min: string;
  max: string;
  unit: string;
}

export interface MarketplaceItem {
  public_group_name: string;
  model: string;
  capabilities: string[];
  input_rate_range: MarketplaceRateRange | null;
  output_rate_range: MarketplaceRateRange | null;
  offer_count: number;
}

export interface MarketplaceResponse {
  generated_at: string;
  revision: string;
  next_cursor: string | null;
  items: MarketplaceItem[];
}

export interface MarketplaceOfferRate {
  usage_class: string;
  unit: string;
  /** Nano-CNY per unit, already normalized by the server (MM-P2a). */
  display_rate_nano: string;
  context_tier: string | null;
  service_tier: string | null;
  modality: string | null;
  cache_ttl: string | null;
}

export interface MarketplaceOffer {
  public_provider_name: string;
  public_channel_name: string;
  api_type: string;
  rates: MarketplaceOfferRate[];
}

export interface MarketplaceOffersResponse {
  generated_at: string;
  revision: string;
  public_group_name: string;
  model: string;
  next_cursor: string | null;
  offers: MarketplaceOffer[];
}

export class MarketplaceApiError extends Error {
  readonly status: number;
  readonly code: string | null;

  constructor(status: number, code: string | null, message: string) {
    super(message);
    this.name = "MarketplaceApiError";
    this.status = status;
    this.code = code;
  }
}

export async function marketplaceRequest<T>(url: string): Promise<T> {
  // The session cookie is sent so a signed-in user gets the catalogue for their own account
  // class. Anonymous visitors carry no usable session, and `optional_current_user` treats a
  // missing or stale cookie as `None`, so the public page still resolves the standard class.
  const response = await fetch(url, { credentials: "include" });
  const data = await response.json();
  if (!response.ok) {
    throw new MarketplaceApiError(
      response.status,
      data?.error?.code ?? null,
      data?.error?.message ?? "Marketplace request failed",
    );
  }
  return data as T;
}

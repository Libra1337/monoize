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
  display_rate_nano_usd: string;
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
  const response = await fetch(url, { credentials: "omit" });
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

export function resolveMarketplacePages<T>(pages: T[], data?: T): T[] {
  return data ? [data, ...pages.slice(1)] : pages;
}

export interface MarketplacePageState<T> {
  key: string | null;
  pages: T[];
}

export function replaceMarketplaceFirstPage<T>(
  state: MarketplacePageState<T>,
  key: string,
  page: T,
): MarketplacePageState<T> {
  if (state.key !== key) return { key, pages: [page] };
  return { key, pages: [page, ...state.pages.slice(1)] };
}

export function appendMarketplacePage<T>(
  state: MarketplacePageState<T>,
  key: string,
  page: T,
  maximumPages = Number.POSITIVE_INFINITY,
): MarketplacePageState<T> {
  if (state.key !== key) return state;
  return { key, pages: [...state.pages, page].slice(-maximumPages) };
}

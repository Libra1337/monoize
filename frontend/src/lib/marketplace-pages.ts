export function resolveMarketplacePages<T>(pages: T[], data?: T): T[] {
  return data ? [data, ...pages.slice(1)] : pages;
}

export interface MarketplacePageState<T> {
  key: string | null;
  pages: T[];
}

interface RevisionedPage {
  revision: string;
}

export function replaceMarketplaceFirstPage<T extends RevisionedPage>(
  state: MarketplacePageState<T>,
  key: string,
  page: T,
): MarketplacePageState<T> {
  if (state.key !== key || state.pages[0]?.revision !== page.revision) {
    return { key, pages: [page] };
  }
  return { key, pages: [page, ...state.pages.slice(1)] };
}

export function appendMarketplacePage<T extends RevisionedPage>(
  state: MarketplacePageState<T>,
  key: string,
  page: T,
  maximumPages = Number.POSITIVE_INFINITY,
): MarketplacePageState<T> {
  if (state.key !== key || state.pages[0]?.revision !== page.revision) return state;
  return { key, pages: [...state.pages, page].slice(-maximumPages) };
}

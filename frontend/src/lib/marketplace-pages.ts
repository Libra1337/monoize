export function resolveMarketplacePages<T>(pages: T[], data?: T): T[] {
  return data ? [data, ...pages.slice(1)] : pages;
}

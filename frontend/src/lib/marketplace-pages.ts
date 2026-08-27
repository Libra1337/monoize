export function resolveMarketplacePages<T>(pages: T[], data?: T): T[] {
  return pages.length > 0 ? pages : data ? [data] : [];
}

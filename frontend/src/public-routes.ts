export const PUBLIC_PATHS = {
  home: "/",
  login: "/login",
  apiDocs: "/apidocs",
  status: "/status",
  marketplace: "/marketplace",
  usageRanking: "/usage-ranking",
} as const;

export const PUBLIC_ROUTES = Object.values(PUBLIC_PATHS);

export function isProtectedConsolePath(pathname: string): boolean {
  if (pathname === "/settings" || pathname.startsWith("/settings/")) {
    return true;
  }
  return pathname === "/dashboard" || pathname.startsWith("/dashboard/");
}

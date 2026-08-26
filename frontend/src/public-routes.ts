export const PUBLIC_PATHS = {
  home: "/",
  login: "/login",
  apiDocs: "/apidocs",
  status: "/status",
  marketplace: "/dashboard/marketplace",
} as const;

export const PUBLIC_ROUTES = Object.values(PUBLIC_PATHS);

export function isProtectedConsolePath(pathname: string): boolean {
  if (pathname === "/settings" || pathname.startsWith("/settings/")) {
    return true;
  }
  return (
    (pathname === "/dashboard" || pathname.startsWith("/dashboard/")) &&
    pathname !== PUBLIC_PATHS.marketplace
  );
}

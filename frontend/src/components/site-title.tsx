import { useEffect } from "react";
import { usePublicSiteSettings } from "@/lib/swr";

// PS-L6: the browser tab title MUST follow the runtime site_name on every
// route. The static HTML <title> is only the pre-hydration fallback.
export function SiteTitle() {
  const { data } = usePublicSiteSettings();

  useEffect(() => {
    if (data?.site_name) {
      document.title = data.site_name;
    }
  }, [data?.site_name]);

  return null;
}

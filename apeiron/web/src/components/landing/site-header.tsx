import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ArrowRight } from "lucide-react";
import { ApeironLogo } from "@/components/logo";
import { Button } from "@/components/ui/button";
import { useAuth } from "@/auth";
import { useCreateTarget } from "@/lib/platform-entry";

export function SiteHeader() {
  const { t } = useTranslation();
  const { me } = useAuth();
  const createTarget = useCreateTarget();
  return (
    <header className="sticky top-0 z-40 border-b border-border/60 bg-background/80 backdrop-blur-xl">
      <div className="mx-auto flex h-14 max-w-5xl items-center justify-between px-4 sm:px-6">
        <Link to="/" className="flex items-center gap-2.5">
          <span className="flex size-8 items-center justify-center rounded-lg border bg-card">
            <ApeironLogo className="h-4 w-4" />
          </span>
          <span className="font-display text-sm font-semibold tracking-tight">
            {t("brand.name")}
          </span>
        </Link>
        <div className="flex items-center gap-2">
          {me?.platform_url ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => window.open(me.platform_url, "_blank", "noreferrer")}
            >
              {t("nav.backToPlatform")}
            </Button>
          ) : null}
          {createTarget === "/create" ? (
            <Link to="/create">
              <Button variant="primary" size="sm">
                {t("landing.promptCta")}
                <ArrowRight />
              </Button>
            </Link>
          ) : (
            <Button variant="primary" size="sm" onClick={() => (window.location.href = createTarget)}>
              {t("landing.promptCta")}
              <ArrowRight />
            </Button>
          )}
        </div>
      </div>
    </header>
  );
}

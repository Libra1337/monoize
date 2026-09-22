import { useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { motion } from "framer-motion";
import { ArrowRight, Loader2 } from "lucide-react";
import { api, ApiError } from "@/lib/api";
import { useAuth } from "@/auth";
import { ApeironLogo } from "@/components/logo";
import { Button } from "@/components/ui/button";

export function HandoffPage() {
  const { t } = useTranslation();
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const { refresh } = useAuth();
  const token = params.get("token");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!token) {
      setError("missing token");
      return;
    }
    let cancelled = false;
    api
      .post("/auth/exchange", { token })
      .then(() => {
        if (cancelled) return;
        refresh();
        navigate("/", { replace: true });
      })
      .catch((cause: unknown) => {
        if (cancelled) return;
        setError(cause instanceof ApiError ? cause.message : String(cause));
      });
    return () => {
      cancelled = true;
    };
  }, [token, navigate, refresh]);

  return (
    <div className="flex h-dvh flex-col items-center justify-center gap-6 bg-background px-6">
      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5 }}
        className="flex flex-col items-center gap-4 text-center"
      >
        <span className="flex size-12 items-center justify-center rounded-lg border bg-card">
          <ApeironLogo className="h-6 w-6" />
        </span>
        <h1 className="font-display text-2xl font-semibold tracking-tight">
          {error ? t("handoff.failed") : t("handoff.exchanging")}
        </h1>
        {error ? (
          <>
            <p className="max-w-md text-sm text-muted-foreground">
              {t("handoff.failedDescription")}
            </p>
            <p className="max-w-md font-mono text-xs text-muted-foreground/70">{error}</p>
            <div className="flex gap-2">
              <Button variant="outline" onClick={() => window.history.back()}>
                {t("handoff.return")}
              </Button>
              <Button variant="primary" onClick={() => navigate("/", { replace: true })}>
                {t("handoff.goHome")}
                <ArrowRight />
              </Button>
            </div>
          </>
        ) : (
          <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
        )}
      </motion.div>
    </div>
  );
}

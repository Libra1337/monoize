import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Check, Copy, KeyRound, Radio, Terminal } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper, motion, transitions } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/skeleton";
import {
  API_FAMILIES,
  API_SAMPLE_LANGUAGES,
  apiFamilyDefinition,
  generateApiSample,
  type ApiFamily,
  type ApiSampleLanguage,
} from "@/lib/api-samples";
import { resolvePublicApiBaseUrl } from "@/lib/public-site";
import { usePublicSiteSettings } from "@/lib/swr";
import { cn } from "@/lib/utils";

export function DashboardApiDocsPage() {
  const { t } = useTranslation();
  const { data: site, isLoading } = usePublicSiteSettings();
  const [family, setFamily] = useState<ApiFamily>("responses");
  const [language, setLanguage] = useState<ApiSampleLanguage>("curl");
  const [copied, setCopied] = useState(false);
  const configuredBaseUrl = site?.api_base_url.trim() ?? "";
  const resolution = configuredBaseUrl
    ? resolvePublicApiBaseUrl(configuredBaseUrl, window.location.origin)
    : { baseUrl: null, error: "public_api_base_url_required" as const };
  const definition = apiFamilyDefinition(family);
  const sample = useMemo(
    () => resolution.baseUrl
      ? generateApiSample(language, family, resolution.baseUrl)
      : "",
    [family, language, resolution.baseUrl],
  );

  const copySample = async () => {
    if (!sample) return;
    await navigator.clipboard.writeText(sample);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1800);
  };

  return (
    <PageWrapper className="flex min-w-0 flex-col gap-6">
      <motion.div initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }} transition={transitions.normal}>
        <PageHeader title={t("apiDocsConsole.title")} description={t("apiDocsConsole.description")} />
      </motion.div>

      <motion.nav
        aria-label={t("publicSite.docs.apiFamilies")}
        className="flex gap-1 overflow-x-auto rounded-lg border bg-card p-1"
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.04, ...transitions.normal }}
      >
        {API_FAMILIES.map((item) => (
          <button
            key={item}
            type="button"
            aria-current={family === item ? "page" : undefined}
            onClick={() => setFamily(item)}
            className={cn(
              "min-h-11 shrink-0 cursor-pointer rounded-md px-3 text-sm font-medium transition-colors duration-200 focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring",
              family === item ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-muted hover:text-foreground",
            )}
          >
            {t(`apiDocsConsole.families.${item}`)}
          </button>
        ))}
      </motion.nav>

      <motion.div
        className="min-w-0"
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.08, ...transitions.normal }}
      >
        <div className="min-w-0 overflow-hidden rounded-xl border bg-card">
          <header className="flex flex-col gap-3 border-b p-4 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <Badge>{definition.method}</Badge>
                <code className="break-all text-sm font-semibold">{definition.path}</code>
              </div>
              <p className="mt-2 text-sm text-muted-foreground">{t(`publicSite.docs.familyDescriptions.${family}`)}</p>
            </div>
            <Badge variant="outline" className="w-fit">
              {definition.supportsStreaming ? t("apiDocsConsole.streamingSupported") : t("apiDocsConsole.streamingUnsupported")}
            </Badge>
          </header>

          <section className="border-b p-4" aria-labelledby="console-base-url">
            <div className="flex items-center gap-2">
              <Terminal className="size-4 text-primary" />
              <h2 id="console-base-url" className="text-sm font-semibold">{t("apiDocsConsole.baseUrl")}</h2>
            </div>
            {isLoading ? <Skeleton className="mt-3 h-11 w-full" /> : resolution.error ? (
              <div className="mt-3 flex gap-2 rounded-lg border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive">
                <AlertTriangle className="mt-0.5 size-4 shrink-0" />
                <span>{t("apiDocsConsole.baseUrlMissing")}</span>
              </div>
            ) : (
              <code className="mt-3 block overflow-x-auto rounded-lg bg-muted px-3 py-3 text-sm">{resolution.baseUrl}</code>
            )}
          </section>

          <section aria-labelledby="console-request-sample">
            <div className="flex flex-wrap items-center gap-1 border-b px-3 py-2">
              <h2 id="console-request-sample" className="sr-only">{t("apiDocsConsole.requestSample")}</h2>
              {API_SAMPLE_LANGUAGES.map((item) => (
                <button
                  key={item}
                  type="button"
                  aria-pressed={language === item}
                  onClick={() => setLanguage(item)}
                  className={cn(
                    "min-h-11 cursor-pointer rounded-md px-3 text-sm font-medium transition-colors duration-200 focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring",
                    language === item ? "bg-muted text-foreground" : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {t(`publicSite.docs.languages.${item}`)}
                </button>
              ))}
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="ml-auto min-h-11"
                disabled={!sample}
                onClick={copySample}
              >
                {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
                {copied ? t("apiDocsConsole.copied") : t("apiDocsConsole.copy")}
              </Button>
            </div>
            <div aria-live="polite" className="sr-only">{copied ? t("apiDocsConsole.copied") : ""}</div>
            {isLoading ? (
              <div className="p-4"><Skeleton className="h-72 w-full" /></div>
            ) : (
              <pre className="max-h-[34rem] overflow-auto bg-foreground p-5 text-sm leading-7 text-background"><code>{sample || t("apiDocsConsole.baseUrlMissing")}</code></pre>
            )}
          </section>
          <div className="grid border-t md:grid-cols-2">
          <section className="border-b p-4 md:border-r">
            <div className="flex items-center gap-2"><KeyRound className="size-4 text-primary" /><h2 className="text-sm font-semibold">{t("apiDocsConsole.authentication")}</h2></div>
            <code className="mt-3 block overflow-x-auto rounded-lg bg-muted p-3 text-xs">Authorization: Bearer $LYNSHEN_API_KEY</code>
          </section>
          <section className="border-b p-4">
            <div className="flex items-center gap-2"><Radio className="size-4 text-primary" /><h2 className="text-sm font-semibold">{t("apiDocsConsole.streaming")}</h2></div>
            <p className="mt-3 text-sm leading-6 text-muted-foreground">
              {definition.supportsStreaming ? t("apiDocsConsole.streamingDescription") : t("apiDocsConsole.streamingUnsupportedDescription")}
            </p>
          </section>
          <section className="p-4 md:border-r">
            <h2 className="text-sm font-semibold">{t("apiDocsConsole.successShape")}</h2>
            <pre className="mt-3 overflow-x-auto rounded-lg bg-muted p-3 text-xs leading-5"><code>{definition.successShape}</code></pre>
          </section>
          <section className="border-t p-4 md:border-t-0">
            <h2 className="text-sm font-semibold">{t("apiDocsConsole.commonErrors")}</h2>
            <pre className="mt-3 overflow-x-auto rounded-lg bg-muted p-3 text-xs leading-5"><code>{definition.commonErrorShape}</code></pre>
            <p className="mt-3 text-xs leading-5 text-muted-foreground">401 unauthorized · 403 forbidden · 429 rate_limited</p>
          </section>
          </div>
        </div>
      </motion.div>
    </PageWrapper>
  );
}

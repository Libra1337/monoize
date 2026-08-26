import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Check, Copy, KeyRound, Radio, Terminal } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { usePublicSiteSettings } from "@/lib/swr";
import { resolvePublicApiBaseUrl } from "@/lib/public-site";
import { cn } from "@/lib/utils";

type SampleLanguage = "curl" | "python" | "javascript" | "go";
type ApiFamily = "responses" | "chat" | "messages" | "gemini" | "images";

const languages: SampleLanguage[] = ["curl", "python", "javascript", "go"];
const families: ApiFamily[] = ["responses", "chat", "messages", "gemini", "images"];

function endpointFor(family: ApiFamily): string {
  return family === "responses" || family === "gemini" ? "responses" : family === "chat" ? "chat/completions" : family === "messages" ? "messages" : "images/generations";
}

function bodyFor(family: ApiFamily): string {
  if (family === "responses") return '{"model":"gpt-5","input":"Explain vector databases."}';
  if (family === "chat") return '{"model":"gpt-5","messages":[{"role":"user","content":"Hello"}],"stream":true}';
  if (family === "messages") return '{"model":"claude-sonnet-4","max_tokens":1024,"messages":[{"role":"user","content":"Hello"}]}';
  if (family === "gemini") return '{"model":"gemini-2.5-pro","input":"Hello"}';
  return '{"model":"gpt-image-1","prompt":"A blue paper console"}';
}

function sampleFor(language: SampleLanguage, family: ApiFamily, baseUrl: string): string {
  const url = `${baseUrl}/${endpointFor(family)}`;
  const body = bodyFor(family);
  if (language === "curl") return [
    `curl "${url}" \\`,
    `  -H "Authorization: Bearer $LYNSHEN_API_KEY" \\`,
    `  -H "Content-Type: application/json" \\`,
    `  -d '${body}'`,
  ].join("\n");
  if (language === "python") return `import os\nimport requests\n\nresponse = requests.post(\n    "${url}",\n    headers={"Authorization": f"Bearer {os.environ['LYNSHEN_API_KEY']}"},\n    json=${body.replace(/true/g, "True")},\n)\nresponse.raise_for_status()\nprint(response.json())`;
  if (language === "javascript") return `const response = await fetch("${url}", {\n  method: "POST",\n  headers: {\n    "Authorization": \`Bearer \${process.env.LYNSHEN_API_KEY}\`,\n    "Content-Type": "application/json",\n  },\n  body: JSON.stringify(${body}),\n});\nif (!response.ok) throw new Error(await response.text());\nconsole.log(await response.json());`;
  return `package main\n\nimport (\n  "bytes"\n  "net/http"\n  "os"\n)\n\nfunc main() {\n  body := []byte(\`${body}\`)\n  req, _ := http.NewRequest("POST", "${url}", bytes.NewReader(body))\n  req.Header.Set("Authorization", "Bearer "+os.Getenv("LYNSHEN_API_KEY"))\n  req.Header.Set("Content-Type", "application/json")\n  response, err := http.DefaultClient.Do(req)\n  if err != nil { panic(err) }\n  defer response.Body.Close()\n}`;
}

export function ApiDocsPage() {
  const { t } = useTranslation();
  const { data: site, isLoading } = usePublicSiteSettings();
  const [family, setFamily] = useState<ApiFamily>("responses");
  const [language, setLanguage] = useState<SampleLanguage>("curl");
  const [copied, setCopied] = useState(false);
  const resolution = resolvePublicApiBaseUrl(site?.api_base_url || "", window.location.origin);
  const sample = useMemo(() => resolution.baseUrl ? sampleFor(language, family, resolution.baseUrl) : "", [family, language, resolution.baseUrl]);

  const copySample = async () => {
    if (!sample || !resolution.baseUrl) return;
    await navigator.clipboard.writeText(sample);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1800);
  };

  return (
    <div className="mx-auto max-w-7xl px-4 py-12 sm:px-6 lg:px-8">
      <header className="max-w-3xl">
        <p className="font-mono text-sm font-medium text-primary">API REFERENCE</p>
        <h1 className="mt-3 font-display text-4xl font-semibold tracking-tight sm:text-5xl">{t("publicSite.docs.title")}</h1>
        <p className="mt-4 text-lg leading-8 text-muted-foreground">{t("publicSite.docs.description")}</p>
      </header>

      <section aria-labelledby="base-url-heading" className="mt-10">
        <h2 id="base-url-heading" className="text-xl font-semibold">{t("publicSite.docs.baseUrl")}</h2>
        {isLoading ? <Skeleton className="mt-4 h-14 max-w-2xl" /> : resolution.error ? (
          <div className="mt-4 flex max-w-3xl gap-3 rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-destructive"><AlertTriangle className="mt-0.5 size-5 shrink-0" /><p>{t("publicSite.docs.baseUrlError")}</p></div>
        ) : (
          <div className="mt-4 flex max-w-3xl items-center gap-3 rounded-lg border bg-card px-4 py-3"><code className="min-w-0 flex-1 overflow-x-auto text-sm">{resolution.baseUrl}</code><Button variant="ghost" size="icon" className="size-11 shrink-0" onClick={() => navigator.clipboard.writeText(resolution.baseUrl)} aria-label={t("common.copy")}><Copy /></Button></div>
        )}
      </section>

      <div className="mt-12 grid gap-10 lg:grid-cols-[240px_minmax(0,1fr)]">
        <aside><h2 className="text-sm font-semibold uppercase tracking-wider text-muted-foreground">{t("publicSite.docs.apiFamilies")}</h2><div className="mt-3 grid gap-1" role="tablist" aria-orientation="vertical">{families.map((item) => <button key={item} type="button" role="tab" aria-selected={family === item} onClick={() => setFamily(item)} className={cn("min-h-11 rounded-md px-3 text-left text-sm font-medium transition-colors duration-200 focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring", family === item ? "bg-primary text-primary-foreground" : "hover:bg-accent")}>{t(`publicSite.families.${item}`)}</button>)}</div></aside>
        <div className="min-w-0">
          <section aria-labelledby="request-example-heading">
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between"><div><h2 id="request-example-heading" className="text-2xl font-semibold">{t(`publicSite.docs.familyDescriptions.${family}Title`)}</h2><p className="mt-2 text-muted-foreground">{t(`publicSite.docs.familyDescriptions.${family}`)}</p></div><span className="w-fit rounded-md border px-2 py-1 font-mono text-xs">POST /v1/{endpointFor(family)}</span></div>
            <div className="mt-6 overflow-hidden rounded-lg border bg-foreground text-background">
              <div className="flex flex-wrap items-center gap-1 border-b border-background/15 px-3 py-2">{languages.map((item) => <button type="button" key={item} onClick={() => setLanguage(item)} className={cn("min-h-11 rounded-md px-3 text-sm font-medium transition-colors duration-200 focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-primary", language === item ? "bg-background/15 text-background" : "text-background/65 hover:text-background")}>{t(`publicSite.docs.languages.${item}`)}</button>)}<Button variant="ghost" size="icon" className="ml-auto size-11 text-background hover:bg-background/15 hover:text-background" onClick={copySample} disabled={!resolution.baseUrl} aria-label={t("common.copy")}>{copied ? <Check /> : <Copy />}</Button></div>
              {isLoading ? <div className="p-5"><Skeleton className="h-56 bg-background/10" /></div> : <pre className="max-h-[34rem] overflow-auto p-5 text-sm leading-7"><code>{sample || t("publicSite.docs.copyDisabled")}</code></pre>}
            </div>
          </section>

          <section className="mt-12 grid gap-4 md:grid-cols-2" aria-label={t("publicSite.docs.behaviorTitle")}>
            <Card className="p-6"><KeyRound className="text-primary" /><h2 className="mt-4 text-lg font-semibold">{t("publicSite.docs.authenticationTitle")}</h2><p className="mt-2 leading-7 text-muted-foreground">{t("publicSite.docs.authentication")}</p></Card>
            <Card className="p-6"><Radio className="text-primary" /><h2 className="mt-4 text-lg font-semibold">{t("publicSite.docs.streamingTitle")}</h2><p className="mt-2 leading-7 text-muted-foreground">{t("publicSite.docs.streaming")}</p></Card>
            <Card className="p-6"><Terminal className="text-primary" /><h2 className="mt-4 text-lg font-semibold">{t("publicSite.docs.errorsTitle")}</h2><p className="mt-2 leading-7 text-muted-foreground">{t("publicSite.docs.errors")}</p><ul className="mt-4 space-y-2 font-mono text-xs"><li>401 · unauthorized</li><li>403 · forbidden</li><li>429 · rate_limited</li><li>403 · model_pricing_required</li></ul></Card>
            <Card className="p-6"><AlertTriangle className="text-warning" /><h2 className="mt-4 text-lg font-semibold">{t("publicSite.docs.pricingTitle")}</h2><p className="mt-2 leading-7 text-muted-foreground">{t("publicSite.docs.pricing")}</p></Card>
          </section>
        </div>
      </div>
    </div>
  );
}

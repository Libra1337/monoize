import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ArrowRight, Boxes, CheckCircle2, CircleDollarSign, Code2, Image, MessageSquareText, Radio, ShieldCheck, Sparkles } from "lucide-react";
import { motion } from "framer-motion";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { usePublicSiteSettings } from "@/lib/swr";
import { resolvePublicApiBaseUrl } from "@/lib/public-site";

const families = [
  ["responses", Sparkles],
  ["chat", MessageSquareText],
  ["messages", Code2],
  ["gemini", Radio],
  ["images", Image],
] as const;

export function WelcomePage() {
  const { t } = useTranslation();
  const { data: site, isLoading } = usePublicSiteSettings();
  const siteName = site?.site_name || "LynShen Console";
  const base = resolvePublicApiBaseUrl(site?.api_base_url || "", window.location.origin);
  const exampleBase = base.baseUrl || "https://lynshen.org/v1";

  return (
    <div>
      <section className="relative isolate overflow-hidden border-b">
        <div className="pointer-events-none absolute inset-0 -z-10 bg-[radial-gradient(circle_at_70%_25%,hsl(var(--primary)/0.14),transparent_32%),linear-gradient(to_right,hsl(var(--border)/0.35)_1px,transparent_1px),linear-gradient(to_bottom,hsl(var(--border)/0.35)_1px,transparent_1px)] bg-[size:auto,32px_32px,32px_32px] [mask-image:linear-gradient(to_bottom,black,transparent)]" />
        <motion.div initial={{ opacity: 0, y: 16 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.24 }} className="mx-auto grid max-w-7xl gap-12 px-4 py-20 sm:px-6 sm:py-28 lg:grid-cols-[1.1fr_0.9fr] lg:items-center lg:px-8">
          <div className="max-w-3xl">
            <p className="mb-5 font-mono text-sm font-medium text-primary">API GATEWAY · MODEL ROUTING</p>
            {isLoading ? <><Skeleton className="h-14 w-full max-w-xl" /><Skeleton className="mt-3 h-14 w-4/5 max-w-lg" /><Skeleton className="mt-7 h-6 w-full max-w-2xl" /></> : (
              <>
                <h1 className="font-display text-4xl font-semibold leading-tight tracking-tight sm:text-6xl">{t("publicSite.welcome.title", { siteName })}</h1>
                <p className="mt-6 max-w-2xl text-lg leading-8 text-muted-foreground">{site?.site_description || t("publicSite.welcome.description")}</p>
              </>
            )}
            <div className="mt-8 flex flex-col gap-3 sm:flex-row">
              <Button asChild size="lg" variant="primary" className="min-h-11"><Link to="/dashboard/marketplace">{t("publicSite.welcome.exploreModels")}<ArrowRight /></Link></Button>
              <Button asChild size="lg" variant="outline" className="min-h-11"><Link to="/apidocs">{t("publicSite.welcome.readDocs")}</Link></Button>
            </div>
          </div>
          <Card className="overflow-hidden bg-card/90 shadow-sm">
            <div className="flex items-center gap-2 border-b px-4 py-3"><span className="size-2.5 rounded-full bg-destructive/70" /><span className="size-2.5 rounded-full bg-warning/70" /><span className="size-2.5 rounded-full bg-success/70" /><span className="ml-2 font-mono text-xs text-muted-foreground">request.sh</span></div>
            <pre className="overflow-x-auto p-5 text-sm leading-7"><code>{`curl ${exampleBase}/responses \\\n  -H "Authorization: Bearer $LYNSHEN_API_KEY" \\\n  -H "Content-Type: application/json" \\\n  -d '{"model":"gpt-5","input":"Hello"}'`}</code></pre>
          </Card>
        </motion.div>
      </section>

      <section className="mx-auto max-w-7xl px-4 py-16 sm:px-6 lg:px-8">
        <div className="max-w-2xl"><p className="font-mono text-sm text-primary">01 · API</p><h2 className="mt-3 font-display text-3xl font-semibold">{t("publicSite.welcome.familiesTitle")}</h2><p className="mt-3 text-base leading-7 text-muted-foreground">{t("publicSite.welcome.familiesDescription")}</p></div>
        <div className="mt-8 grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
          {families.map(([key, Icon]) => <Card key={key} className="p-5 transition-colors duration-200 hover:border-primary/40"><Icon className="size-5 text-primary" /><h3 className="mt-5 font-semibold">{t(`publicSite.families.${key}`)}</h3></Card>)}
        </div>
      </section>

      <section className="border-y bg-muted/35">
        <div className="mx-auto grid max-w-7xl gap-8 px-4 py-16 sm:px-6 lg:grid-cols-2 lg:px-8">
          <div><p className="font-mono text-sm text-primary">02 · GROUPS</p><h2 className="mt-3 font-display text-3xl font-semibold">{t("publicSite.welcome.pricingTitle")}</h2><p className="mt-4 max-w-xl text-base leading-7 text-muted-foreground">{t("publicSite.welcome.pricingDescription")}</p></div>
          <div className="grid gap-4 sm:grid-cols-2"><Card className="p-6"><Boxes className="text-primary" /><h3 className="mt-4 font-semibold">{t("publicSite.welcome.groupTitle")}</h3><p className="mt-2 leading-6 text-muted-foreground">{t("publicSite.welcome.groupDescription")}</p></Card><Card className="p-6"><CircleDollarSign className="text-primary" /><h3 className="mt-4 font-semibold">{t("publicSite.welcome.priceTitle")}</h3><p className="mt-2 leading-6 text-muted-foreground">{t("publicSite.welcome.priceDescription")}</p></Card></div>
        </div>
      </section>

      <section className="mx-auto max-w-7xl px-4 py-16 sm:px-6 lg:px-8">
        <p className="font-mono text-sm text-primary">03 · CONNECT</p><h2 className="mt-3 font-display text-3xl font-semibold">{t("publicSite.welcome.stepsTitle")}</h2>
        <ol className="mt-8 grid gap-5 md:grid-cols-3">
          {["key", "model", "request"].map((key, index) => <li key={key} className="relative rounded-lg border bg-card p-6"><span className="font-mono text-sm text-primary">0{index + 1}</span><h3 className="mt-5 text-lg font-semibold">{t(`publicSite.steps.${key}Title`)}</h3><p className="mt-2 leading-7 text-muted-foreground">{t(`publicSite.steps.${key}Description`)}</p></li>)}
        </ol>
      </section>

      <section className="border-y bg-foreground text-background">
        <div className="mx-auto grid max-w-7xl gap-8 px-4 py-16 sm:px-6 lg:grid-cols-[0.75fr_1.25fr] lg:items-center lg:px-8">
          <div><p className="font-mono text-sm text-primary">04 · HTTP</p><h2 className="mt-3 font-display text-3xl font-semibold">{t("publicSite.welcome.codeTitle")}</h2><p className="mt-4 leading-7 text-background/70">{t("publicSite.welcome.codeDescription")}</p></div>
          <pre className="overflow-x-auto rounded-lg border border-background/15 bg-background/5 p-5 text-sm leading-7"><code>{`POST /v1/chat/completions HTTP/1.1\nHost: lynshen.org\nAuthorization: Bearer $LYNSHEN_API_KEY\nContent-Type: application/json\n\n{"model":"gpt-5","messages":[{"role":"user","content":"Hello"}]}`}</code></pre>
        </div>
      </section>

      <section className="mx-auto max-w-7xl px-4 py-16 sm:px-6 lg:px-8">
        <Card className="flex flex-col gap-6 p-7 sm:flex-row sm:items-center sm:justify-between"><div className="flex gap-4"><ShieldCheck className="mt-1 size-6 shrink-0 text-success" /><div><h2 className="font-display text-2xl font-semibold">{t("publicSite.welcome.statusTitle")}</h2><p className="mt-2 text-muted-foreground">{t("publicSite.welcome.statusDescription")}</p></div></div><Button asChild variant="outline" className="min-h-11 shrink-0"><Link to="/status">{t("publicSite.welcome.viewStatus")}<CheckCircle2 /></Link></Button></Card>
      </section>
    </div>
  );
}

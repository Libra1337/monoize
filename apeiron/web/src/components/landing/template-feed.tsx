import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import useSWR from "swr";
import { Layers } from "lucide-react";
import { api, type Template } from "@/lib/api";
import { ScrollReveal, SectionKicker } from "@/components/ui/motion";
import { Skeleton } from "@/components/ui/badge";
import { GraphPreview } from "@/components/landing/graph-preview";

/** Template feed — data-driven cards, no fake thumbnails. */
export function TemplateFeed() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { data } = useSWR("landing-templates", () =>
    api.get<{ templates: Template[] }>("/templates"),
  );
  const templates = data?.templates ?? [];

  return (
    <section className="border-y border-border/60 bg-card/40">
      <div className="mx-auto max-w-5xl px-4 py-16 sm:px-6">
        <ScrollReveal className="mb-8 space-y-3">
          <SectionKicker index="03" label={t("landing.sectionTemplatesTitle")} />
          <h2 className="font-display text-2xl font-semibold tracking-tight text-balance sm:text-3xl">
            {t("landing.sectionTemplatesTitle")}
          </h2>
          <p className="text-sm text-muted-foreground">{t("landing.sectionTemplatesDescription")}</p>
        </ScrollReveal>
        <ScrollReveal delay={0.1}>
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            {templates.length === 0
              ? [0, 1, 2, 3].map((index) => <Skeleton key={index} className="h-36" />)
              : templates.slice(0, 4).map((template) => (
                  <button
                    key={template.id}
                    onClick={async () => {
                      const created = await api.post<{ id: string }>("/projects", {
                        title: template.name,
                        template_id: template.id,
                      });
                      navigate(`/canvas/${created.id}`);
                    }}
                    className="flex h-44 flex-col rounded-lg border bg-card p-4 text-left transition-colors hover:border-primary/50"
                  >
                    <span className="flex size-8 items-center justify-center rounded-md bg-muted">
                      <Layers className="h-4 w-4 text-muted-foreground" />
                    </span>
                    <span className="mt-3 text-sm font-medium">{template.name}</span>
                    <span className="mt-1 line-clamp-2 text-xs leading-relaxed text-muted-foreground">
                      {template.description}
                    </span>
                    <span className="mt-auto pt-2">
                      <GraphPreview graph={template.graph} />
                    </span>
                    <span className="font-mono text-[10px] text-muted-foreground/70">
                      {template.graph?.nodes?.length ?? 0} nodes ·{" "}
                      {template.graph?.edges?.length ?? 0} edges
                    </span>
                  </button>
                ))}
          </div>
        </ScrollReveal>
      </div>
    </section>
  );
}

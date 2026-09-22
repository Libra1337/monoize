import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { SiteHeader } from "@/components/landing/site-header";
import { Hero } from "@/components/landing/hero";
import { Pipeline } from "@/components/landing/pipeline";
import { CanvasShowcase } from "@/components/landing/canvas-showcase";
import { TemplateFeed } from "@/components/landing/template-feed";
import { Features, SiteFooter } from "@/components/landing/features";

/**
 * The landing page plays in permanent dark — a cinema stage for the
 * Anaximander narrative. The visitor's theme preference is restored on leave.
 */
export function LandingPage() {
  const { t } = useTranslation();
  useEffect(() => {
    const root = document.documentElement;
    root.classList.add("dark");
    return () => {
      const stored = localStorage.getItem("apeiron-theme");
      const dark = stored
        ? stored === "dark"
        : window.matchMedia("(prefers-color-scheme: dark)").matches;
      root.classList.toggle("dark", dark);
    };
  }, []);

  useEffect(() => {
    document.title = `${t("brand.name")} · ${t("brand.tagline")}`;
  }, [t]);

  return (
    <div className="h-dvh overflow-y-auto bg-background text-foreground">
      <SiteHeader />
      <main>
        <Hero />
        <TemplateFeed />
        <Pipeline />
        <CanvasShowcase />
        <Features />
      </main>
      <SiteFooter />
    </div>
  );
}

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

function source(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

const appSource = source("../src/App.tsx");
const layoutSource = source("../src/pages/layout.tsx");
const studioApiSource = source("../src/lib/studio-api.ts");
const canvasSource = source("../src/pages/studio/canvas.tsx");
const locales = ["en", "zh", "zh-TW", "ja"].map((locale) =>
  JSON.parse(source(`../src/locales/${locale}.json`)),
);

// STU-1..STU-5: routes, navigation, API surface, and locale completeness.
describe("Studio workbench", () => {
  test("registers the studio routes in the dashboard shell", () => {
    expect(appSource).toContain('path="studio"');
    expect(appSource).toContain('path="studio/assets"');
    expect(appSource).toContain('path="studio/p/:id"');
    expect(appSource).toContain('path="studio-admin"');
  });

  test("exposes studio entries in the user and admin sidebars", () => {
    expect(layoutSource).toContain('"/dashboard/studio"');
    expect(layoutSource).toContain('"/dashboard/studio-admin"');
    expect(layoutSource).toContain('t("nav.studio")');
    expect(layoutSource).toContain('t("nav.studioAdmin")');
  });

  test("targets the spec dashboard and admin endpoints (ST-U1/U2)", () => {
    expect(studioApiSource).toContain('"/api/dashboard/studio"');
    expect(studioApiSource).toContain('"/api/dashboard/admin/studio"');
    for (const endpoint of [
      "/projects",
      "/runs",
      "/work-order",
      "/assets",
      "/uploads",
      "/templates",
      "/settings",
    ]) {
      expect(studioApiSource).toContain(endpoint);
    }
  });

  test("canvas subscribes to the studio SSE stream (ST-X1, DC16)", () => {
    expect(canvasSource).toContain("studio-events");
    expect(canvasSource).toContain('new EventSource("/api/dashboard/studio/events")');
    expect(canvasSource).toContain("graph_patch");
    expect(canvasSource).toContain("run_update");
  });

  test("every locale carries nav and studio keys", () => {
    for (const locale of locales) {
      expect(locale.nav.studio).toBeTruthy();
      expect(locale.nav.studioAdmin).toBeTruthy();
      expect(locale.studio.title).toBeTruthy();
      expect(locale.studio.node.script).toBeTruthy();
      expect(locale.studio.agent.placeholder).toBeTruthy();
      expect(locale.studio.admin.settings).toBeTruthy();
      expect(locale.studio.assets.title).toBeTruthy();
    }
  });
});

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { PUBLIC_ROUTES, isProtectedConsolePath } from "../src/public-routes";
import { resolvePublicApiBaseUrl } from "../src/lib/public-site";

const apiDocsSource = readFileSync(
  new URL("../src/pages/api-docs.tsx", import.meta.url),
  "utf8",
);

describe("public browser route contract", () => {
  test("registers every public surface without a dashboard session", () => {
    expect(PUBLIC_ROUTES).toEqual([
      "/",
      "/login",
      "/apidocs",
      "/status",
      "/marketplace",
      "/usage-ranking",
    ]);
  });

  test("protects every Console path and keeps the public Marketplace separate", () => {
    expect(isProtectedConsolePath("/dashboard")).toBe(true);
    expect(isProtectedConsolePath("/dashboard/providers")).toBe(true);
    expect(isProtectedConsolePath("/settings")).toBe(true);
    expect(isProtectedConsolePath("/dashboard/marketplace")).toBe(true);
    expect(isProtectedConsolePath("/marketplace")).toBe(false);
    expect(isProtectedConsolePath("/usage-ranking")).toBe(false);
    expect(isProtectedConsolePath("/")).toBe(false);
  });
});

describe("public API Base URL", () => {
  test("uses the configured Base URL without a trailing slash", () => {
    expect(
      resolvePublicApiBaseUrl("https://api.example.test/v1/", "http://localhost:8080"),
    ).toEqual({ baseUrl: "https://api.example.test/v1", error: null });
  });

  test("uses the HTTPS browser origin when no Base URL is configured", () => {
    expect(resolvePublicApiBaseUrl("", "https://lynshen.org")).toEqual({
      baseUrl: "https://lynshen.org/v1",
      error: null,
    });
  });

  test("rejects an empty Base URL on a non-HTTPS origin", () => {
    expect(resolvePublicApiBaseUrl("", "http://localhost:8080")).toEqual({
      baseUrl: null,
      error: "public_api_base_url_required",
    });
  });
});

test("API Docs labels missing pricing as HTTP 403", () => {
  expect(apiDocsSource).toContain("<li>403 · model_pricing_required</li>");
  expect(apiDocsSource).not.toContain("<li>400 · model_pricing_required</li>");
});

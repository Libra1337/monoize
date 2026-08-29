import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  API_FAMILIES,
  API_SAMPLE_LANGUAGES,
  apiFamilyDefinition,
  generateApiSample,
} from "../src/lib/api-samples";

const pageSource = readFileSync(
  new URL("../src/pages/dashboard-api-docs.tsx", import.meta.url),
  "utf8",
);
const publicPageSource = readFileSync(
  new URL("../src/pages/api-docs.tsx", import.meta.url),
  "utf8",
);

describe("Dashboard API Docs", () => {
  test("defines every API family and executable language sample", () => {
    expect(API_FAMILIES).toEqual(["responses", "chat", "messages", "gemini", "images"]);
    expect(API_SAMPLE_LANGUAGES).toEqual(["curl", "python", "javascript", "go"]);

    for (const family of API_FAMILIES) {
      const definition = apiFamilyDefinition(family);
      expect(definition.method).toBe("POST");
      expect(definition.path.startsWith("/v1/")).toBe(true);
      expect(definition.successShape).toContain("id");
      expect(definition.commonErrorShape).toContain("error");
      for (const language of API_SAMPLE_LANGUAGES) {
        const sample = generateApiSample(language, family, "https://api.lynshen.org");
        expect(sample).toContain("https://api.lynshen.org/v1/");
        expect(sample).toContain("LYNSHEN_API_KEY");
        expect(sample).not.toContain("sk-");
      }
    }
  });

  test("documents supported streaming behavior explicitly", () => {
    expect(apiFamilyDefinition("responses").supportsStreaming).toBe(true);
    expect(apiFamilyDefinition("chat").supportsStreaming).toBe(true);
    expect(apiFamilyDefinition("messages").supportsStreaming).toBe(true);
    expect(apiFamilyDefinition("gemini").supportsStreaming).toBe(true);
    expect(apiFamilyDefinition("images").supportsStreaming).toBe(false);
  });

  test("renders a compact Console page with secure Base URL and copy feedback", () => {
    expect(pageSource).toContain("usePublicSiteSettings");
    expect(pageSource).toContain("resolvePublicApiBaseUrl");
    expect(pageSource).toContain("generateApiSample");
    expect(pageSource).toContain("navigator.clipboard.writeText");
    expect(pageSource).toContain("apiDocsConsole.baseUrlMissing");
    expect(pageSource).toContain("apiDocsConsole.streaming");
    expect(pageSource).toContain("apiDocsConsole.successShape");
    expect(pageSource).toContain("apiDocsConsole.commonErrors");
    expect(pageSource).toContain("<Skeleton");
    expect(pageSource).not.toContain("window.localStorage");
  });

  test("keeps the public API Docs on the shared pure generator", () => {
    expect(publicPageSource).toContain('from "@/lib/api-samples"');
    expect(publicPageSource).not.toContain("function sampleFor");
    expect(publicPageSource).not.toContain("function endpointFor");
  });

  test("defines Console API Docs copy in every locale", () => {
    for (const locale of ["en", "zh", "zh-TW", "ja"]) {
      const catalog = JSON.parse(
        readFileSync(new URL(`../src/locales/${locale}.json`, import.meta.url), "utf8"),
      );
      for (const key of [
        "title",
        "description",
        "baseUrl",
        "baseUrlMissing",
        "authentication",
        "streaming",
        "streamingSupported",
        "streamingUnsupported",
        "successShape",
        "commonErrors",
        "copy",
        "copied",
      ]) {
        expect(typeof catalog.apiDocsConsole?.[key], `${locale}: ${key}`).toBe("string");
      }
    }
  });
});

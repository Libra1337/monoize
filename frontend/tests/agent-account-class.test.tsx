import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { api } from "../src/lib/api";

function source(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

const apiSource = source("../src/lib/api.ts");
const swrSource = source("../src/lib/swr.ts");
const groupsSource = source("../src/pages/groups.tsx");
const providersSource = source("../src/pages/providers.tsx");
const usersSource = source("../src/pages/users.tsx");
const layoutSource = source("../src/pages/layout.tsx");
const wholesaleDialogSource = source("../src/pages/providers/WholesaleProviderDialog.tsx");
const locales = ["en", "zh", "zh-TW", "ja"].map((locale) =>
  JSON.parse(source(`../src/locales/${locale}.json`)),
);

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
});

// GR-E1b / PP-W: the agent class is a fourth account class whose Providers are created
// through the wholesale flow and whose catalogue is isolated like the enterprise one.
describe("Agent account-class API", () => {
  test("types the agent class into the shared union", () => {
    expect(apiSource).toContain(
      'export type AccountClass = "standard" | "enterprise" | "private" | "agent"',
    );
  });

  test("posts the wholesale copy to the dedicated endpoint", async () => {
    let url = "";
    let body = "";
    globalThis.fetch = (async (input, init) => {
      url = String(input);
      body = String(init?.body);
      return Response.json({ id: "provider-1" });
    }) as typeof fetch;

    await api.createWholesaleProvider({
      group_id: "agent-group",
      source_provider_id: "source-provider",
      multiplier: "0.8",
      model_multipliers: { "gpt-shared": "0.8" },
      confirm_public_exposure: true,
    });

    expect(url).toBe("/api/dashboard/providers/wholesale");
    expect(JSON.parse(body)).toEqual({
      group_id: "agent-group",
      source_provider_id: "source-provider",
      multiplier: "0.8",
      model_multipliers: { "gpt-shared": "0.8" },
      confirm_public_exposure: true,
    });
  });
});

describe("Agent administration", () => {
  // GR-E1b: every admin class selector must offer the agent scope, or an agent Group and its
  // wholesale Providers become unmanageable from the dashboard.
  test("offers the agent class in every admin scope selector", () => {
    expect(groupsSource).toContain('"agent"');
    expect(providersSource).toContain(
      "const ACCOUNT_CLASSES = ['standard', 'enterprise', 'private', 'agent'] as const",
    );
    expect(usersSource).toContain('const USER_SCOPES = ["standard", "enterprise", "private", "agent", "sales"]');
  });

  // PP-WF1: the agent scope must not open the ordinary editor, whose upstream fields would
  // bypass the wholesale multiplier materialization.
  test("opens the wholesale dialog instead of the ordinary editor on the agent scope", () => {
    expect(providersSource).toContain("WholesaleProviderDialog");
    expect(providersSource).toContain("openCreate");
    expect(providersSource).toContain("setWholesaleOpen(true)");
  });

  // PP-WF2: the wholesale dialog picks a non-agent source, prefills the source's effective
  // multipliers, and requires the public-exposure confirmation.
  test("builds the wholesale dialog from a non-agent source with editable multipliers", () => {
    expect(wholesaleDialogSource).toContain("group.account_class === 'agent'");
    expect(wholesaleDialogSource).toContain("account_class !== 'agent'");
    expect(wholesaleDialogSource).toContain("multiplier_override ?? source.multiplier");
    expect(wholesaleDialogSource).toContain("createWholesaleProviderOptimistic");
    expect(wholesaleDialogSource).toContain("providerPublicExposureConfirm");
    expect(wholesaleDialogSource).toContain("Skeleton");
    expect(swrSource).toContain("createWholesaleProviderOptimistic");
    expect(swrSource).toContain("SWR_KEYS.MARKETPLACE_MODELS");
  });

  // PP-WF2a: with no agent Group the select would be a dead end, so the dialog must offer
  // inline Group creation with the same public-exposure confirmation.
  test("creates an agent Group inline when the registry has none", () => {
    expect(wholesaleDialogSource).toContain("createGroupOptimistic");
    expect(wholesaleDialogSource).toContain("account_class: 'agent'");
    expect(wholesaleDialogSource).toContain("is_public: true");
    expect(wholesaleDialogSource).toContain("groups.publicExposureConfirm");
    expect(wholesaleDialogSource).toContain("setGroupId(created.id)");
  });

  // DL5d: the agent class is a full-featured account and must keep the standard sidebar; the
  // reduced set stays gated on enterprise alone.
  test("gives the agent class the full sidebar", () => {
    expect(layoutSource).toContain('user?.account_class === "enterprise"');
    expect(layoutSource).not.toContain('user?.account_class !== "standard"');
  });

  test("labels the agent class and wholesale action in all locales", () => {
    for (const locale of locales) {
      expect(locale.accountClass.agent).toBeString();
      expect(locale.users.scopes.agent).toBeString();
      expect(locale.providers.addWholesaleProvider).toBeString();
    }
  });
});

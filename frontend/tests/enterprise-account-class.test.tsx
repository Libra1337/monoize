import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { api } from "../src/lib/api";

function source(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

const apiSource = source("../src/lib/api.ts");
const swrSource = source("../src/lib/swr.ts");
const usersSource = source("../src/pages/users.tsx");
const groupsSource = source("../src/pages/groups.tsx");
const providersSource = source("../src/pages/providers.tsx");
const providerDialogSource = source("../src/pages/providers/ProviderDialog.tsx");
const layoutSource = source("../src/pages/layout.tsx");
const locales = ["en", "zh", "zh-TW", "ja"].map((locale) =>
  JSON.parse(source(`../src/locales/${locale}.json`)),
);

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("Enterprise account-class API", () => {
  test("sends the destructive confirmation to the dedicated endpoint", async () => {
    let url = "";
    let body = "";
    globalThis.fetch = (async (input, init) => {
      url = String(input);
      body = String(init?.body);
      return Response.json({ id: "user-1", account_class: "enterprise" });
    }) as typeof fetch;

    await api.updateUserAccountClass("user-1", "enterprise");

    expect(url).toBe("/api/dashboard/users/user-1/account-class");
    expect(JSON.parse(body)).toEqual({
      account_class: "enterprise",
      confirm_delete_api_keys: true,
    });
  });

  test("types users and Groups with one immutable account class", () => {
    expect(apiSource).toContain('export type AccountClass = "standard" | "enterprise"');
    expect(apiSource).toContain("account_class: AccountClass");
    expect(apiSource).toContain("account_class?: AccountClass");
  });
});

describe("Enterprise administration", () => {
  test("switches user class only through a separate destructive confirmation", () => {
    // JSX props evaluate while the page renders, before the Dialog mounts, so the account
    // class block must narrow `editUser` instead of reading it through `?.`. Reading
    // `editUser.account_class` under an `editUser?.` guard blanks the whole Users page.
    // JSX props evaluate while the page renders, before the Dialog mounts, so every
    // non-optional `editUser` read must sit after a guard that narrows it. Reading
    // `editUser.account_class` under an `editUser?.` guard blanks the whole Users page.
    const narrowingGuard = usersSource.indexOf(
      'editUser !== null && editUser.role !== "super_admin"',
    );
    expect(narrowingGuard).toBeGreaterThan(-1);
    const firstUnguardedRead = usersSource.indexOf("editUser.account_class");
    expect(firstUnguardedRead).toBeGreaterThan(narrowingGuard);
    expect(usersSource).toContain("accountClassTarget");
    expect(usersSource).toContain("updateUserAccountClassOptimistic");
    expect(usersSource).toContain('"users.accountClassDeleteKeysWarning"');
    expect(swrSource).toContain("updateUserAccountClassOptimistic");
    expect(swrSource).toContain("SWR_KEYS.MARKETPLACE_MODELS");
  });

  test("filters and reorders Groups inside the selected class", () => {
    expect(groupsSource).toContain('useState<AccountClass>("standard")');
    expect(groupsSource).toContain("group.account_class === accountClass");
    expect(groupsSource).toContain("account_class: accountClass");
    expect(groupsSource).toContain("visibleGroups");
  });

  test("filters Providers through their Group class and constrains the editor", () => {
    expect(providersSource).toContain('useState<AccountClass>("standard")');
    expect(providersSource).toContain("groupById.get(provider.group_id)?.account_class");
    expect(providersSource).toContain("accountClass={accountClass}");
    expect(providerDialogSource).toContain("accountClass: AccountClass");
    expect(providerDialogSource).toContain("group.account_class === accountClass");
  });
});

describe("Enterprise navigation", () => {
  test("keeps only the approved concise Enterprise destinations", () => {
    expect(layoutSource).toContain("enterpriseNavItems");
    expect(layoutSource).toContain('user?.account_class === "enterprise"');
    expect(layoutSource).toContain('to: "/dashboard/wallet"');
    expect(layoutSource).toContain('to: "/dashboard/tokens"');
    expect(layoutSource).toContain('to: "/dashboard/usage"');
    expect(layoutSource).toContain('to: "/dashboard/logs"');
    expect(layoutSource).toContain('to: "/dashboard/marketplace"');
    expect(layoutSource).toContain('to: "/dashboard/api-docs"');
  });

  test("ships class labels and destructive warnings in all locales", () => {
    for (const locale of locales) {
      expect(locale.accountClass.standard).toBeString();
      expect(locale.accountClass.enterprise).toBeString();
      expect(locale.users.accountClassDeleteKeysWarning).toBeString();
      expect(locale.groups.accountClassDescription).toBeString();
      expect(locale.providers.accountClassDescription).toBeString();
    }
  });
});

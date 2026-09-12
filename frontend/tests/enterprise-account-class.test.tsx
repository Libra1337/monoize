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
  // DL5d: the private class is isolated like enterprise but is a full-featured account, so
  // the reduced sidebar must be gated on enterprise alone. Testing the condition rather than
  // the rendered output is what catches a future `!== "standard"` refactor, which would
  // silently strip the private class of the overview, ranking, status, and playground pages.
  test("gives the private class the full sidebar, not the reduced Enterprise one", () => {
    expect(layoutSource).toContain('user?.account_class === "enterprise"');
    expect(layoutSource).not.toContain('user?.account_class !== "standard"');

    const start = layoutSource.indexOf("const visibleNavItems");
    expect(start).toBeGreaterThan(-1);
    const condition = layoutSource.slice(start, layoutSource.indexOf(";", start));
    expect(condition).not.toContain("private");
    expect(condition).toContain("enterpriseNavItems");
    expect(condition).toContain("navItems");
  });

  // DL-UM1 and DL-UM2: sales agents are ordinary standard-class accounts, so a grouping that
  // filtered on account_class alone would leave them mixed in with regular users. The
  // precedence in scopeOf is what keeps each user in exactly one grouping.
  test("groups the user list and keeps agents out of the other groupings", () => {
    expect(usersSource).toContain('const USER_SCOPES = ["standard", "enterprise", "private", "agent", "sales"]');

    const start = usersSource.indexOf("function scopeOf");
    expect(start).toBeGreaterThan(-1);
    const body = usersSource.slice(start, usersSource.indexOf("}", start));
    // The agent check must come first, or an enterprise-class agent lands in enterprise.
    expect(body.indexOf("is_sales_agent")).toBeLessThan(body.indexOf("account_class"));

    // DL-UM3: the account-class switch keeps offering the real classes, never sales.
    expect(usersSource).toContain('["standard", "enterprise", "private", "agent"]');
    const switchStart = usersSource.indexOf("setAccountClassTarget");
    expect(usersSource.slice(0, switchStart)).not.toContain('next: "sales"');

    // DL-UM4: the summary counts the visible grouping.
    expect(usersSource).toContain("for (const user of scopedUsers)");
    expect(usersSource).toContain("data={scopedUsers}");
  });

  test("labels every user grouping in all locales", () => {
    for (const locale of locales) {
      for (const scope of ["standard", "enterprise", "private", "agent", "sales"]) {
        expect(locale.users.scopes[scope]).toBeString();
      }
    }
  });

  // GR-E1a: the third class must reach every admin scope selector, or a private Group and
  // its Providers become unmanageable from the dashboard.
  test("offers the private class wherever account class is selected", () => {
    for (const locale of locales) {
      expect(locale.accountClass.private).toBeString();
    }
    for (const source of [groupsSource, providersSource]) {
      expect(source).toContain("private");
    }
    expect(usersSource).toContain('["standard", "enterprise", "private", "agent"]');
  });

  // DL5c: assert against the Enterprise array itself. A whole-file `toContain` cannot tell
  // the two navigation sets apart, because every Enterprise route also appears in the
  // standard one, so it would pass even if Store were missing from Enterprise.
  test("keeps the approved concise Enterprise destinations, including Store", () => {
    expect(layoutSource).toContain("enterpriseNavItems");
    expect(layoutSource).toContain('user?.account_class === "enterprise"');

    const start = layoutSource.indexOf("const enterpriseNavItems = [");
    expect(start).toBeGreaterThan(-1);
    const block = layoutSource.slice(start, layoutSource.indexOf("];", start));
    const routes = [...block.matchAll(/to: "([^"]+)"/g)].map((match) => match[1]);

    expect(routes).toEqual([
      "/dashboard/wallet",
      "/dashboard/store",
      "/dashboard/orders",
      "/dashboard/tokens",
      "/dashboard/usage",
      "/dashboard/usage/cache",
      "/dashboard/logs",
      "/dashboard/marketplace",
      "/dashboard/api-docs",
    ]);
    // Store checkout is the only self-service way to add balance, so it must stay reachable.
    expect(routes).toContain("/dashboard/store");
    expect(routes).not.toContain("/dashboard/playground");
    expect(routes).not.toContain("/dashboard/usage-ranking");
  });

  test("ships class labels and destructive warnings in all locales", () => {
    for (const locale of locales) {
      expect(locale.accountClass.standard).toBeString();
      expect(locale.accountClass.enterprise).toBeString();
      expect(locale.accountClass.private).toBeString();
      expect(locale.users.accountClassDeleteKeysWarning).toBeString();
      expect(locale.groups.accountClassDescription).toBeString();
      expect(locale.providers.accountClassDescription).toBeString();
    }
  });
});

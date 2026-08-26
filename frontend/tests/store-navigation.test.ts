import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const readSource = (relativePath: string) =>
  readFileSync(new URL(relativePath, import.meta.url), "utf8");

const appSource = readSource("../src/App.tsx");
const layoutSource = readSource("../src/pages/layout.tsx");
const locales = {
  en: JSON.parse(readSource("../src/locales/en.json")),
  zh: JSON.parse(readSource("../src/locales/zh.json")),
  "zh-TW": JSON.parse(readSource("../src/locales/zh-TW.json")),
  ja: JSON.parse(readSource("../src/locales/ja.json")),
};

describe("Store dashboard navigation", () => {
  test("registers separate Store, Orders, and admin Store routes", () => {
    expect(appSource).toContain('path="store"');
    expect(appSource).toContain('path="orders"');
    expect(appSource).toContain('path="store-admin"');
    expect(appSource).toContain("StoreAdminRoute");
  });

  test("uses the approved icons and animated active navigation", () => {
    expect(layoutSource).toContain("ShoppingBag");
    expect(layoutSource).toContain("ReceiptText");
    expect(layoutSource).toContain("BadgeDollarSign");
    expect(layoutSource).toContain('layoutId={layoutId}');
  });

  test("keeps Store Management inside the admin-only navigation block", () => {
    const adminBlock = layoutSource.slice(
      layoutSource.indexOf("const adminNavItems"),
      layoutSource.indexOf("return (", layoutSource.indexOf("const adminNavItems")),
    );
    expect(adminBlock).toContain('/dashboard/store-admin');
    expect(adminBlock).toContain('t("nav.storeManagement")');
    expect(layoutSource).toContain("isAdmin &&");
  });

  test("defines Store navigation labels in every supported locale", () => {
    for (const locale of Object.values(locales)) {
      expect(locale.nav.store).toBeString();
      expect(locale.nav.orders).toBeString();
      expect(locale.nav.storeManagement).toBeString();
    }
  });

  test("uses 实得 wording in Simplified Chinese", () => {
    const zhText = JSON.stringify(locales.zh);
    expect(zhText).toContain("实得");
    expect(zhText).toContain("实得金额");
    expect(zhText).not.toContain("实到");
  });
});

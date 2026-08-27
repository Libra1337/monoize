import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { StoreModeContent } from "../src/pages/store/store-mode-content";

describe("Store mode content", () => {
  test("renders redemption without payment methods or an order summary", () => {
    const html = renderToStaticMarkup(
      <StoreModeContent
        activeTab="redeem"
        purchaseContent={(
          <>
            <section data-testid="order-summary" />
            <section data-testid="payment-methods" />
          </>
        )}
        redemptionContent={<section data-testid="redemption-panel" />}
      />,
    );

    expect(html).toContain("redemption-panel");
    expect(html).not.toContain("order-summary");
    expect(html).not.toContain("payment-methods");
  });

  test("renders purchase content for balance and plan tabs", () => {
    for (const activeTab of ["balance", "plan"] as const) {
      const html = renderToStaticMarkup(
        <StoreModeContent
          activeTab={activeTab}
          purchaseContent={<section data-testid="purchase-content" />}
          redemptionContent={<section data-testid="redemption-panel" />}
        />,
      );
      expect(html).toContain("purchase-content");
      expect(html).not.toContain("redemption-panel");
    }
  });
});

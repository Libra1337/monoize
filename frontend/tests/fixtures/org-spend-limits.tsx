import { createRoot } from "react-dom/client";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { createInstance } from "i18next";
import { I18nextProvider } from "react-i18next";
import { mutate } from "swr";
import { OrgLimitsPage } from "../../src/pages/org/limits";
import { STORE_EXCHANGE_RATE_KEY } from "../../src/hooks/use-store-exchange-rate";

const i18n = createInstance();
await i18n.init({ lng: "en", resources: {} });

createRoot(document.getElementById("root")!).render(
  <I18nextProvider i18n={i18n}>
    <button onClick={() => mutate(STORE_EXCHANGE_RATE_KEY, { cny_per_usd: "0" }, false)}>
      Invalidate exchange rate
    </button>
    <MemoryRouter initialEntries={["/org/test-org/limits"]}>
      <Routes>
        <Route path="/org/:orgId/limits" element={<OrgLimitsPage />} />
      </Routes>
    </MemoryRouter>
  </I18nextProvider>,
);

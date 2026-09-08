import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { RequestLog } from "../src/lib/api";
import {
  LogRowCells,
  RequestLogModelTooltipDetails,
} from "../src/pages/request-logs/log-row-cells";

function requestLog(): RequestLog {
  return {
    id: "log-layout",
    created_at: "2026-08-28T00:00:00.000Z",
    status: "success",
    is_stream: false,
    model: "MODEL_VISIBLE",
    provider: { id: "provider-id", name: "PROVIDER_ADMIN_ONLY" },
    channel: { id: "channel-id", name: "CHANNEL_ADMIN_ONLY" },
    user: { id: "user-id", username: "user" },
    api_key: { id: "key-id", name: "key" },
    tokens: {},
    timing: {},
    billing: {},
    error: {},
    affinity: {},
  };
}

function renderRow(isAdmin: boolean): string {
  return renderToStaticMarkup(
    <table>
      <tbody>
        <tr>
          <LogRowCells
            affinityTargetNames={new Map()}
            log={requestLog()}
            isAdmin={isAdmin}
            showIp={false}
            t={(key) => key}
            onOpenCapture={() => {}}
            onTooltipOpenChange={() => {}}
          />
        </tr>
      </tbody>
    </table>,
  );
}

function renderModelTooltip(isAdmin: boolean): string {
  return renderToStaticMarkup(
    <RequestLogModelTooltipDetails
      log={requestLog()}
      isAdmin={isAdmin}
      t={(key) => key}
    />,
  );
}

describe("request log model cell layout (FL9)", () => {
  test("does not render Provider identifiers in the user Model tooltip", () => {
    const html = renderModelTooltip(false);

    expect(html).toContain("MODEL_VISIBLE");
    expect(html).not.toContain("provider-id");
    expect(renderModelTooltip(true)).toContain("provider-id");
  });

  test("renders one centered model line without Channel data for a user", () => {
    const html = renderRow(false);

    expect(html).toContain("MODEL_VISIBLE");
    expect(html).toContain("h-9");
    expect(html).toContain("justify-center");
    expect(html).not.toContain("CHANNEL_ADMIN_ONLY");
    expect(html).not.toContain("PROVIDER_ADMIN_ONLY");
    expect(html).not.toContain("channel-id");
    expect(html).not.toContain("provider-id");
  });

  test("retains the visible Channel line for an admin", () => {
    const html = renderRow(true);

    expect(html).toContain("MODEL_VISIBLE");
    expect(html).toContain("CHANNEL_ADMIN_ONLY");
  });
});

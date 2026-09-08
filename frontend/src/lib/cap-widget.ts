import capWasmUrl from "@cap.js/wasm/browser/cap_wasm_bg.wasm?url";
import pakoUrl from "pako/dist/pako_inflate.min.js?url";

let widgetLoad: Promise<unknown> | null = null;

export function loadCapWidget() {
  window.CAP_CUSTOM_WASM_URL = capWasmUrl;
  window.CAP_PAKO_URL = pakoUrl;
  const nonce = document
    .querySelector<HTMLMetaElement>('meta[name="csp-nonce"]')
    ?.content.trim();
  if (nonce) {
    window.CAP_SCRIPT_NONCE = nonce;
    window.CAP_CSS_NONCE = nonce;
  }
  widgetLoad ??= import("cap-widget");
  return widgetLoad;
}

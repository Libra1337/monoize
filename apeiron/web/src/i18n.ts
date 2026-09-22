import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import zh from "./locales/zh.json";
import zhTw from "./locales/zh-TW.json";
import ja from "./locales/ja.json";

const stored = localStorage.getItem("apeiron-language");
const initial =
  stored ??
  (() => {
    const languages = navigator.languages?.map((raw) => raw.toLowerCase()) ?? [
      navigator.language.toLowerCase(),
    ];
    if (languages.some((raw) => raw.startsWith("zh") && (raw.includes("tw") || raw.includes("hk")))) {
      return "zh-TW";
    }
    if (languages.some((raw) => raw.startsWith("zh"))) return "zh";
    if (languages.some((raw) => raw.startsWith("ja"))) return "ja";
    return "en";
  })();

i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    zh: { translation: zh },
    "zh-TW": { translation: zhTw },
    ja: { translation: ja },
  },
  lng: initial,
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export const LANGUAGES = ["en", "zh", "zh-TW", "ja"] as const;

export function cycleLanguage() {
  const order = [...LANGUAGES];
  const next = order[(order.indexOf(i18n.language as "en") + 1) % order.length];
  void i18n.changeLanguage(next as string);
  localStorage.setItem("apeiron-language", next);
}

export default i18n;

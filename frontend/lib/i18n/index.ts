import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./en.json";
import zhCN from "./zh-CN.json";

export const SUPPORTED_LANGUAGES = ["zh-CN", "en"] as const;
export type SupportedLanguage = (typeof SUPPORTED_LANGUAGES)[number];

/** "system" means follow the browser/OS; otherwise a concrete language. */
export type LanguagePreference = SupportedLanguage | "system";

const STORAGE_KEY = "golish.language";

function readPreference(): LanguagePreference {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "zh-CN" || v === "en" || v === "system") return v;
  } catch {
    /* localStorage unavailable (Tauri webview, SSR test) */
  }
  return "system";
}

/** Resolve "system" / persisted preference to an actual language code. */
function resolveLanguage(): SupportedLanguage {
  const pref = readPreference();
  if (pref === "zh-CN" || pref === "en") return pref;
  // pref === "system": derive from navigator/OS.
  if (typeof navigator !== "undefined") {
    const nav = (navigator.language || "").toLowerCase();
    if (nav.startsWith("zh")) return "zh-CN";
  }
  return "en";
}

i18n.use(initReactI18next).init({
  // Bypass i18next-browser-languagedetector entirely. We saw repeated reports
  // of `changeLanguage("zh-CN")` not propagating to subscribers in Tauri's
  // webview after multiple re-spawns; resolving the language ourselves at
  // boot avoids the detector's async resolution path completely.
  lng: resolveLanguage(),
  resources: {
    "zh-CN": { translation: zhCN },
    en: { translation: en },
  },
  fallbackLng: "en",
  supportedLngs: SUPPORTED_LANGUAGES,
  interpolation: { escapeValue: false },
  returnEmptyString: false,
  react: {
    useSuspense: false,
  },
});

function syncHtmlLang(lng: string) {
  if (typeof document !== "undefined") {
    document.documentElement.lang = lng;
  }
}
syncHtmlLang(i18n.language);
i18n.on("languageChanged", syncHtmlLang);

// Dev-only diagnostics handle. Run `__golishI18nDebug()` in the Tauri webview
// console to see the full i18n state in one shot. Kept after shipping — costs
// nothing and is useful when a future regression breaks the switch again.
if (typeof window !== "undefined") {
  (window as unknown as { __golishI18nDebug?: unknown }).__golishI18nDebug = () => {
    const out = {
      storedPref: readPreference(),
      i18nLanguage: i18n.language,
      i18nLanguages: i18n.languages,
      htmlLang: typeof document !== "undefined" ? document.documentElement.lang : null,
      hasZh: i18n.hasResourceBundle("zh-CN", "translation"),
      hasEn: i18n.hasResourceBundle("en", "translation"),
      sample_active: i18n.t("settings.languageHint"),
      sample_zh: i18n.t("settings.languageHint", { lng: "zh-CN" }),
      sample_en: i18n.t("settings.languageHint", { lng: "en" }),
    };
    // eslint-disable-next-line no-console
    console.log("[golish-i18n-debug]", out);
    return out;
  };
}

export function getLanguagePreference(): LanguagePreference {
  return readPreference();
}

export async function setLanguagePreference(pref: LanguagePreference): Promise<void> {
  try {
    if (pref === "system") {
      localStorage.removeItem(STORAGE_KEY);
    } else {
      localStorage.setItem(STORAGE_KEY, pref);
    }
  } catch {
    /* localStorage unavailable */
  }
  // Compute the concrete language code the same way `resolveLanguage()`
  // does at boot, then ask i18next to switch to it live. With the
  // simplified init (no async detector, single inline-resource set,
  // useSuspense:false, explicit I18nextProvider at the root) the
  // `languageChanged` event reaches every `useTranslation` subscriber and
  // they re-render synchronously — no reload needed.
  const next: SupportedLanguage = pref === "zh-CN" || pref === "en" ? pref : resolveLanguage();
  if (i18n.language !== next) {
    await i18n.changeLanguage(next);
  }
  if (typeof document !== "undefined") {
    document.documentElement.lang = next;
  }
}

export default i18n;

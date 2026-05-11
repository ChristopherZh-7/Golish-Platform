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
// console to see the full i18n state in one shot — useful when "switch did
// nothing" is reported and we need to know whether i18next actually flipped
// language vs. the issue is somewhere else.
if (typeof window !== "undefined") {
  (window as unknown as { __golishI18nDebug?: unknown }).__golishI18nDebug = () => {
    const out = {
      storedPref: readPreference(),
      i18nLanguage: i18n.language,
      i18nLanguages: i18n.languages,
      htmlLang: typeof document !== "undefined" ? document.documentElement.lang : null,
      hasZh: i18n.hasResourceBundle("zh-CN", "translation"),
      hasEn: i18n.hasResourceBundle("en", "translation"),
      // Resolve the same key in both languages and at the active language so
      // we can see whether the active language is actually serving Chinese.
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
  // Force a full webview reload so the new preference is picked up by
  // `resolveLanguage()` in i18n.init(). We tried three subscription-based
  // fixes first (load:currentOnly drop / useSuspense:false / explicit
  // I18nextProvider) — all reproduced as "no-op" in the user's Tauri
  // webview because dozens of `useTranslation` consumers sit behind
  // React.lazy() boundaries that the languageChanged event apparently
  // can't traverse. Reload is correctness-first; the preference is
  // already persisted so the next boot uses it.
  if (typeof window !== "undefined") {
    window.location.reload();
  }
}

export default i18n;

import i18n from "i18next";
import LanguageDetector from "i18next-browser-languagedetector";
import { initReactI18next } from "react-i18next";
import en from "./en.json";
import zhCN from "./zh-CN.json";

export const SUPPORTED_LANGUAGES = ["zh-CN", "en"] as const;
export type SupportedLanguage = (typeof SUPPORTED_LANGUAGES)[number];

/**
 * `system` means "follow the browser/OS — i.e. let the LanguageDetector decide".
 * Stored as a separate sentinel in localStorage so we can distinguish
 * "user explicitly chose Chinese" from "user chose to follow the system".
 */
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

// Custom detector that respects the `system` sentinel: when the user chose
// "follow the system", we return undefined so the next detector (navigator)
// runs. When the user chose a concrete language, we return it.
const detector = new LanguageDetector();
detector.addDetector({
  name: "customLocalStorage",
  lookup() {
    const pref = readPreference();
    if (pref === "system") return undefined;
    return pref;
  },
  cacheUserLanguage() {
    /* writes are routed through `setLanguagePreference` below. */
  },
});

i18n
  .use(detector)
  .use(initReactI18next)
  .init({
    resources: {
      "zh-CN": { translation: zhCN },
      en: { translation: en },
    },
    fallbackLng: "en",
    supportedLngs: SUPPORTED_LANGUAGES,
    // Map zh / zh-TW / zh-HK / zh-SG → zh-CN so users with non-mainland zh
    // browsers still see Chinese instead of falling back to English.
    nonExplicitSupportedLngs: true,
    load: "currentOnly",
    interpolation: { escapeValue: false },
    returnEmptyString: false,
    detection: {
      // Manual `golish.language` wins. Then navigator. We *don't* read i18next's
      // own cookie or `i18nextLng` localStorage — that would shadow our own
      // sentinel and confuse "system" mode after a refresh.
      order: ["customLocalStorage", "navigator", "htmlTag"],
      caches: [],
      lookupLocalStorage: STORAGE_KEY,
    },
  });

function syncHtmlLang(lng: string) {
  if (typeof document !== "undefined") {
    document.documentElement.lang = lng;
  }
}
syncHtmlLang(i18n.language);
i18n.on("languageChanged", syncHtmlLang);

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
  if (pref === "system") {
    // Re-detect from navigator. The detector chain skips customLocalStorage
    // (returns undefined for "system") and falls through to navigator.
    const detected = i18n.services.languageDetector.detect();
    const next = Array.isArray(detected) ? detected[0] : detected;
    await i18n.changeLanguage(next || "en");
  } else {
    await i18n.changeLanguage(pref);
  }
}

export default i18n;

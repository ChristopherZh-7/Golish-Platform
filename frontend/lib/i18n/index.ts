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
    // We ship resources inline (no backend plugin), so let i18next pre-load
    // every supportedLng up front. `load: "currentOnly"` would tell it to
    // try to *fetch* zh-CN on first changeLanguage — with no backend that's
    // a silent no-op and `t()` keeps returning English.
    interpolation: { escapeValue: false },
    returnEmptyString: false,
    react: {
      // Don't suspend on language change. With Suspense enabled, the very
      // first `changeLanguage("zh-CN")` would suspend Settings → AppearanceSettings
      // (a lazy()-loaded module) and Radix Select would render the *previous*
      // resolved snapshot, making the switch look like a no-op.
      useSuspense: false,
    },
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

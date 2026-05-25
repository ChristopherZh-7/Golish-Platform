import i18n from "i18next";
import LanguageDetector from "i18next-browser-languagedetector";
import { initReactI18next } from "react-i18next";
import en from "./en.json";
import zhCN from "./zh-CN.json";

export type AppLanguage = "system" | "en" | "zh-CN";

export const LANGUAGE_STORAGE_KEY = "golish.language";

export const LANGUAGE_OPTIONS: Array<{ value: AppLanguage; label: string }> = [
  { value: "system", label: "System default" },
  { value: "en", label: "English" },
  { value: "zh-CN", label: "简体中文" },
];

function isAppLanguage(value: string | null): value is Exclude<AppLanguage, "system"> {
  return value === "en" || value === "zh-CN";
}

function resolveSystemLanguage(): Exclude<AppLanguage, "system"> {
  if (typeof navigator !== "undefined" && navigator.language.toLowerCase().startsWith("zh")) {
    return "zh-CN";
  }
  return "en";
}

export function getStoredAppLanguage(): AppLanguage {
  if (typeof localStorage === "undefined") {
    return "system";
  }
  try {
    const stored = localStorage.getItem(LANGUAGE_STORAGE_KEY);
    return isAppLanguage(stored) ? stored : "system";
  } catch {
    return "system";
  }
}

export function applyAppLanguage(language: AppLanguage): void {
  const next = language === "system" ? resolveSystemLanguage() : language;
  try {
    if (language === "system") {
      localStorage.removeItem(LANGUAGE_STORAGE_KEY);
    } else {
      localStorage.setItem(LANGUAGE_STORAGE_KEY, language);
    }
  } catch {
    // Ignore storage failures; language still changes for the current session.
  }
  void i18n.changeLanguage(next);
}

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      "zh-CN": { translation: zhCN },
      en: { translation: en },
    },
    fallbackLng: "en",
    interpolation: { escapeValue: false },
    detection: {
      order: ["localStorage", "navigator", "htmlTag"],
      lookupLocalStorage: LANGUAGE_STORAGE_KEY,
      caches: [],
    },
  });

export default i18n;

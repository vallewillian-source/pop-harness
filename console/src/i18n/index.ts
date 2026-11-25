import { useSyncExternalStore, type ReactNode } from "react";
import { flatCatalogs, type TranslationKey } from "./catalog";
import {
    formatCurrency,
    formatDate,
    formatDateTime,
    formatNumber,
    formatTime,
} from "./format";
import type { I18n, Locale, LocaleMeta, TranslationParams } from "./types";

const STORAGE_KEY = "poph-console.locale";
const DEFAULT_LOCALE: Locale = "pt-BR";

export const LOCALES: Record<Locale, LocaleMeta> = {
    "pt-BR": { short: "PT", label: "Português (Brasil)" },
    "en-US": { short: "EN", label: "English (United States)" },
};

const listeners = new Set<() => void>();

function isLocale(value: unknown): value is Locale {
    return value === "pt-BR" || value === "en-US";
}

function readStoredLocale(): Locale {
    try {
        const stored = globalThis.localStorage?.getItem(STORAGE_KEY);
        return isLocale(stored) ? stored : DEFAULT_LOCALE;
    } catch {
        return DEFAULT_LOCALE;
    }
}

function persistLocale(locale: Locale): void {
    try {
        globalThis.localStorage?.setItem(STORAGE_KEY, locale);
    } catch {
        return;
    }
}

function interpolate(template: string, params?: TranslationParams): string {
    if (!params) return template;
    return template.replace(/\{(\w+)\}/g, (match, name: string) => {
        const value = params[name];
        return value === undefined ? match : String(value);
    });
}

let current: Locale = readStoredLocale();

export function getLocale(): Locale {
    return current;
}

export function translate(key: TranslationKey, params?: TranslationParams): string {
    const template =
        flatCatalogs[current][key] ?? flatCatalogs[DEFAULT_LOCALE][key] ?? key;
    return interpolate(template, params);
}

function applyDocument(locale: Locale): void {
    if (typeof document === "undefined") return;
    document.documentElement.lang = locale;
    document.title = translate("common.documentTitle");
}

export function setLocale(locale: Locale): void {
    if (!isLocale(locale)) return;
    persistLocale(locale);
    if (locale === current) {
        applyDocument(locale);
        return;
    }
    current = locale;
    applyDocument(locale);
    listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
        listeners.delete(listener);
    };
}

function getSnapshot(): Locale {
    return current;
}

function getServerSnapshot(): Locale {
    return DEFAULT_LOCALE;
}

export function useLocale(): Locale {
    return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

export function useI18n(): I18n {
    useLocale();
    return {
        locale: current,
        setLocale,
        translate,
        formatDate,
        formatDateTime,
        formatTime,
        formatNumber,
        formatCurrency,
    };
}

export function I18nProvider({ children }: { children: ReactNode }): ReactNode {
    useLocale();
    return children;
}

applyDocument(current);

export type Locale = "pt-BR" | "en-US";

export type TranslationParams = Record<string, string | number>;

export type LocaleMeta = {
    short: string;
    label: string;
};

export type I18n = {
    locale: Locale;
    setLocale: (locale: Locale) => void;
    translate: (key: import("./catalog").TranslationKey, params?: TranslationParams) => string;
    formatDate: (value: import("./format").DateInput) => string;
    formatDateTime: (value: import("./format").DateInput) => string;
    formatTime: (value: import("./format").DateInput) => string;
    formatNumber: (value: number, options?: Intl.NumberFormatOptions) => string;
    formatCurrency: (value: number, currency?: string) => string;
};

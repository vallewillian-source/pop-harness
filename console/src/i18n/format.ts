import { getLocale } from "./index";

export type DateInput = Date | number | string;

function toDate(value: DateInput): Date {
    return value instanceof Date ? value : new Date(value);
}

export function formatDate(value: DateInput): string {
    return new Intl.DateTimeFormat(getLocale(), {
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
    }).format(toDate(value));
}

export function formatDateTime(value: DateInput): string {
    return new Intl.DateTimeFormat(getLocale(), {
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
    }).format(toDate(value));
}

export function formatTime(value: DateInput): string {
    return new Intl.DateTimeFormat(getLocale(), {
        hour: "2-digit",
        minute: "2-digit",
    }).format(toDate(value));
}

export function formatNumber(value: number, options?: Intl.NumberFormatOptions): string {
    return new Intl.NumberFormat(getLocale(), options).format(value);
}

export function formatCurrency(value: number, currency = "BRL"): string {
    return new Intl.NumberFormat(getLocale(), { style: "currency", currency }).format(value);
}

import { beforeEach, describe, expect, it, vi } from "vitest";

const STORAGE_KEY = "poph-console.locale";

function flattenCatalog(catalog: CatalogShape): Map<string, string> {
    const flat = new Map<string, string>();
    for (const [domain, entries] of Object.entries(catalog)) {
        for (const [name, value] of Object.entries(entries)) {
            flat.set(`${domain}.${name}`, value);
        }
    }
    return flat;
}

type CatalogShape = Record<string, Record<string, string>>;

function placeholders(template: string): string[] {
    return [...template.matchAll(/\{(\w+)\}/g)].map((match) => match[1] ?? "").sort();
}

beforeEach(() => {
    window.localStorage.clear();
    vi.resetModules();
    document.documentElement.lang = "";
    document.title = "";
});

describe("locale store", () => {
    it("falls back to pt-BR when nothing is stored", async () => {
        const { getLocale } = await import("./index");

        expect(getLocale()).toBe("pt-BR");
        expect(document.documentElement.lang).toBe("pt-BR");
        expect(document.title).toBe("POP Harness — Console");
        expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull();
    });

    it("switches to en-US, persists the choice and updates the document", async () => {
        const { getLocale, setLocale, translate } = await import("./index");

        setLocale("en-US");

        expect(getLocale()).toBe("en-US");
        expect(window.localStorage.getItem(STORAGE_KEY)).toBe("en-US");
        expect(document.documentElement.lang).toBe("en-US");
        expect(translate("client.httpError", { status: 503 })).toBe("the API responded 503");
    });

    it("reads the stored locale on load", async () => {
        window.localStorage.setItem(STORAGE_KEY, "en-US");

        const { getLocale } = await import("./index");

        expect(getLocale()).toBe("en-US");
        expect(document.documentElement.lang).toBe("en-US");
    });

    it("ignores an invalid stored locale", async () => {
        window.localStorage.setItem(STORAGE_KEY, "fr-FR");

        const { getLocale } = await import("./index");

        expect(getLocale()).toBe("pt-BR");
    });

    it("notifies subscribers once per locale change", async () => {
        const { setLocale, getLocale, useLocale } = await import("./index");
        const counts: string[] = [];
        const Probe = () => {
            counts.push(useLocale());
            return null;
        };
        const { createElement } = await import("react");
        const { act, render } = await import("@testing-library/react");
        render(createElement(Probe));

        act(() => setLocale("en-US"));
        act(() => setLocale("en-US"));
        act(() => setLocale("pt-BR"));

        expect(getLocale()).toBe("pt-BR");
        expect(counts).toEqual(["pt-BR", "en-US", "pt-BR"]);
    });
});

describe("translate", () => {
    it("interpolates parameters and keeps unknown placeholders", async () => {
        const { translate } = await import("./index");

        expect(translate("client.networkError", { reason: "offline" })).toBe(
            "não foi possível falar com a instância do POP Harness (offline)",
        );
        expect(translate("client.networkError")).toContain("{reason}");
    });

    it("translates the same key in both locales", async () => {
        const { setLocale, translate } = await import("./index");

        expect(translate("client.healthError", { status: 500 })).toBe(
            "a instância respondeu 500",
        );

        setLocale("en-US");

        expect(translate("client.healthError", { status: 500 })).toBe(
            "the instance responded 500",
        );
    });
});

describe("formatters", () => {
    const instant = new Date(2026, 10, 24, 15, 30, 0);

    it("formats in pt-BR", async () => {
        const { setLocale } = await import("./index");
        const { formatCurrency, formatDate, formatDateTime, formatNumber, formatTime } =
            await import("./format");

        setLocale("pt-BR");

        expect(formatDate(instant)).toBe("24/11/2026");
        expect(formatTime(instant)).toBe("15:30");
        expect(formatDateTime(instant)).toContain("24/11/2026");
        expect(formatNumber(1234.56)).toBe("1.234,56");
        expect(formatCurrency(1234.56)).toContain("1.234,56");
    });

    it("formats in en-US", async () => {
        const { setLocale } = await import("./index");
        const { formatCurrency, formatDate, formatDateTime, formatNumber, formatTime } =
            await import("./format");

        setLocale("en-US");

        expect(formatDate(instant)).toBe("11/24/2026");
        expect(formatTime(instant)).toMatch(/^03:30/);
        expect(formatDateTime(instant)).toContain("11/24/2026");
        expect(formatNumber(1234.56)).toBe("1,234.56");
        expect(formatCurrency(1234.56)).toContain("1,234.56");
    });
});

describe("catalog parity", () => {
    it("has the same keys, no empty value and the same placeholders in both locales", async () => {
        const { enUS, ptBR } = await import("./catalog");

        const flatPt = flattenCatalog(ptBR);
        const flatEn = flattenCatalog(enUS);

        expect([...flatEn.keys()].sort()).toEqual([...flatPt.keys()].sort());

        for (const [key, value] of flatPt) {
            const translated = flatEn.get(key) ?? "";
            expect(value.trim(), key).not.toBe("");
            expect(translated.trim(), key).not.toBe("");
            expect(placeholders(translated), key).toEqual(placeholders(value));
        }
    });

    it("covers the eighteen domains", async () => {
        const { ptBR } = await import("./catalog");

        expect(Object.keys(ptBR)).toHaveLength(18);
    });
});

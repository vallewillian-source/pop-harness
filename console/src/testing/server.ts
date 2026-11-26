
import { vi } from "vitest";

export type Call = {
    "method": string;
    url: string;
    "headers": Record<string, string>;
    "corpo": unknown;
    "raw"?: unknown;
};

type ApiResponse = {
    status: number;
    "corpo"?: unknown;
    texto?: string;
    "headers"?: Record<string, string>;
};

type Route = {
    "method": string;
    "excerpt": string;
    "respond": (call: Call) => ApiResponse;
};

export class FakeServer {
    readonly "calls": Call[] = [];
    private "routes": Route[] = [];

    "respond"(
        method: string,
        excerpt: string,
        response: ApiResponse | ((call: Call) => ApiResponse),
    ): this {
        this["routes"].push({
            "method": method.toUpperCase(),
            "excerpt": excerpt,
            "respond": typeof response === "function" ? response : () => response,
        });
        return this;
    }

    "install"(): this {
        vi.stubGlobal(
            "fetch",
            vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
                const url =
                    typeof input === "string"
                        ? input
                        : input instanceof URL
                          ? input.toString()
                          : input.url;
                const method = (init?.method ?? "GET").toUpperCase();
                const headers: Record<string, string> = {};
                new Headers(init?.headers).forEach((value, chave) => {
                    headers[chave.toLowerCase()] = value;
                });
                let body: unknown;
                if (typeof init?.body === "string") {
                    try {
                        body = JSON.parse(init.body);
                    } catch {
                        body = init.body;
                    }
                }
                const call: Call = { "method": method, url, "headers": headers, "corpo": body, "raw": init?.body };
                this["calls"].push(call);

                const route = this["routes"].find(
                    (candidate) =>
                        candidate["method"] === method && url.includes(candidate["excerpt"]),
                );
                if (!route) {
                    throw new Error(`sem rota falsa para ${method} ${url}`);
                }
                const { status, "corpo": dados, texto, "headers": responseHeaders } = route["respond"](call);
                const content =
                    texto !== undefined
                        ? texto
                        : dados === undefined || status === 204
                          ? ""
                          : JSON.stringify(dados);
                return {
                    status,
                    ok: status >= 200 && status < 300,
                    headers: new Headers(responseHeaders),
                    text: async () => content,
                    json: async () => JSON.parse(content),
                    blob: async () => new Blob([content]),
                } as unknown as Response;
            }),
        );
        return this;
    }

    "callsFor"(method: string, excerpt: string): Call[] {
        return this["calls"].filter(
            (call) => call["method"] === method.toUpperCase() && call.url.includes(excerpt),
        );
    }
}

export function apiError(
    status: number,
    code: string,
    message: string,
    details: unknown = null,
): ApiResponse {
    return { status, "corpo": { code, message, details } };
}

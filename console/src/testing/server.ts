
import { vi } from "vitest";

export type Call = {
    "metodo": string;
    url: string;
    "cabecalhos": Record<string, string>;
    "corpo": unknown;
    "bruto"?: unknown;
};

type ApiResponse = {
    status: number;
    "corpo"?: unknown;
    texto?: string;
    "cabecalhos"?: Record<string, string>;
};

type Route = {
    "metodo": string;
    "trecho": string;
    "responder": (call: Call) => ApiResponse;
};

export class FakeServer {
    readonly "chamadas": Call[] = [];
    private "rotas": Route[] = [];

    "responder"(
        method: string,
        excerpt: string,
        response: ApiResponse | ((call: Call) => ApiResponse),
    ): this {
        this["rotas"].push({
            "metodo": method.toUpperCase(),
            "trecho": excerpt,
            "responder": typeof response === "function" ? response : () => response,
        });
        return this;
    }

    "instalar"(): this {
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
                const call: Call = { "metodo": method, url, "cabecalhos": headers, "corpo": body, "bruto": init?.body };
                this["chamadas"].push(call);

                const route = this["rotas"].find(
                    (candidate) =>
                        candidate["metodo"] === method && url.includes(candidate["trecho"]),
                );
                if (!route) {
                    throw new Error(`sem rota falsa para ${method} ${url}`);
                }
                const { status, "corpo": dados, texto, "cabecalhos": responseHeaders } = route["responder"](call);
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

    "chamadasPara"(method: string, excerpt: string): Call[] {
        return this["chamadas"].filter(
            (call) => call["metodo"] === method.toUpperCase() && call.url.includes(excerpt),
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

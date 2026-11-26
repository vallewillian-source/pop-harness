
import type { components } from "./schema";
import { translate } from "../i18n";

export type Org = components["schemas"]["OrgJson"];
export type OrgList = components["schemas"]["ListaJson_OrgJson"];
export type ResponseError = components["schemas"]["RespostaErro"];

export type ListInstance = {
    id: string;
    situacao: string;
    orgs: number;
    mensagem?: string | null;
};

export type OrgInList = Org & { agents_active_count?: number };

export type ClusterOrgList = {
    items: OrgInList[];
    next_cursor?: string | null;
    "instancias"?: ListInstance[];
};

export class ApiError extends Error {
    readonly status: number;
    readonly "code": string;
    readonly "details"?: unknown;

    constructor(status: number, code: string, mensagem: string, details?: unknown) {
        super(mensagem);
        this.name = "ErroApi";
        this.status = status;
        this["code"] = code;
        this["details"] = details;
    }

    get "naoAutorizado"(): boolean {
        return this.status === 401;
    }

    get "semServidor"(): boolean {
        return this.status === 0;
    }

    get "semPermissao"(): boolean {
        return this.status === 403;
    }
}

type ExpiryListener = () => void;

let currentToken: string | null = null;
let onExpire: ExpiryListener | null = null;

export function setToken(token: string | null): void {
    currentToken = token;
}

export function whenExpired(listener: ExpiryListener | null): void {
    onExpire = listener;
}

const BASE = "";

function networkMessage(error: unknown): string {
    const motivo = error instanceof Error ? error.message : String(error);
    return translate("client.networkError", { reason: motivo });
}

export type RequestOptions = {
    "encerraSessao"?: boolean;
};

export async function request<T>(
    path: string,
    init: RequestInit = {},
    options: RequestOptions = {},
): Promise<T> {
    const headers = new Headers(init.headers);
    if (init.body !== undefined && !headers.has("content-type")) {
        headers.set("content-type", "application/json");
    }
    if (currentToken) {
        headers.set("authorization", `Bearer ${currentToken}`);
    }
    headers.set("accept", "application/json");

    let response: Response;
    try {
        response = await fetch(`${BASE}${path}`, { ...init, headers: headers });
    } catch (error) {
        throw new ApiError(0, "rede", networkMessage(error));
    }

    if (response.status === 204) {
        return undefined as T;
    }

    const body = readBody(await response.text());
    if (!response.ok) {
        throw responseError(response, body, options["encerraSessao"] ?? true);
    }

    return body as T;
}

function readBody(texto: string): unknown {
    if (!texto) return undefined;
    try {
        return JSON.parse(texto) as unknown;
    } catch {
        return undefined;
    }
}

function responseError(response: Response, body: unknown, endSession: boolean): ApiError {
    const error = body as Partial<ResponseError> | undefined;
    const code = typeof error?.code === "string" ? error.code : `http_${response.status}`;
    const status = response.status;
    const mensagem =
        typeof error?.message === "string" && error.message
            ? error.message
            : translate("client.httpError", { status });
    if (status === 401 && endSession && currentToken !== null) {
        onExpire?.();
    }
    return new ApiError(status, code, mensagem, error?.details);
}

export async function requestFlow(
    path: string,
    init: RequestInit = {},
    options: RequestOptions = {},
): Promise<Response> {
    const headers = new Headers(init.headers);
    if (currentToken) {
        headers.set("authorization", `Bearer ${currentToken}`);
    }
    headers.set("accept", "text/event-stream");

    let response: Response;
    try {
        response = await fetch(`${BASE}${path}`, { ...init, headers: headers });
    } catch (error) {
        if (error instanceof DOMException && error.name === "AbortError") throw error;
        throw new ApiError(0, "rede", networkMessage(error));
    }

    if (response.ok) {
        return response;
    }
    const body = readBody(await response.text());
    throw responseError(response, body, options["encerraSessao"] ?? true);
}

export type BinaryResponse = {
    blob: Blob;
    headers: Headers;
};

export async function requestBytes(
    path: string,
    options: RequestOptions = {},
): Promise<BinaryResponse> {
    const headers = new Headers();
    if (currentToken) {
        headers.set("authorization", `Bearer ${currentToken}`);
    }

    let response: Response;
    try {
        response = await fetch(`${BASE}${path}`, { method: "GET", headers: headers });
    } catch (error) {
        throw new ApiError(0, "rede", networkMessage(error));
    }

    if (!response.ok) {
        const body = readBody(await response.text());
        throw responseError(response, body, options["encerraSessao"] ?? true);
    }

    return { blob: await response.blob(), headers: response.headers };
}

export type Health = {
    status: string;
    version: string;
    uptime_seconds: number;
    databases: { harness: number; rag: number };
};

export async function health(): Promise<Health> {
    let response: Response;
    try {
        response = await fetch(`${BASE}/healthz`, { headers: { accept: "application/json" } });
    } catch (error) {
        throw new ApiError(0, "rede", networkMessage(error));
    }
    if (!response.ok) {
        throw new ApiError(response.status, "healthz", translate("client.healthError", { status: response.status }));
    }
    return (await response.json()) as Health;
}

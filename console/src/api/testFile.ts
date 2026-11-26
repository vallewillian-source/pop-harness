
import { translate } from "../i18n";
import { requestBytes } from "./client";

const IMAGE_TYPES = new Set(["image/jpeg", "image/png", "image/gif", "image/webp"]);

export type FileOrigin = {
    chave: string;
    nome: string;
    mime: string;
    de: "cliente" | "agente";
    "caminho": string;
};

export type DownloadedFile = {
    blob: Blob;
    nome: string | null;
};

export function attachmentPath(
    org: string,
    agente: string,
    conversa: string,
    mensagem: string,
    attachment: string,
): string {
    return (
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}` +
        `/test-chat/${encodeURIComponent(conversa)}/messages/${encodeURIComponent(mensagem)}` +
        `/attachments/${encodeURIComponent(attachment)}/content`
    );
}

export function canSeeOnScreen(mime: string): boolean {
    return IMAGE_TYPES.has(mime.toLowerCase());
}

export function headerName(value: string | null): string | null {
    if (!value) return null;

    const estendido = /filename\*\s*=\s*UTF-8\x27\x27([^;]+)/i.exec(value);
    if (estendido?.[1]) {
        try {
            const nome = decodeURIComponent(estendido[1].trim());
            if (nome) return nome;
        } catch {
        }
    }

    const simple =
        /filename\s*=\s*\x22((?:[^\x22\\]|\\.)*)\x22/i.exec(value) ?? /filename\s*=\s*([^;]+)/i.exec(value);
    const nome = simple?.[1]?.trim().replace(/\\(.)/g, "$1");
    return nome ? nome : null;
}

export function agentFilePath(org: string, agente: string, file: string): string {
    return (
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}` +
        `/files/${encodeURIComponent(file)}/content`
    );
}

export function clientAttachmentOrigin(
    org: string,
    agente: string,
    conversa: string,
    mensagem: string,
    attachment: { id: string; name: string; mime: string },
): FileOrigin {
    return {
        chave: `anexo:${attachment.id}`,
        nome: attachment.name,
        mime: attachment.mime,
        de: "cliente",
        "caminho": attachmentPath(org, agente, conversa, mensagem, attachment.id),
    };
}

export function agentFileOrigin(
    org: string,
    agente: string,
    file: { id: string; "arquivo_id": string; name: string; mime: string },
): FileOrigin {
    return {
        chave: `envio:${file.id}`,
        nome: file.name,
        mime: file.mime,
        de: "agente",
        "caminho": agentFilePath(org, agente, file["arquivo_id"]),
    };
}

export function sendLabel(estado: string): string {
    switch (estado) {
        case "sent":
            return translate("testChat.uploadSent");
        case "pending":
        case "sending":
            return translate("testChat.uploadQueued");
        case "failed":
            return translate("testChat.uploadFailed");
        default:
            return estado;
    }
}

export async function downloadContent(path: string): Promise<DownloadedFile> {
    const response = await requestBytes(path);
    return {
        blob: response.blob,
        nome: headerName(response["cabecalhos"].get("content-disposition")),
    };
}

export async function downloadTestAttachment(
    org: string,
    agente: string,
    conversa: string,
    mensagem: string,
    attachment: string,
): Promise<DownloadedFile> {
    return downloadContent(attachmentPath(org, agente, conversa, mensagem, attachment));
}

export async function readAsDataUrl(file: Blob, mime: string): Promise<string> {
    const bytes = new Uint8Array(await file.arrayBuffer());
    let binario = "";
    for (let start = 0; start < bytes.length; start += 0x8000) {
        binario += String.fromCharCode(...bytes.subarray(start, start + 0x8000));
    }
    return `data:${mime};base64,${btoa(binario)}`;
}

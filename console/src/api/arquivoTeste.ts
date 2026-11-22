/**
 * Ver e baixar o arquivo que o cliente mandou no chat de teste (HR-46c).
 *
 * A rota é `GET …/test-chat/{conversa}/messages/{mensagem}/attachments/{anexo}/content` (HR-46b, em
 * `docs/API.md`). Ela devolve os bytes crus com o tipo gravado, o nome em `Content-Disposition` e o
 * SHA-256 em `x-content-sha256`. A chamada sai pelo mesmo cliente HTTP das outras (com o token).
 *
 * O arquivo que o **agente** enviou (HR-46e) usa a rota da biblioteca do agente
 * (`…/files/{arquivo}/content`, HR-46b) e entra pelo mesmo caminho: `files_out` do turno no
 * histórico traz o `arquivo_id`. Os dois casos saem daqui com a mesma origem de arquivo.
 */

import { pedirBytes } from "./cliente";

/** Tipos de imagem que a tela mostra no lugar. Outros tipos (PDF, Office) são só baixados. */
const TIPOS_DE_IMAGEM = new Set(["image/jpeg", "image/png", "image/gif", "image/webp"]);

/**
 * Um arquivo que a tela pode ver ou baixar, seja do cliente (anexo) seja do agente (biblioteca).
 * `chave` identifica o botão (download em curso e erro); `caminho` é a rota de conteúdo.
 */
export type OrigemDeArquivo = {
    chave: string;
    nome: string;
    mime: string;
    /** De quem veio: só muda o texto da tela ("enviada pelo cliente" ou "pelo agente"). */
    de: "cliente" | "agente";
    caminho: string;
};

/** Dados do arquivo como o servidor o entregou. */
export type ArquivoBaixado = {
    blob: Blob;
    /** Nome do `Content-Disposition` (`filename*` UTF-8 tem preferência); `null` se não vier. */
    nome: string | null;
};

/** Caminho da rota de conteúdo de um anexo de mensagem. */
export function caminhoDoAnexo(
    org: string,
    agente: string,
    conversa: string,
    mensagem: string,
    anexo: string,
): string {
    return (
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}` +
        `/test-chat/${encodeURIComponent(conversa)}/messages/${encodeURIComponent(mensagem)}` +
        `/attachments/${encodeURIComponent(anexo)}/content`
    );
}

/** `true` quando a tela mostra o arquivo como imagem (prévia). */
export function podeVerNaTela(mime: string): boolean {
    return TIPOS_DE_IMAGEM.has(mime.toLowerCase());
}

/**
 * Tira o nome do `Content-Disposition`. Prefere `filename*=UTF-8''…` (acentos e espaços); cai para
 * `filename="…"`. Sem nenhum dos dois, devolve `null` e quem chama usa o nome do histórico.
 */
export function nomeDoCabecalho(valor: string | null): string | null {
    if (!valor) return null;

    const estendido = /filename\*\s*=\s*UTF-8''([^;]+)/i.exec(valor);
    if (estendido?.[1]) {
        try {
            const nome = decodeURIComponent(estendido[1].trim());
            if (nome) return nome;
        } catch {
            // Percentual malformado: segue para o `filename` simples.
        }
    }

    const simples =
        /filename\s*=\s*"((?:[^"\\]|\\.)*)"/i.exec(valor) ?? /filename\s*=\s*([^;]+)/i.exec(valor);
    const nome = simples?.[1]?.trim().replace(/\\(.)/g, "$1");
    return nome ? nome : null;
}

/** Caminho da rota de conteúdo de um arquivo da biblioteca do agente (o que ele enviou). */
export function caminhoDoArquivoDoAgente(org: string, agente: string, arquivo: string): string {
    return (
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}` +
        `/files/${encodeURIComponent(arquivo)}/content`
    );
}

/** Origem de um anexo que o cliente mandou numa mensagem da conversa de teste. */
export function origemDoAnexoDoCliente(
    org: string,
    agente: string,
    conversa: string,
    mensagem: string,
    anexo: { id: string; name: string; mime: string },
): OrigemDeArquivo {
    return {
        chave: `anexo:${anexo.id}`,
        nome: anexo.name,
        mime: anexo.mime,
        de: "cliente",
        caminho: caminhoDoAnexo(org, agente, conversa, mensagem, anexo.id),
    };
}

/** Origem de um arquivo que o agente enviou (`files_out` do turno), pela biblioteca dele. */
export function origemDoArquivoDoAgente(
    org: string,
    agente: string,
    arquivo: { id: string; arquivo_id: string; name: string; mime: string },
): OrigemDeArquivo {
    return {
        chave: `envio:${arquivo.id}`,
        nome: arquivo.name,
        mime: arquivo.mime,
        de: "agente",
        caminho: caminhoDoArquivoDoAgente(org, agente, arquivo.arquivo_id),
    };
}

/** Rótulo do estado do envio do arquivo do agente. Estado desconhecido sai como veio. */
export function rotuloDoEnvio(estado: string): string {
    switch (estado) {
        case "sent":
            return "enviado";
        case "pending":
        case "sending":
            return "na fila para envio";
        case "failed":
            return "falhou ao enviar";
        default:
            return estado;
    }
}

/** Baixa os bytes de uma rota de conteúdo (anexo ou arquivo do agente). Erro sai como `ErroApi`. */
export async function baixarConteudo(caminho: string): Promise<ArquivoBaixado> {
    const resposta = await pedirBytes(caminho);
    return {
        blob: resposta.blob,
        nome: nomeDoCabecalho(resposta.cabecalhos.get("content-disposition")),
    };
}

/** Baixa os bytes do anexo de uma mensagem da conversa. Erro sai como `ErroApi`. */
export async function baixarAnexoTeste(
    org: string,
    agente: string,
    conversa: string,
    mensagem: string,
    anexo: string,
): Promise<ArquivoBaixado> {
    return baixarConteudo(caminhoDoAnexo(org, agente, conversa, mensagem, anexo));
}

/**
 * Converte a imagem em `data:` para a prévia. A CSP do Console libera `img-src 'self' data:` e não
 * libera `blob:`, então uma imagem em `blob:` seria bloqueada pelo navegador. O conteúdo fica só no
 * atributo da imagem, nunca em log. O `mime` é o gravado no anexo (a prévia só vale para imagem).
 */
export async function lerComoDataUrl(arquivo: Blob, mime: string): Promise<string> {
    const bytes = new Uint8Array(await arquivo.arrayBuffer());
    let binario = "";
    // Em pedaços: passar tudo de uma vez como argumentos estoura a pilha em arquivo de vários MB.
    for (let inicio = 0; inicio < bytes.length; inicio += 0x8000) {
        binario += String.fromCharCode(...bytes.subarray(inicio, inicio + 0x8000));
    }
    return `data:${mime};base64,${btoa(binario)}`;
}

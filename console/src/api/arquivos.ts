/**
 * Biblioteca de arquivos do agente (HR-43): listar, enviar e remover.
 *
 * O contrato é o OpenAPI (`schema.d.ts`). A biblioteca não tem versão nem `If-Match`: o arquivo não
 * é editado, só enviado de novo ou removido (docs/BIBLIOTECA_ARQUIVOS.md).
 *
 * O envio é **binário**: o corpo é o próprio arquivo e os metadados vão na query (`filename`,
 * `name`, `description`), como o servidor espera. O limite de 16 MB é conferido aqui antes de sair
 * da tela e de novo pelo servidor, que é a referência.
 */

import { pedir } from "./cliente";
import type { components } from "./schema";

/** Metadados de um arquivo da biblioteca (sem o conteúdo). */
export type Arquivo = components["schemas"]["ArquivoJson"];

/** Limite do servidor por arquivo: 16 MiB (`poph-files`, `LIMITE_ARQUIVO_PADRAO`). */
export const LIMITE_ARQUIVO_BYTES = 16 * 1024 * 1024;

/** Nome mostrado ao cliente: até 200 caracteres (docs/BIBLIOTECA_ARQUIVOS.md). */
export const LIMITE_NOME_ARQUIVO = 200;

/** Descrição que o modelo lê para decidir o envio: até 500 caracteres. */
export const LIMITE_DESCRICAO_ARQUIVO = 500;

/** Limite da listagem: a tela ainda não pagina (ver limites no relatório). */
export const LIMITE_LISTA_ARQUIVOS = 200;

/** Tamanho em MB com uma casa decimal, para a mensagem ("3,2 MB"). */
export function tamanhoEmMb(bytes: number): string {
    return (bytes / (1024 * 1024)).toFixed(1).replace(".", ",") + " MB";
}

function base(org: string, agente: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/files`;
}

/** Lista os arquivos do agente (primeira página, até 200). */
export async function listarArquivos(org: string, agente: string): Promise<Arquivo[]> {
    const resposta = await pedir<{ items: Arquivo[]; next_cursor?: string | null }>(
        `${base(org, agente)}?limit=${LIMITE_LISTA_ARQUIVOS}`,
    );
    return resposta.items;
}

/**
 * Envia um arquivo para a biblioteca do agente.
 *
 * Arquivo acima do limite nem sai da tela: o erro vem com o tamanho e o limite. O servidor também
 * recusa (400 com o motivo em `details.reason`); quem chama mostra o motivo como veio.
 */
export async function enviarArquivo(
    org: string,
    agente: string,
    arquivo: File,
    nome: string,
    descricao: string,
): Promise<Arquivo> {
    const parametros = new URLSearchParams({ filename: arquivo.name });
    if (nome.trim()) parametros.set("name", nome.trim());
    if (descricao.trim()) parametros.set("description", descricao.trim());
    return pedir<Arquivo>(`${base(org, agente)}?${parametros.toString()}`, {
        method: "POST",
        headers: { "content-type": "application/octet-stream" },
        body: arquivo,
    });
}

/** Remove o arquivo da biblioteca (o conteúdo fica no disco até a retenção apagar o que não tem dono). */
export async function excluirArquivo(org: string, agente: string, id: string): Promise<void> {
    await pedir<unknown>(`${base(org, agente)}/${encodeURIComponent(id)}`, { method: "DELETE" });
}

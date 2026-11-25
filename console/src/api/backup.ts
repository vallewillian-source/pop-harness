/**
 * Exportar e importar a organização pela tela de operação (HR-48e-2).
 *
 * As rotas estão publicadas e são as únicas que esta tela usa (não há rota nova aqui):
 * - `GET /admin/v1/orgs/{org}/backup` (sem query = sem conversas) e `?com_conversas=true`, que
 *   devolve o arquivo como anexo (`Content-Disposition`), com o mesmo formato da linha de comando.
 * - `POST /admin/v1/orgs/{org}/backup/import`, corpo cru `application/json` com o texto do arquivo,
 *   até 64 MB. A org do caminho é o destino; nada que já existe lá é apagado.
 *
 * Erros viram texto de operador pelo `code` (nunca pelo `message` do servidor). Segredo nunca sai no
 * arquivo: o relatório só diz quantas integrações precisam do segredo digitado de novo.
 */

import { ErroApi, pedir, pedirBytes } from "./cliente";
import { nomeDoCabecalho } from "./arquivoTeste";
import { tamanhoEmMb } from "./arquivos";
import { translate } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";

export { tamanhoEmMb };

/** Teto do corpo da importação (o servidor recusa acima disso com `413`). */
export const LIMITE_IMPORTACAO_BYTES = 64 * 1024 * 1024;

/** Quantos conflitos a tela lista antes de dizer "e mais N". */
export const CONFLITOS_MOSTRADOS = 5;

/** Caminho da exportação. Sem conversas, não há query (padrão do servidor e da linha de comando). */
export function caminhoDaExportacao(org: string, comConversas: boolean): string {
    const base = `/admin/v1/orgs/${encodeURIComponent(org)}/backup`;
    return comConversas ? `${base}?com_conversas=true` : base;
}

/**
 * Caminho do envio do arquivo para a organização de destino. Sem `sobrescrever`, o servidor recusa
 * (`409 importacao_sobrescreve`) quando a importação atualizaria linhas que já existem; o parâmetro
 * só vai na URL quando o operador confirmou a sobrescrita.
 */
export function caminhoDaImportacao(org: string, sobrescrever = false): string {
    const base = `/admin/v1/orgs/${encodeURIComponent(org)}/backup/import`;
    return sobrescrever ? `${base}?sobrescrever=true` : base;
}

/** Arquivo baixado da exportação: o conteúdo e o nome que o servidor sugeriu. */
export type ArquivoExportado = { blob: Blob; nome: string };

/**
 * Baixa o arquivo da organização. O nome vem do `Content-Disposition`; sem ele, usa um nome de
 * reserva com o id da org (a tela nunca deixa o arquivo sem nome).
 */
export async function exportarOrganizacao(org: string, comConversas: boolean): Promise<ArquivoExportado> {
    const { blob, cabecalhos } = await pedirBytes(caminhoDaExportacao(org, comConversas));
    const nome = nomeDoCabecalho(cabecalhos.get("content-disposition")) ?? `organizacao-${org}-backup.json`;
    return { blob, nome };
}

/** Relatório que o servidor devolve quando a importação grava (`200`). */
export type RelatorioImportacao = {
    org: { id: string; criada: boolean };
    com_conversas: boolean;
    tabelas: Record<string, { inseridas: number; atualizadas: number }>;
    total: number;
    segredos_a_redigitar: number;
    indice: {
        base_conhecimento: { agentes: number; trechos: number };
        produtos: { trechos: number };
    };
    /** `null` quando a reindexação rodou; texto técnico quando o modelo de embedding faltou. */
    aviso: string | null;
};

/**
 * Envia o texto do arquivo para importar na organização. O corpo é o próprio texto (não é
 * multipart), com `content-type: application/json`. `sobrescrever` é a autorização explícita do
 * operador para atualizar linhas que já existem na org.
 */
export async function importarOrganizacao(
    org: string,
    texto: string,
    sobrescrever = false,
): Promise<RelatorioImportacao> {
    return pedir<RelatorioImportacao>(caminhoDaImportacao(org, sobrescrever), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: texto,
    });
}

/** Lê o texto de um arquivo escolhido pelo operador (FileReader cobre navegador e testes). */
export function lerTextoDoArquivo(arquivo: Blob): Promise<string> {
    return new Promise((resolver, rejeitar) => {
        const leitor = new FileReader();
        leitor.onload = () => resolver(typeof leitor.result === "string" ? leitor.result : "");
        leitor.onerror = () =>
            rejeitar(leitor.error ?? new Error(translate("orgs.backupReadFileError")));
        leitor.readAsText(arquivo);
    });
}

/**
 * Baixa o arquivo pelo navegador: cria uma URL temporária, dispara o download com o nome do
 * servidor e revoga a URL logo depois. A tela não guarda a URL, então sair da tela não deixa nada.
 */
export function baixarArquivo(arquivo: ArquivoExportado): void {
    const url = URL.createObjectURL(arquivo.blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = arquivo.nome;
    link.rel = "noopener";
    document.body.appendChild(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
}

/** `true` quando o arquivo passa do teto e não deve sair da tela. */
export function passaDoLimite(bytes: number): boolean {
    return bytes > LIMITE_IMPORTACAO_BYTES;
}

/** Nomes de operador para as tabelas que o arquivo leva (o resto aparece com o nome técnico). */
export const NOMES_DE_TABELA: Record<string, TranslationKey> = {
    agents: "orgs.tableAgents",
    agent_instructions: "orgs.tableAgentInstructions",
    departments: "orgs.tableDepartments",
    product_categories: "orgs.tableProductCategories",
    agent_product_categories: "orgs.tableAgentProductCategories",
    products: "orgs.tableProducts",
    knowledge_groups: "orgs.tableKnowledgeGroups",
    agent_knowledge_groups: "orgs.tableAgentKnowledgeGroups",
    knowledge_items: "orgs.tableKnowledgeItems",
    file_library: "orgs.tableFileLibrary",
    integrations: "orgs.tableIntegrations",
    conversations: "orgs.tableConversations",
    messages: "orgs.tableMessages",
    turns: "orgs.tableTurns",
    attachments: "orgs.tableAttachments",
    outbox: "orgs.tableOutbox",
    inbox: "orgs.tableInbox",
    carts: "orgs.tableCarts",
    cart_items: "orgs.tableCartItems",
};

/** Nome de operador de uma tabela; tabela fora da lista mantém o nome técnico. */
export function nomeDaTabela(tabela: string): string {
    const key = Object.prototype.hasOwnProperty.call(NOMES_DE_TABELA, tabela)
        ? NOMES_DE_TABELA[tabela]
        : undefined;
    return key === undefined ? tabela : translate(key);
}

/**
 * Texto de um conflito como o operador vê: sem crase de Markdown, e com o nome de operador da tabela
 * quando o servidor manda a tabela na frente ("`product_categories` <id> (…)" vira
 * "Categorias de produto: <id> (…)").
 */
export function textoDeConflito(item: string): string {
    const limpo = item.replace(/`/g, "").trim();
    const casa = /^([a-z_]+)\s+(.*)$/.exec(limpo);
    if (!casa) return limpo;
    const [, tabela = "", resto = ""] = casa;
    if (!Object.prototype.hasOwnProperty.call(NOMES_DE_TABELA, tabela)) return limpo;
    return `${nomeDaTabela(tabela)}: ${resto}`;
}

/** Erro de importação ou exportação já traduzido, com o que o operador precisa saber. */
export type FalhaDeBackup = {
    titulo: string;
    texto: string;
    /** Conflitos listados (só no `409` de importação). */
    conflitos?: { itens: string[]; mais: number };
    /** Versão do arquivo e a que a instalação entende (só no `400` de versão desconhecida). */
    versoes?: { arquivo: number; instalacao: number };
    /** `true` quando tentar de novo pode resolver (queda de conexão, sessão, erro do servidor). */
    podeTentarDeNovo: boolean;
};

/** Lê um número de um objeto de detalhes sem confiar no formato. */
function numeroDoDetalhe(detalhes: unknown, campo: string): number | undefined {
    if (!detalhes || typeof detalhes !== "object") return undefined;
    const valor = (detalhes as Record<string, unknown>)[campo];
    return typeof valor === "number" && Number.isFinite(valor) ? valor : undefined;
}

/** Lista de textos do detalhe (conflitos), descartando o que não for texto. */
function listaDoDetalhe(detalhes: unknown, campo: string): string[] {
    if (!detalhes || typeof detalhes !== "object") return [];
    const valor = (detalhes as Record<string, unknown>)[campo];
    return Array.isArray(valor) ? valor.filter((item): item is string => typeof item === "string") : [];
}

/**
 * Recusa da importação que sobrescreveria o que já existe na org (`409 importacao_sobrescreve`).
 * Nada foi gravado; a tela mostra o que seria atualizado e oferece autorizar.
 */
export type RecusaSobrescrita = {
    /** Linhas que seriam atualizadas, por tabela (tabela técnica → quantidade). */
    tabelas: Record<string, number>;
    /** Total de linhas que seriam atualizadas. */
    linhas: number;
    /** Até 5 exemplos legíveis, no formato "tabela: identificador". */
    exemplos: string[];
};

/** Quantos exemplos a tela mostra da recusa (o servidor já manda até 5). */
export const EXEMPLOS_MOSTRADOS = 5;

/**
 * Lê a recusa de sobrescrita de um erro. Decide pelo `code` e pelo status; devolve `null` para
 * qualquer outro erro. O formato do detalhe é conferido campo a campo, sem confiar nele.
 */
export function recusaDeSobrescrita(erro: unknown): RecusaSobrescrita | null {
    if (!(erro instanceof ErroApi)) return null;
    if (erro.status !== 409 || erro.codigo !== "importacao_sobrescreve") return null;

    const detalhes = erro.detalhes && typeof erro.detalhes === "object"
        ? (erro.detalhes as Record<string, unknown>)
        : {};
    const bruto = detalhes.tabelas && typeof detalhes.tabelas === "object"
        ? (detalhes.tabelas as Record<string, unknown>)
        : {};
    const tabelas: Record<string, number> = {};
    for (const [tabela, valor] of Object.entries(bruto)) {
        if (typeof valor === "number" && Number.isFinite(valor) && valor > 0) tabelas[tabela] = valor;
    }
    const somaDasTabelas = Object.values(tabelas).reduce((soma, valor) => soma + valor, 0);
    const linhas = numeroDoDetalhe(detalhes, "linhas") ?? somaDasTabelas;
    const exemplos = listaDoDetalhe(detalhes, "exemplos").slice(0, EXEMPLOS_MOSTRADOS);
    return { tabelas, linhas, exemplos };
}

/**
 * Texto de um exemplo de sobrescrita como o operador vê: "agents: bia" vira "Agentes: bia" (o nome
 * de operador da tabela quando ela é conhecida; o resto fica como veio, sem crase).
 */
export function textoDeExemplo(item: string): string {
    const limpo = item.replace(/`/g, "").trim();
    const indice = limpo.indexOf(":");
    if (indice <= 0) return limpo;
    const tabela = limpo.slice(0, indice).trim();
    const resto = limpo.slice(indice + 1).trim();
    if (!Object.prototype.hasOwnProperty.call(NOMES_DE_TABELA, tabela)) return limpo;
    return `${nomeDaTabela(tabela)}: ${resto}`;
}

/**
 * Traduz um erro de exportar ou importar em texto de operador. Decide pelo `code` e pelo status;
 * o texto da API não entra na tela (ele é técnico e pode ter caminho de arquivo).
 */
export function falhaDeBackup(erro: unknown, operacao: "exportar" | "importar"): FalhaDeBackup {
    if (!(erro instanceof ErroApi)) {
        return {
            titulo: translate(operacao === "exportar" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupGeneric"),
            podeTentarDeNovo: true,
        };
    }

    if (erro.semServidor) {
        return {
            titulo: translate(operacao === "exportar" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupOffline"),
            podeTentarDeNovo: true,
        };
    }

    if (erro.status === 401) {
        return {
            titulo: translate("orgs.backupSessionTitle"),
            texto: translate("orgs.backupSessionText"),
            podeTentarDeNovo: false,
        };
    }

    if (erro.status === 403) {
        return {
            titulo: translate(operacao === "exportar" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupForbiddenText"),
            podeTentarDeNovo: true,
        };
    }

    if (erro.status === 404) {
        return {
            titulo: translate("orgs.backupNotFoundTitle"),
            texto: translate("orgs.backupNotFoundText"),
            podeTentarDeNovo: false,
        };
    }

    if (erro.status === 413 || erro.codigo === "payload_too_large") {
        return {
            titulo: translate("orgs.backupTooLargeTitle"),
            texto: translate("orgs.backupTooLargeText"),
            podeTentarDeNovo: false,
        };
    }

    if (erro.codigo === "backup_incompleto") {
        return {
            titulo: translate("orgs.backupExportFailed"),
            texto: translate("orgs.backupIncompleteText"),
            podeTentarDeNovo: false,
        };
    }

    // A tela trata esta recusa à parte (com o botão de autorizar); aqui é só a salvaguarda.
    if (erro.status === 409 && erro.codigo === "importacao_sobrescreve") {
        return {
            titulo: translate("orgs.backupOverwriteTitle"),
            texto: translate("orgs.backupOverwriteText"),
            podeTentarDeNovo: false,
        };
    }

    if (erro.status === 409 && erro.codigo === "conflict") {
        const conflitos = listaDoDetalhe(erro.detalhes, "conflicts");
        const mais = numeroDoDetalhe(erro.detalhes, "conflicts_more") ?? 0;
        return {
            titulo: translate("orgs.backupConflictTitle"),
            texto: translate("orgs.backupConflictText"),
            conflitos: {
                itens: conflitos.slice(0, CONFLITOS_MOSTRADOS).map(textoDeConflito),
                mais: mais + Math.max(0, conflitos.length - CONFLITOS_MOSTRADOS),
            },
            podeTentarDeNovo: false,
        };
    }

    if (erro.status === 400 && erro.codigo === "invalid_backup") {
        const arquivo = numeroDoDetalhe(erro.detalhes, "version");
        const instalacao = numeroDoDetalhe(erro.detalhes, "supported_version");
        return {
            titulo: translate("orgs.backupInvalidTitle"),
            texto: translate("orgs.backupInvalidText"),
            versoes:
                arquivo !== undefined && instalacao !== undefined
                    ? { arquivo, instalacao }
                    : undefined,
            podeTentarDeNovo: false,
        };
    }

    if (erro.status >= 500) {
        return {
            titulo: translate(operacao === "exportar" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupServerError", { status: erro.status }),
            podeTentarDeNovo: true,
        };
    }

    return {
        titulo: translate(operacao === "exportar" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
        texto: translate("orgs.backupGeneric"),
        podeTentarDeNovo: true,
    };
}

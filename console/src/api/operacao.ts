/**
 * Operação da instância (HR-48a): saúde, métricas e consulta de manifesto de plugin.
 *
 * Só rotas que já existem no contrato publicado:
 * - `GET /healthz`: sem token. Responde `200` com o corpo, ou `503` com o mesmo corpo quando um
 *   banco não responde. O `503` também traz corpo, então a tela lê os dois casos.
 * - `GET /metrics`: sem token. Texto no formato Prometheus. A tela só lê os nomes que sabe
 *   explicar (`NOMES_DE_METRICA`); o resto do texto não é mostrado.
 * - `GET /admin/v1/orgs/{org}/manifests`: com token, lista os plugins declarados (HR-48b). Um item
 *   por tipo; plugin que não respondeu sai `indisponivel` sem derrubar a lista (`listarPlugins`).
 * - `GET /admin/v1/orgs/{org}/manifests/{kind}`: com token, o detalhe de um tipo, lido por
 *   `obterManifesto` (já existia em `integracoes.ts`).
 */

import { ErroApi, pedir, type Saude } from "./cliente";
import { obterManifesto, type ManifestoDoPlugin } from "./integracoes";

/** Nomes de métrica que a tela explica, com o texto que aparece ao lado de cada número. */
export const NOMES_DE_METRICA = {
    filaTurnos: "poph_fila_turnos",
    filaSaida: "poph_fila_saida",
    conversasAtivas: "poph_conversas_ativas",
    agentesCarregados: "poph_agentes_carregados",
    memoriaProcesso: "poph_memoria_processo_bytes",
    bancoPrincipal: "poph_harness_db_bytes",
    bancoBusca: "poph_rag_db_bytes",
    chamadasLlm: "poph_chamadas_llm_total",
    errosLlm: "poph_erros_llm_total",
    chamadasJev: "poph_chamadas_jev_total",
    errosJev: "poph_erros_jev_total",
    chamadasEvolution: "poph_chamadas_evolution_total",
    errosEvolution: "poph_erros_evolution_total",
    tokens: "poph_tokens_total",
} as const;

export type Medicoes = Partial<Record<keyof typeof NOMES_DE_METRICA, number>>;

export type SaudeDaInstancia = {
    /** `ok` quando o servidor respondeu 200; `degradada` quando respondeu 503 (banco fora). */
    situacao: "ok" | "degradada";
    saude: Saude;
};

async function buscarTexto(caminho: string, aceita: string): Promise<{ status: number; texto: string }> {
    let resposta: Response;
    try {
        resposta = await fetch(caminho, { headers: { accept: aceita } });
    } catch {
        throw new ErroApi(0, "rede", "não consegui falar com a instância: sem conexão com o servidor");
    }
    return { status: resposta.status, texto: await resposta.text() };
}

function lerJson(texto: string): unknown {
    if (!texto) return undefined;
    try {
        return JSON.parse(texto) as unknown;
    } catch {
        return undefined;
    }
}

/** Saúde do servidor. `200` = ok; `503` com corpo = servidor de pé, banco com problema. */
export async function consultarSaude(): Promise<SaudeDaInstancia> {
    const { status, texto } = await buscarTexto("/healthz", "application/json");
    const corpo = lerJson(texto) as Partial<Saude> | undefined;
    if (status === 200 || status === 503) {
        if (corpo && typeof corpo.version === "string") {
            return {
                situacao: status === 200 ? "ok" : "degradada",
                saude: corpo as Saude,
            };
        }
        throw new ErroApi(status, "healthz", `a instância respondeu ${status} sem o resumo de saúde`);
    }
    throw new ErroApi(status, "healthz", `a instância respondeu ${status} ao pedido de saúde`);
}

/**
 * Lê o texto do `/metrics` e devolve só os números que a tela sabe nomear. Linha de histograma
 * (com rótulo `{…}`) e comentário são ignorados; um valor que não é número finito também.
 */
export function interpretarMetricas(texto: string): Medicoes {
    const porNome = new Map<string, number>();
    for (const linha of texto.split("\n")) {
        const limpa = linha.trim();
        if (!limpa || limpa.startsWith("#")) continue;
        const partes = limpa.split(/\s+/);
        if (partes.length !== 2) continue;
        const nome = partes[0] ?? "";
        const valorTexto = partes[1] ?? "";
        if (nome.includes("{")) continue;
        const valor = Number(valorTexto);
        if (!Number.isFinite(valor)) continue;
        porNome.set(nome, valor);
    }
    const medicoes: Medicoes = {};
    for (const [chave, nome] of Object.entries(NOMES_DE_METRICA) as [keyof Medicoes, string][]) {
        const valor = porNome.get(nome);
        if (valor !== undefined) medicoes[chave] = valor;
    }
    return medicoes;
}

/** Métricas do processo atual. Os contadores zeram quando o servidor reinicia. */
export async function consultarMetricas(): Promise<Medicoes> {
    const { status, texto } = await buscarTexto("/metrics", "text/plain");
    if (status !== 200) {
        throw new ErroApi(status, "metrics", `a instância respondeu ${status} ao pedido de métricas`);
    }
    return interpretarMetricas(texto);
}

/** Um plugin declarado na instância, como a lista (`GET …/manifests`) devolve. */
export type ItemDePlugin = {
    kind: string;
    /** `ok`: manifesto lido (tem `nome` e `versao`). `indisponivel`: não respondeu (tem `mensagem`). */
    situacao: "ok" | "indisponivel";
    nome?: string;
    versao?: string;
    mensagem?: string;
};

/** Frase de reserva quando o servidor não manda a mensagem de um plugin indisponível. */
const MENSAGEM_INDISPONIVEL_PADRAO =
    "o plugin deste tipo não respondeu; verifique o programa declarado na configuração da instância";

/**
 * Lista os plugins declarados na instância. Item sem `kind` de texto é descartado (não há como
 * mostrá-lo nem consultá-lo). Situação que não seja `ok` vira `indisponivel`, para nada aparecer
 * como "no ar" por engano.
 */
export async function listarPlugins(org: string): Promise<ItemDePlugin[]> {
    const corpo = await pedir<{ items?: unknown } | undefined>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/manifests`,
    );
    const brutos: unknown[] = Array.isArray(corpo?.items) ? (corpo?.items as unknown[]) : [];
    const itens: ItemDePlugin[] = [];
    for (const bruto of brutos) {
        if (!bruto || typeof bruto !== "object") continue;
        const registro = bruto as Record<string, unknown>;
        if (typeof registro.kind !== "string" || !registro.kind) continue;
        if (registro.situacao === "ok") {
            itens.push({
                kind: registro.kind,
                situacao: "ok",
                nome: typeof registro.nome === "string" ? registro.nome : undefined,
                versao: typeof registro.versao === "string" ? registro.versao : undefined,
            });
        } else {
            itens.push({
                kind: registro.kind,
                situacao: "indisponivel",
                mensagem:
                    typeof registro.mensagem === "string" && registro.mensagem
                        ? registro.mensagem
                        : MENSAGEM_INDISPONIVEL_PADRAO,
            });
        }
    }
    return itens;
}

/** Consulta um tipo de plugin pelo nome (`kind`). Usa a rota de manifesto que já existia. */
export async function consultarPlugin(org: string, kind: string): Promise<ManifestoDoPlugin> {
    return obterManifesto(org, kind);
}

/** Quantos campos de configuração e de segredo o manifesto declara (para o resumo da tela). */
export function resumoDoManifesto(manifesto: ManifestoDoPlugin): {
    configuracao: number;
    segredos: number;
} {
    return {
        configuracao: Object.keys(manifesto.configuracao?.properties ?? {}).length,
        segredos: Object.keys(manifesto.segredos?.properties ?? {}).length,
    };
}

/*
 * Recarga do mapa de cluster (HR-48f): `POST /admin/v1/cluster/reload`, rota que já existia.
 * Relê o arquivo de cluster no processo vivo e troca o mapa em uso. Só token global (token de org
 * recebe 403). O corpo de sucesso chega como `unknown` no contrato, então os tipos abaixo são
 * escritos à mão a partir de `RelatoRecarga` do servidor e lidos com cuidado.
 */

/** Resumo de um mapa, sem segredo: o valor do segredo nunca sai, só se existe. */
export type ResumoDoMapa = {
    modo: "cluster" | "instancia_unica";
    /** Id desta instalação no mapa. Vazio quando não há. */
    instanciaLocal: string;
    /** Instância padrão declarada. `null` quando não há. */
    instanciaPadrao: string | null;
    segredoConfigurado: boolean;
    /** Instâncias declaradas, só id e se é a local. O endereço não vai para a tela. */
    instancias: { id: string; local: boolean }[];
    /** Quantas orgs o mapa roteia para alguma instância. */
    quantidadeDeOrgs: number;
};

/** Uma org que mudou de instância na recarga. `null` quando não tinha (ou deixou de ter) destino. */
export type TrocaDeOrg = {
    org: string;
    de: string | null;
    para: string | null;
};

/** Resultado de uma recarga que deu certo. */
export type RelatorioRecarga = {
    mudou: boolean;
    impressaoAntes: string;
    impressaoDepois: string;
    antes: ResumoDoMapa;
    depois: ResumoDoMapa;
    orgsQueTrocaram: TrocaDeOrg[];
    instanciasAdicionadas: string[];
    instanciasRemovidas: string[];
};

function comoObjeto(valor: unknown): Record<string, unknown> | null {
    return valor !== null && typeof valor === "object" && !Array.isArray(valor)
        ? (valor as Record<string, unknown>)
        : null;
}

function textoOuNulo(valor: unknown): string | null {
    return typeof valor === "string" && valor !== "" ? valor : null;
}

function listaDeTextos(valor: unknown): string[] {
    return Array.isArray(valor) ? valor.filter((item): item is string => typeof item === "string") : [];
}

function lerResumo(bruto: unknown): ResumoDoMapa {
    const registro = comoObjeto(bruto) ?? {};
    const instancias = Array.isArray(registro.instancias) ? registro.instancias : [];
    const orgs = comoObjeto(registro.orgs);
    return {
        modo: registro.modo === "cluster" ? "cluster" : "instancia_unica",
        instanciaLocal: typeof registro.instance_id === "string" ? registro.instance_id : "",
        instanciaPadrao: textoOuNulo(registro.default_instance),
        segredoConfigurado: registro.segredo_configurado === true,
        instancias: instancias
            .map((item) => comoObjeto(item))
            .filter((item): item is Record<string, unknown> => item !== null)
            .map((item) => ({
                id: typeof item.id === "string" ? item.id : "",
                local: item.local === true,
            })),
        quantidadeDeOrgs: orgs ? Object.keys(orgs).length : 0,
    };
}

/**
 * Lê o corpo de sucesso da recarga. Corpo sem o formato esperado vira erro de formato, para a tela
 * não mostrar um resultado inventado.
 */
export function interpretarRelatorio(corpo: unknown): RelatorioRecarga {
    const registro = comoObjeto(corpo);
    if (!registro || typeof registro.mudou !== "boolean" || !comoObjeto(registro.antes) || !comoObjeto(registro.depois)) {
        throw new ErroApi(200, "formato", "a instância respondeu a recarga sem o resumo esperado");
    }
    const trocas = Array.isArray(registro.orgs_que_trocaram) ? registro.orgs_que_trocaram : [];
    return {
        mudou: registro.mudou,
        impressaoAntes: typeof registro.fingerprint_antes === "string" ? registro.fingerprint_antes : "",
        impressaoDepois: typeof registro.fingerprint_depois === "string" ? registro.fingerprint_depois : "",
        antes: lerResumo(registro.antes),
        depois: lerResumo(registro.depois),
        orgsQueTrocaram: trocas
            .map((item) => comoObjeto(item))
            .filter((item): item is Record<string, unknown> => item !== null)
            .map((item) => ({
                org: typeof item.org === "string" ? item.org : "",
                de: textoOuNulo(item.de),
                para: textoOuNulo(item.para),
            })),
        instanciasAdicionadas: listaDeTextos(registro.instancias_adicionadas),
        instanciasRemovidas: listaDeTextos(registro.instancias_removidas),
    };
}

/**
 * Pede ao servidor que releia o arquivo de cluster e troque o mapa em uso. Ação da instalação
 * inteira: quem chama deve pedir confirmação antes. Erro sai como `ErroApi`; em 400 o `detalhes`
 * traz `arquivo` e `problemas`.
 */
export async function recarregarCluster(): Promise<RelatorioRecarga> {
    const corpo = await pedir<unknown>("/admin/v1/cluster/reload", { method: "POST" });
    return interpretarRelatorio(corpo);
}

/**
 * Painel do "por quê" de um turno (HR-47): lê o que o turno gravou e devolve só o que a tela mostra.
 *
 * A fonte é o próprio turno que a leitura do chat de teste já traz (`turns[]` em
 * `GET …/test-chat/{conversa}`); não há rota nova. Os campos chegam como JSON sem tipo no contrato
 * (`unknown`), então cada leitura aqui é defensiva: campo ausente ou com outro tipo vira `null` ou
 * lista vazia, nunca um número inventado.
 *
 * O que **não** é gravado hoje e por isso não aparece como dado: o texto da pergunta enviada ao
 * Jev, as probabilidades de cada opção, a estrutura completa do retorno de cada ferramenta (fica só o
 * texto que voltou ao modelo), a busca na base separada da montagem, a indicação de corte por trecho,
 * o link para a fonte oficial e o custo (`cost_micros` nunca é preenchido pelo turno). A tela declara
 * essas faltas.
 *
 * As chamadas de ferramenta vêm de `tool_calls` (HR-47b): estado executada ou recusada, motivo, duração
 * e os resumos de argumentos e de resultado, já prontos para exibir.
 */

import { DESCRICAO_GATE, type IdGate } from "./modeloJev";

/** Um gate decidido no turno, como o motor grava em `turns.jev_json`. */
export type DecisaoDoGate = {
    /** Id do gate (`g1` a `g8`), como vem do registro. */
    gate: string;
    /** Escolha que valeu no turno (inclui o padrão do gate quando o Jev não decidiu). */
    escolha: string | null;
    /** O que o Jev respondeu, mesmo quando a confiança não bastou para valer. */
    escolhaDoJev: string | null;
    /** `jev`, `cache` ou `fallback`. */
    origem: string | null;
    /** Confiança declarada, de 0 a 1. */
    confianca: number | null;
    /** Vantagem sobre a segunda opção, de 0 a 1. */
    margem: number | null;
    /** Por que caiu no padrão, quando caiu. */
    motivo: string | null;
    /** Tempo da chamada do lote que decidiu (o mesmo para os gates do mesmo lote). */
    ms: number | null;
    /** Tokens do lote que decidiu. */
    tokens: number | null;
};

/** Resumo da chamada ao Jev do turno (um lote por turno, no máximo). */
export type ChamadaDoJev = {
    perguntas: number | null;
    decididasPorCache: number | null;
    ms: number | null;
    tokens: number | null;
    entrada: number | null;
    saida: number | null;
    total: number | null;
    /** Motivo da falha do lote (prazo, erro do provedor), quando houve. */
    falha: string | null;
};

/** O que a revisão do G6 fez com a resposta. Só existe quando o G6 estava ligado e julgou. */
export type RevisaoG6 = {
    escolha: string | null;
    confianca: number | null;
    regenerou: boolean;
    segundaEscolha: string | null;
    respostaSegura: boolean;
};

export type JevDoTurno = {
    chamada: ChamadaDoJev | null;
    decisoes: DecisaoDoGate[];
    revisao: RevisaoG6 | null;
};

/** Trecho da base de conhecimento usado na resposta. A fonte é o item e o grupo. */
export type TrechoDoTurno = {
    item: string | null;
    titulo: string;
    grupo: string;
    texto: string;
    /** Cosseno entre a consulta e o trecho. */
    similaridade: number | null;
    /** Nota combinada por rank (RRF). Não é percentual. */
    relevancia: number | null;
};

/** Produto do catálogo usado na resposta, com o preço que o cliente viu. */
export type ProdutoDoTurno = {
    produto: string | null;
    nome: string;
    precoTexto: string | null;
    precoCentavos: number | null;
    unidade: string;
    condicoes: string;
};

export type RecuperadosDoTurno = {
    trechos: TrechoDoTurno[];
    produtos: ProdutoDoTurno[];
};

export type TemposDoTurno = {
    filaMs: number | null;
    montagemMs: number | null;
    primeiroTextoMs: number | null;
    totalMs: number | null;
    rodadasFerramenta: number | null;
    modelo: string | null;
    motivoFim: string | null;
};

type Objeto = Record<string, unknown>;

function ehObjeto(valor: unknown): valor is Objeto {
    return typeof valor === "object" && valor !== null && !Array.isArray(valor);
}

function numero(valor: unknown): number | null {
    return typeof valor === "number" && Number.isFinite(valor) ? valor : null;
}

function texto(valor: unknown): string | null {
    return typeof valor === "string" && valor.trim() !== "" ? valor : null;
}

function logico(valor: unknown): boolean {
    return valor === true;
}

function lista(valor: unknown): unknown[] {
    return Array.isArray(valor) ? valor : [];
}

/** Lê o registro de Jev do turno. `null` quando nenhum gate decidiu. */
export function lerJevDoTurno(valor: unknown): JevDoTurno | null {
    if (!ehObjeto(valor)) return null;

    const chamadaBruta = ehObjeto(valor.chamada) ? valor.chamada : null;
    const usage = chamadaBruta && ehObjeto(chamadaBruta.usage) ? chamadaBruta.usage : null;
    const chamada: ChamadaDoJev | null = chamadaBruta
        ? {
              perguntas: numero(chamadaBruta.perguntas),
              decididasPorCache: numero(chamadaBruta.decididas_por_cache),
              ms: numero(chamadaBruta.ms),
              tokens: numero(chamadaBruta.tokens),
              entrada: usage ? numero(usage.entrada) : null,
              saida: usage ? numero(usage.saida) : null,
              total: usage ? numero(usage.total) : null,
              falha: texto(chamadaBruta.falha),
          }
        : null;

    const decisoes: DecisaoDoGate[] = lista(valor.decisoes)
        .filter(ehObjeto)
        .map((item) => ({
            gate: texto(item.gate) ?? "desconhecido",
            escolha: texto(item.escolha),
            escolhaDoJev: texto(item.escolha_do_jev),
            origem: texto(item.origem),
            confianca: numero(item.confianca),
            margem: numero(item.margem),
            motivo: texto(item.motivo),
            ms: numero(item.ms),
            tokens: numero(item.tokens),
        }));

    const revisaoBruta = ehObjeto(valor.revisao) ? valor.revisao : null;
    const revisao: RevisaoG6 | null = revisaoBruta
        ? {
              escolha: texto(revisaoBruta.escolha),
              confianca: numero(revisaoBruta.confianca),
              regenerou: logico(revisaoBruta.regenerou),
              segundaEscolha: texto(revisaoBruta.segunda_escolha),
              respostaSegura: logico(revisaoBruta.resposta_segura),
          }
        : null;

    if (!chamada && decisoes.length === 0 && !revisao) return null;
    return { chamada, decisoes, revisao };
}

/** Lê a lista do que foi recuperado (trechos e produtos), na ordem em que o turno gravou. */
export function lerRecuperadosDoTurno(valor: unknown): RecuperadosDoTurno {
    const trechos: TrechoDoTurno[] = [];
    const produtos: ProdutoDoTurno[] = [];
    for (const item of lista(valor)) {
        if (!ehObjeto(item)) continue;
        if (item.tipo === "produto") {
            produtos.push({
                produto: texto(item.produto),
                nome: texto(item.nome) ?? "Produto sem nome",
                precoTexto: texto(item.preco_texto),
                precoCentavos: numero(item.preco_centavos),
                unidade: texto(item.unidade) ?? "",
                condicoes: texto(item.condicoes) ?? "",
            });
        } else if (item.tipo === "trecho" || texto(item.titulo) !== null) {
            trechos.push({
                item: texto(item.item),
                titulo: texto(item.titulo) ?? "Trecho sem título",
                grupo: texto(item.grupo) ?? "",
                texto: texto(item.texto) ?? "",
                similaridade: numero(item.similaridade),
                relevancia: numero(item.relevancia),
            });
        }
    }
    return { trechos, produtos };
}

/** Nomes das ferramentas chamadas, na ordem. O turno grava só o nome. */
export function lerFerramentasDoTurno(valor: unknown): string[] {
    return lista(valor).filter((item): item is string => typeof item === "string" && item !== "");
}

/**
 * Uma chamada de ferramenta como o turno a gravou (HR-47b). `rodada` é a do laço, começando em 0 (a
 * primeira resposta do modelo); a tela mostra a partir de 1.
 */
export type ChamadaDeFerramenta = {
    nome: string;
    rodada: number | null;
    ordem: number | null;
    /** `executada`, `recusada` ou `nao_registrado` quando o turno não trouxe o estado. */
    estado: "executada" | "recusada" | "nao_registrado";
    /** Motivo da recusa (`nao_habilitada`, …). Sempre `null` quando executada. */
    motivo: string | null;
    duracaoMs: number | null;
    /** Resumo dos argumentos, já pronto para exibir (JSON compacto, redigido e cortado pela API). */
    argumentos: string | null;
    /** Resumo do texto devolvido ao modelo (para recusa, é o erro que o modelo recebeu). */
    resultado: string | null;
};

/**
 * Lê `tool_calls` do turno, na ordem de rodada e de chamada. Turno gravado antes da HR-47b não tem o
 * campo: devolve lista vazia, e quem chama mostra só os nomes de `tools`.
 */
export function lerChamadasDeFerramenta(valor: unknown): ChamadaDeFerramenta[] {
    const chamadas: ChamadaDeFerramenta[] = [];
    for (const item of lista(valor)) {
        if (!ehObjeto(item)) continue;
        const nome = texto(item.nome);
        if (nome === null) continue;
        const estado =
            item.estado === "executada" || item.estado === "recusada" ? item.estado : "nao_registrado";
        chamadas.push({
            nome,
            rodada: numero(item.rodada),
            ordem: numero(item.ordem),
            estado,
            motivo: estado === "recusada" ? texto(item.motivo) : null,
            duracaoMs: numero(item.duracao_ms),
            argumentos: texto(item.argumentos),
            resultado: texto(item.resultado),
        });
    }
    return chamadas.sort((a, b) => (a.rodada ?? 0) - (b.rodada ?? 0) || (a.ordem ?? 0) - (b.ordem ?? 0));
}

/** Motivo da recusa de uma chamada, em português claro. Motivo desconhecido aparece como veio. */
export function rotuloDoMotivoRecusa(motivo: string | null): string {
    switch (motivo) {
        case "nao_habilitada":
            return "ferramenta não habilitada neste agente";
        case "desconhecida":
            return "ferramenta desconhecida (o nome não existe no sistema)";
        case "argumento_invalido":
            return "argumento inválido";
        case "erro_de_execucao":
            return "falha ao executar a ferramenta";
        case "limite_de_chamadas":
            return "passou do limite de chamadas de ferramenta nesta mensagem";
        case "fora_do_turno":
            return "fora do escopo deste turno";
        case null:
            return "motivo não registrado";
        default:
            return motivo;
    }
}

/** Duração de uma chamada. Zero é "menos de 1 ms" (o turno mediu e arredondou para baixo), não "não medido". */
export function formatarDuracaoDeFerramenta(ms: number | null): string {
    if (ms === null) return "não registrada";
    if (ms === 0) return "menos de 1 ms";
    return formatarMs(ms);
}

/**
 * Texto de argumentos ou de resultado de uma ferramenta, pronto para a tela (HR-49b).
 *
 * Se o texto é um objeto ou uma lista JSON válidos, sai indentado com 2 espaços. Senão (texto livre,
 * JSON cortado com "…[cortado]" ou quebrado), sai como veio. Só os espaços entre os itens mudam: cada
 * valor sai do mesmo jeito que foi gravado (`10.50` continua `10.50`). O `JSON.parse` serve só para
 * confirmar que é JSON; a saída não passa por ele. Número com 16 ou mais dígitos também sai como veio,
 * porque converter para número perderia precisão.
 */
export function formatarConteudoDeFerramenta(texto: string): string {
    if (!/^\s*[[{]/.test(texto) || /\d{16,}/.test(texto)) return texto;
    try {
        JSON.parse(texto);
    } catch {
        return texto;
    }
    const itens = tokensJson(texto);
    let saida = "";
    let nivel = 0;
    const novaLinha = () => `\n${"  ".repeat(nivel)}`;
    itens.forEach((item, indice) => {
        const anterior = indice > 0 ? itens[indice - 1] : undefined;
        if (item === "," || item === ":") {
            saida += item;
            return;
        }
        if (item === "}" || item === "]") {
            if (anterior === "{" || anterior === "[") {
                saida += item;
            } else {
                nivel -= 1;
                saida += `${novaLinha()}${item}`;
            }
            return;
        }
        if (anterior === ":") saida += " ";
        else if (anterior === ",") saida += novaLinha();
        else if (anterior === "{" || anterior === "[") {
            nivel += 1;
            saida += novaLinha();
        }
        saida += item;
    });
    return saida;
}

/** Divide um JSON em pedaços (textos entre aspas, pontuação e valores), sem os espaços entre eles. */
function tokensJson(texto: string): string[] {
    const itens: string[] = [];
    let i = 0;
    while (i < texto.length) {
        const caractere = texto[i] as string;
        if (/\s/.test(caractere)) {
            i += 1;
        } else if (caractere === '"') {
            let fim = i + 1;
            let escapado = false;
            while (fim < texto.length) {
                const atual = texto[fim];
                if (escapado) escapado = false;
                else if (atual === "\\") escapado = true;
                else if (atual === '"') break;
                fim += 1;
            }
            itens.push(texto.slice(i, fim + 1));
            i = fim + 1;
        } else if ("{}[],:".includes(caractere)) {
            itens.push(caractere);
            i += 1;
        } else {
            let fim = i;
            while (fim < texto.length && !/[\s"{}[\],:]/.test(texto[fim] as string)) fim += 1;
            itens.push(texto.slice(i, fim));
            i = fim;
        }
    }
    return itens;
}

/** Quantas vezes cada ferramenta foi chamada, na ordem da primeira chamada. */
export function contarFerramentas(nomes: string[]): Array<{ nome: string; vezes: number }> {
    const contagem = new Map<string, number>();
    for (const nome of nomes) contagem.set(nome, (contagem.get(nome) ?? 0) + 1);
    return [...contagem.entries()].map(([nome, vezes]) => ({ nome, vezes }));
}

/** Lê os tempos por etapa e o motivo do fim. `null` quando o turno não gravou nada disso. */
export function lerTemposDoTurno(valor: unknown): TemposDoTurno | null {
    if (!ehObjeto(valor)) return null;
    return {
        filaMs: numero(valor.fila_ms),
        montagemMs: numero(valor.montagem_ms),
        primeiroTextoMs: numero(valor.primeiro_texto_ms),
        totalMs: numero(valor.total_ms),
        rodadasFerramenta: numero(valor.rodadas_ferramenta),
        modelo: texto(valor.modelo),
        motivoFim: texto(valor.motivo_fim),
    };
}

/** Título do gate na tela: o nome que a aba Modelo e Jev já usa; o id cru se for desconhecido. */
export function tituloDoGate(gate: string): string {
    const chave = gate as IdGate;
    return DESCRICAO_GATE[chave]?.titulo ?? gate.toUpperCase();
}

/** Como o gate foi decidido, em linguagem comum. */
export function rotuloDaOrigem(origem: string | null): string {
    switch (origem) {
        case "jev":
            return "Respondida pelo Jev";
        case "cache":
            return "Reaproveitada do cache (sem chamada ao Jev)";
        case "fallback":
            return "Valeu o padrão do gate; a resposta do Jev não foi usada";
        default:
            return "não registrado";
    }
}

/** Motivo do fim do turno, em linguagem comum. */
export function rotuloDoMotivoFim(motivo: string | null): string {
    switch (motivo) {
        case "concluido":
            return "Resposta completa";
        case "chamadas_ferramenta":
            return "Parou para chamar ferramentas";
        case "limite_de_tokens":
            return "Parou no limite de tokens de saída";
        case null:
            return "não registrado";
        default:
            return motivo;
    }
}

/** Confiança de 0 a 1 como porcentagem inteira. */
export function formatarConfianca(valor: number | null): string {
    if (valor === null) return "não registrado";
    return `${Math.round(valor * 100)}%`;
}

/** Margem de 0 a 1 como porcentagem com uma casa. */
export function formatarMargem(valor: number | null): string {
    if (valor === null) return "não registrado";
    return `${(valor * 100).toFixed(1).replace(".", ",")}%`;
}

export function formatarMs(valor: number | null): string {
    if (valor === null) return "não registrado";
    return `${valor.toLocaleString("pt-BR")} ms`;
}

export function formatarTokens(valor: number | null): string {
    if (valor === null) return "não registrado";
    return `${valor.toLocaleString("pt-BR")} tokens`;
}

/**
 * Custo como a API manda (`cost_micros`), sem conversão. A API não diz a moeda, então a tela diz o
 * que é: micros da moeda que o modelo cobra.
 *
 * Zero não é custo medido: o turno ainda não calcula o custo, e a gravação usa 0 como padrão. Por
 * isso `null` e `0` dizem que o custo não foi calculado, em vez de mostrar um custo zero.
 */
export function formatarCustoMicros(valor: number | null): string {
    if (valor === null || valor === 0) return "não calculado neste turno (o turno ainda não grava o custo)";
    return `${valor.toLocaleString("pt-BR")} micros de moeda (a API não informa a moeda)`;
}

/** Preço em centavos para leitura, em reais. Sem valor, `null`. */
export function formatarPrecoCentavos(centavos: number | null): string | null {
    if (centavos === null) return null;
    return (centavos / 100).toLocaleString("pt-BR", { style: "currency", currency: "BRL" });
}

/**
 * Duração do turno: o total que o próprio turno mediu; sem ele, a diferença entre início e fim.
 * Turno ainda em andamento não tem fim, então não há duração.
 */
export function duracaoDoTurno(
    inicio: number,
    fim: number | null,
    tempos: TemposDoTurno | null,
): number | null {
    if (tempos && tempos.totalMs !== null) return tempos.totalMs;
    if (fim === null) return null;
    return Math.max(0, fim - inicio);
}

/** Rótulo do estado do turno, em português. */
export function rotuloDoEstadoDoTurno(estado: string): string {
    switch (estado) {
        case "done":
            return "Concluído";
        case "failed":
            return "Falhou";
        case "skipped":
            return "Não respondeu";
        case "running":
            return "Em andamento";
        default:
            return estado;
    }
}

/**
 * Frase de um turno que não deixou resposta do agente (falha, corte, "não respondeu" ou ainda em
 * andamento). Não diz que foi enviada resposta quando não foi.
 */
export function frasePeloEstadoSemResposta(estado: string): string {
    switch (estado) {
        case "failed":
            return "O turno falhou antes de a IA enviar uma resposta.";
        case "skipped":
            return "A IA não respondeu a esta mensagem.";
        case "done":
            return "O turno terminou sem texto de resposta.";
        case "running":
            return "O turno ainda está em andamento.";
        default:
            return "Nenhuma resposta foi enviada neste turno.";
    }
}

export type ItemDoRegistro<M, T> = { tipo: "mensagem"; mensagem: M } | { tipo: "turno"; turno: T };

/**
 * Monta o registro da conversa com os turnos que não deixaram mensagem de resposta (falha, corte,
 * "não respondeu"). Cada um entra antes da primeira mensagem que veio depois de ele começar; o que
 * sobra vai no fim. As mensagens mantêm a ordem em que chegaram.
 */
export function intercalarTurnos<M extends { created_at: number }, T extends { started_at: number }>(
    mensagens: M[],
    turnos: T[],
): ItemDoRegistro<M, T>[] {
    const ordenados = [...turnos].sort((a, b) => a.started_at - b.started_at);
    const saida: ItemDoRegistro<M, T>[] = [];
    let proximo = 0;
    for (const mensagem of mensagens) {
        let turno = ordenados[proximo];
        while (turno !== undefined && turno.started_at < mensagem.created_at) {
            saida.push({ tipo: "turno", turno });
            proximo += 1;
            turno = ordenados[proximo];
        }
        saida.push({ tipo: "mensagem", mensagem });
    }
    for (const turno of ordenados.slice(proximo)) {
        saida.push({ tipo: "turno", turno });
    }
    return saida;
}

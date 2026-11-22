/**
 * Testes da tela do módulo Matrix (MX-25), com o `fetch` falso.
 *
 * Cobrem: a faixa de indicadores (ligado, conexão nula, atraso, falhas, corte do backfill); os
 * avisos de Synapse fora e de módulo desligado; ligar e desligar com `If-Match` e `412` (relê e avisa);
 * botão travado durante a gravação; a tabela com a linha "Sem espaço" e as ações de cada linha;
 * criar espaço do departamento, ligar um existente pelo diálogo (cancelar não grava) e erro `409`
 * com a instrução da API; desligar espaço com confirmação; carregando, vazio e erro; `403` e `401`.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { ServidorFalso, erroApi, type Chamada } from "../testes/servidor";
import { ErroApi, definirToken, quandoExpirar } from "../api/cliente";
import { esperarFocoDoModal } from "../testes/modal";
import {
    Matrix,
    MENSAGEM_DE_CONFLITO,
    mensagemDoEspaco,
    montarLinhas,
    situacaoDaConexao,
    textoDaOrigem,
    valorDasFalhas,
    valorDoAtraso,
} from "./Matrix";

const ORG = "org_1";
const CAMINHO = `/admin/v1/orgs/${ORG}/matrix`;

const MODULO_DESLIGADO = {
    org_id: ORG,
    enabled: false,
    enabled_at: null,
    updated_at: 1700,
    connected: true,
    backlog: { messages: 0, oldest_seconds: 0 },
    pending_failures: 0,
    last_failure: null,
};

const MODULO_LIGADO = {
    org_id: ORG,
    enabled: true,
    enabled_at: 1_700_000_000_000,
    updated_at: 2400,
    connected: true,
    backlog: { messages: 3, oldest_seconds: 65 },
    pending_failures: 1,
    last_failure: "o WhatsApp recusou o envio",
};

const DEPARTAMENTOS = {
    items: [
        { id: "dep_1", org_id: ORG, name: "Vendas", status: "active", version: 1, updated_at: 1 },
        { id: "dep_2", org_id: ORG, name: "Suporte", status: "active", version: 1, updated_at: 1 },
        { id: "dep_3", org_id: ORG, name: "Antigo", status: "archived", version: 1, updated_at: 1 },
    ],
    next_cursor: null,
};

const ESPACO_SUPORTE = {
    alias: "#dep_dep2:matrix-dev.local",
    created_at: 1,
    department_id: "dep_2",
    department_name: "Suporte",
    origin: "auto",
    space_room_id: "!suporte:matrix-dev.local",
    updated_at: 1,
};

function montar(caminho = `/orgs/${ORG}/matrix`) {
    return render(
        <MemoryRouter initialEntries={[caminho]}>
            <Routes>
                <Route path="/orgs/:orgId/matrix" element={<Matrix />} />
            </Routes>
        </MemoryRouter>,
    );
}

/** Servidor com módulo, espaços e departamentos. Rotas específicas vêm antes das gerais. */
function servidorPadrao(
    opcoes: {
        modulo?: unknown;
        moduloStatus?: number;
        espacos?: unknown[];
        espacosStatus?: number;
        departamentos?: unknown;
    } = {},
) {
    return new ServidorFalso()
        .responder("GET", `${CAMINHO}/spaces`, {
            status: opcoes.espacosStatus ?? 200,
            corpo: opcoes.espacosStatus === 500
                ? { code: "interno", message: "falha" }
                : { items: opcoes.espacos ?? [] },
        })
        .responder("PUT", `${CAMINHO}/spaces`, {
            status: 200,
            corpo: { ...ESPACO_SUPORTE, department_id: "dep_1", space_room_id: "!novo:matrix-dev.local" },
        })
        .responder("DELETE", `${CAMINHO}/spaces/`, {
            status: 200,
            corpo: { department_id: "dep_2", removed: true, rooms_forgotten: 1 },
        })
        .responder("GET", "/departments", {
            status: 200,
            corpo: opcoes.departamentos ?? DEPARTAMENTOS,
        })
        .responder("GET", CAMINHO, {
            status: opcoes.moduloStatus ?? 200,
            corpo: opcoes.moduloStatus && opcoes.moduloStatus !== 200
                ? { code: "interno", message: "falha" }
                : opcoes.modulo ?? MODULO_DESLIGADO,
        })
        .responder("PUT", CAMINHO, (chamada) => ({
            status: 200,
            corpo: { ...MODULO_LIGADO, enabled: (chamada.corpo as { enabled: boolean }).enabled },
        }))
        .instalar();
}

/** Primeira chamada da lista, ou falha com a mensagem do que não foi chamado. */
function primeira(chamadas: Chamada[]): Chamada {
    const [primeiraChamada] = chamadas;
    if (!primeiraChamada) throw new Error("a chamada esperada não aconteceu");
    return primeiraChamada;
}

/** Linha da tabela do departamento, pelo nome (cada linha é um `role="row"`). */
function linhaDo(nome: string): HTMLElement {
    const tabela = screen.getByRole("table", { name: "Espaços por departamento" });
    const linha = within(tabela)
        .getAllByRole("row")
        .find((celula) => within(celula).queryByText(nome, { exact: true }) !== null);
    if (!linha) throw new Error(`sem linha para ${nome}`);
    return linha;
}

/** Diálogo aberto na tela, ou falha com a mensagem do que não apareceu. */
async function dialogoAberto(): Promise<HTMLElement> {
    return waitFor(() => {
        const d = document.querySelector('[role="dialog"]');
        if (!d) throw new Error("sem diálogo");
        return d as HTMLElement;
    });
}

function secao(titulo: string): HTMLElement {
    const cabecalho = screen.getByRole("heading", { level: 2, name: titulo });
    const bloco = cabecalho.closest("section");
    if (!bloco) throw new Error(`seção sem bloco: ${titulo}`);
    return bloco;
}

afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    definirToken(null);
    quandoExpirar(null);
});

describe("textos da tela", () => {
    it("conexão: conectado, desconectado e desconhecido quando vem nulo", () => {
        expect(situacaoDaConexao(true)).toEqual({ valor: "Conectado" });
        expect(situacaoDaConexao(false).valor).toBe("Desconectado");
        expect(situacaoDaConexao(null).valor).toBe("Desconhecido");
        expect(situacaoDaConexao(undefined).valor).toBe("Desconhecido");
    });

    it("atraso e falhas em abreviação de painel", () => {
        expect(valorDoAtraso(0)).toBe("0 msgs atrás");
        expect(valorDoAtraso(1)).toBe("1 msg atrás");
        expect(valorDoAtraso(142)).toBe("142 msgs atrás");
        expect(valorDasFalhas(0)).toBe("0 pendentes");
        expect(valorDasFalhas(1)).toBe("1 pendente");
        expect(textoDaOrigem("auto")).toBe("Automático (módulo)");
        expect(textoDaOrigem("linked")).toBe("Vinculado (Element)");
    });

    it("erro 409 e 400 mostram a instrução da API; o resto passa pela tradução comum", () => {
        expect(mensagemDoEspaco(new ErroApi(409, "x", "convide o bot e dê poder 50"), "o espaço")).toBe(
            "convide o bot e dê poder 50",
        );
        expect(mensagemDoEspaco(new ErroApi(404, "x", "cru"), "o departamento")).toBe(
            "Não encontrei o departamento.",
        );
    });

    it("linhas: todo departamento ativo aparece; espaço de departamento arquivado não some", () => {
        const ativos = DEPARTAMENTOS.items as Parameters<typeof montarLinhas>[0];
        const linhas = montarLinhas(ativos, [ESPACO_SUPORTE]);
        expect(linhas.map((linha) => [linha.nome, linha.espaco !== null])).toEqual([
            ["Vendas", false],
            ["Suporte", true],
        ]);
        const orfao = montarLinhas(ativos, [{ ...ESPACO_SUPORTE, department_id: "dep_3", department_name: "Antigo" }]);
        expect(orfao.find((linha) => linha.nome === "Antigo")?.arquivado).toBe(true);
    });
});

describe("módulo nesta organização", () => {
    it("faixa de indicadores: ligado, corte, Synapse, atraso, falhas e última falha, sem dado de conversa", async () => {
        servidorPadrao({ modulo: MODULO_LIGADO });

        montar();

        const bloco = await waitFor(() => {
            const b = secao("Módulo nesta organização");
            expect(within(b).getByText("Ligado")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByText("Conectado")).toBeInTheDocument();
        expect(within(bloco).getByText("3 msgs atrás")).toBeInTheDocument();
        expect(within(bloco).getByText("a mais antiga há 1 min")).toBeInTheDocument();
        expect(within(bloco).getByText("1 pendente")).toBeInTheDocument();
        expect(within(bloco).getByText("o WhatsApp recusou o envio")).toBeInTheDocument();
        expect(within(bloco).getByText(/ativo · desde/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Desligar módulo" })).toBeInTheDocument();
    });

    it("conexão nula aparece como desconhecido e módulo nunca ligado diz isso", async () => {
        servidorPadrao({ modulo: { ...MODULO_DESLIGADO, connected: null } });

        montar();

        const bloco = await waitFor(() => {
            const b = secao("Módulo nesta organização");
            expect(within(b).getByText("Desconhecido")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByText("não configurado nesta instância")).toBeInTheDocument();
        expect(within(bloco).getByText("pausado · nunca ligado")).toBeInTheDocument();
        expect(within(bloco).getByText("—")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Ligar módulo Matrix" })).toBeInTheDocument();
    });

    it("módulo desligado: diz que ligar não traz o histórico anterior", async () => {
        servidorPadrao({ modulo: MODULO_DESLIGADO });

        montar();

        expect(await screen.findByText("Módulo Matrix desativado para esta organização")).toBeInTheDocument();
        expect(screen.getByText(/o histórico anterior não é trazido/)).toBeInTheDocument();
    });

    it("Synapse fora com módulo ligado: alerta de espelhamento parado e 'Tentar reconectar'", async () => {
        const servidor = servidorPadrao({ modulo: { ...MODULO_LIGADO, connected: false } });

        montar();

        expect(
            await screen.findByText(/o envio para o Element está acumulando fila/),
        ).toBeInTheDocument();
        expect(screen.getByText(/continuam operando normalmente, sem perda de mensagens/)).toBeInTheDocument();
        const antes = servidor.chamadasPara("GET", CAMINHO).filter((c) => !c.url.includes("/spaces")).length;
        await userEvent.click(screen.getByRole("button", { name: "Tentar reconectar" }));
        await waitFor(() =>
            expect(
                servidor.chamadasPara("GET", CAMINHO).filter((c) => !c.url.includes("/spaces")).length,
            ).toBeGreaterThan(antes),
        );
    });

    it("carregando: mostra o esqueleto antes de chegar a resposta", async () => {
        vi.stubGlobal("fetch", vi.fn(() => new Promise(() => {})));

        montar();

        const bloco = secao("Módulo nesta organização");
        expect(bloco.querySelector(".sofa-skeleton")).not.toBeNull();
        expect(within(bloco).queryByText("Ligado")).toBeNull();
        expect(within(bloco).queryByText("Desligado")).toBeNull();
    });

    it("erro de leitura mostra 'Tentar de novo' e refaz a consulta", async () => {
        const servidor = servidorPadrao({ moduloStatus: 500 });

        montar();

        const bloco = await waitFor(() => {
            const b = secao("Módulo nesta organização");
            expect(within(b).getByText("Não consegui ler o estado do módulo")).toBeInTheDocument();
            return b;
        });
        const antes = servidor.chamadasPara("GET", CAMINHO).filter((c) => !c.url.includes("/spaces")).length;
        await userEvent.click(within(bloco).getByRole("button", { name: "Tentar de novo" }));

        await waitFor(() =>
            expect(
                servidor.chamadasPara("GET", CAMINHO).filter((c) => !c.url.includes("/spaces")).length,
            ).toBeGreaterThan(antes),
        );
    });

    it("erro 403 na leitura diz que o token não alcança a organização", async () => {
        new ServidorFalso()
            .responder("GET", `${CAMINHO}/spaces`, { status: 200, corpo: { items: [] } })
            .responder("GET", "/departments", { status: 200, corpo: DEPARTAMENTOS })
            .responder("GET", CAMINHO, erroApi(403, "forbidden", "token de outra org"))
            .instalar();

        montar();

        expect(
            await screen.findByText(/não alcança esta organização/),
        ).toBeInTheDocument();
    });

    it("ligar envia o If-Match com a versão lida e mostra o módulo ligado", async () => {
        const servidor = servidorPadrao({ modulo: MODULO_DESLIGADO });
        montar();

        await userEvent.click(await screen.findByRole("button", { name: "Ligar módulo Matrix" }));

        await waitFor(() => expect(servidor.chamadasPara("PUT", CAMINHO)).toHaveLength(1));
        const chamada = primeira(servidor.chamadasPara("PUT", CAMINHO));
        expect(chamada.cabecalhos["if-match"]).toBe("1700");
        expect(chamada.corpo).toEqual({ enabled: true });
        expect(await screen.findByText("Módulo ligado. Só o que chegar daqui em diante é espelhado.")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Desligar módulo" })).toBeInTheDocument();
    });

    it("desligar pede confirmação e só envia depois dela, com o If-Match da leitura", async () => {
        const servidor = servidorPadrao({ modulo: MODULO_LIGADO });
        montar();

        await userEvent.click(await screen.findByRole("button", { name: "Desligar módulo" }));
        const dialogo = await dialogoAberto();
        await esperarFocoDoModal();
        expect(servidor.chamadasPara("PUT", CAMINHO)).toHaveLength(0);

        await userEvent.click(within(dialogo).getByRole("button", { name: "Desligar módulo" }));

        await waitFor(() => expect(servidor.chamadasPara("PUT", CAMINHO)).toHaveLength(1));
        const chamada = primeira(servidor.chamadasPara("PUT", CAMINHO));
        expect(chamada.cabecalhos["if-match"]).toBe("2400");
        expect(chamada.corpo).toEqual({ enabled: false });
    });

    it("412 mostra a frase de conflito e relê o estado", async () => {
        const servidor = new ServidorFalso()
            .responder("GET", `${CAMINHO}/spaces`, { status: 200, corpo: { items: [] } })
            .responder("GET", "/departments", { status: 200, corpo: DEPARTAMENTOS })
            .responder("PUT", CAMINHO, erroApi(412, "version_conflict", "mudou"))
            .responder("GET", CAMINHO, { status: 200, corpo: MODULO_DESLIGADO })
            .instalar();
        montar();

        await userEvent.click(await screen.findByRole("button", { name: "Ligar módulo Matrix" }));

        expect(await screen.findByText(MENSAGEM_DE_CONFLITO)).toBeInTheDocument();
        await waitFor(() =>
            expect(
                servidor.chamadasPara("GET", CAMINHO).filter((c) => !c.url.includes("/spaces")).length,
            ).toBeGreaterThanOrEqual(2),
        );
    });

    it("botão fica travado enquanto a gravação não volta", async () => {
        let liberar: () => void = () => {};
        const pendente = new Promise<void>((resolver) => {
            liberar = resolver;
        });
        vi.stubGlobal(
            "fetch",
            vi.fn(async (entrada: RequestInfo | URL, init?: RequestInit) => {
                const url = typeof entrada === "string" ? entrada : entrada instanceof URL ? entrada.toString() : entrada.url;
                const metodo = (init?.method ?? "GET").toUpperCase();
                if (metodo === "PUT" && url.endsWith(CAMINHO)) {
                    await pendente;
                    return { status: 200, ok: true, headers: new Headers(), text: async () => JSON.stringify(MODULO_LIGADO) } as Response;
                }
                const corpo = url.includes("/spaces")
                    ? { items: [] }
                    : url.includes("/departments")
                      ? DEPARTAMENTOS
                      : MODULO_DESLIGADO;
                return { status: 200, ok: true, headers: new Headers(), text: async () => JSON.stringify(corpo) } as Response;
            }),
        );
        montar();

        const botao = await screen.findByRole("button", { name: "Ligar módulo Matrix" });
        await userEvent.click(botao);

        expect(await screen.findByRole("button", { name: "Ligar módulo Matrix" })).toBeDisabled();
        await act(async () => {
            liberar();
        });
        expect(await screen.findByRole("button", { name: "Desligar módulo" })).toBeInTheDocument();
    });

    it("401 numa sessão aberta dispara o encerramento da sessão", async () => {
        const encerrar = vi.fn();
        definirToken("poph_token_da_sessao");
        quandoExpirar(encerrar);
        new ServidorFalso()
            .responder("GET", `${CAMINHO}/spaces`, { status: 200, corpo: { items: [] } })
            .responder("GET", "/departments", { status: 200, corpo: DEPARTAMENTOS })
            .responder("GET", CAMINHO, erroApi(401, "invalid_token", "sessão caiu"))
            .instalar();

        montar();

        await waitFor(() => expect(encerrar).toHaveBeenCalled());
    });
});

describe("espaços por departamento", () => {
    it("sem espaço: cada departamento ativo vira linha 'Sem espaço' com as duas ações", async () => {
        servidorPadrao({ espacos: [] });

        montar();

        const tabela = await screen.findByRole("table", { name: "Espaços por departamento" });
        expect(within(tabela).getAllByText("Nenhum espaço vinculado")).toHaveLength(2);
        expect(within(tabela).getAllByText("Sem espaço")).toHaveLength(2);
        expect(within(tabela).getAllByRole("button", { name: /Criar espaço automático/ })).toHaveLength(2);
        expect(within(tabela).getAllByRole("button", { name: /Ligar espaço existente/ })).toHaveLength(2);
        expect(within(tabela).queryByText("Antigo")).toBeNull();
    });

    it("sem nenhum departamento ativo mostra o vazio", async () => {
        servidorPadrao({ espacos: [], departamentos: { items: [], next_cursor: null } });

        montar();

        expect(await screen.findByText("Nenhum departamento ativo")).toBeInTheDocument();
    });

    it("linha com espaço mostra alias, origem, status e o botão de desligar", async () => {
        servidorPadrao({ espacos: [ESPACO_SUPORTE] });

        montar();

        const linha = await waitFor(() => linhaDo("Suporte"));
        expect(within(linha).getByText("#dep_dep2:matrix-dev.local")).toBeInTheDocument();
        expect(within(linha).getByText("Automático (módulo)")).toBeInTheDocument();
        expect(within(linha).getByText("Ativo")).toBeInTheDocument();
        expect(within(linha).getByRole("button", { name: "Desligar espaço do departamento Suporte" })).toBeInTheDocument();
        expect(within(linhaDo("Vendas")).getByText("Sem espaço")).toBeInTheDocument();
    });

    it("regra de operação fixa: desligar só remove o vínculo; sala e histórico ficam no Synapse", async () => {
        servidorPadrao({ espacos: [ESPACO_SUPORTE] });

        montar();

        expect(await screen.findByText("Regra de operação")).toBeInTheDocument();
        expect(
            screen.getByText(/remove apenas o vínculo do departamento no Harness/),
        ).toBeInTheDocument();
    });

    it("erro ao listar mostra 'Tentar de novo' no lugar da tabela", async () => {
        servidorPadrao({ espacosStatus: 500 });

        montar();

        const bloco = await waitFor(() => {
            const b = secao("Espaços por departamento");
            expect(within(b).getByText("Não consegui ler os espaços")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
    });

    it("'Criar auto' envia só o departamento (espaço automático) e cria uma vez", async () => {
        const servidor = servidorPadrao({ espacos: [] });
        montar();

        await userEvent.click(
            await screen.findByRole("button", { name: "Criar espaço automático do departamento Vendas" }),
        );

        await waitFor(() => expect(servidor.chamadasPara("PUT", `${CAMINHO}/spaces`)).toHaveLength(1));
        const chamada = primeira(servidor.chamadasPara("PUT", `${CAMINHO}/spaces`));
        expect(chamada.corpo).toEqual({ department_id: "dep_1" });
        expect(await screen.findByText("Espaço do departamento criado.")).toBeInTheDocument();
    });

    it("'Ligar existente' abre o diálogo; o id vai no PUT e nada cria segundo espaço", async () => {
        const servidor = servidorPadrao({ espacos: [] });
        montar();

        await userEvent.click(
            await screen.findByRole("button", { name: "Ligar espaço existente ao departamento Vendas" }),
        );
        const dialogo = await dialogoAberto();
        expect(within(dialogo).getByText("Vendas (dep_1)")).toBeInTheDocument();
        await userEvent.type(
            within(dialogo).getByRole("textbox", { name: /Alias ou room ID/ }),
            "!abc:matrix-dev.local",
        );
        await userEvent.click(within(dialogo).getByRole("button", { name: "Ligar espaço" }));

        await waitFor(() => expect(servidor.chamadasPara("PUT", `${CAMINHO}/spaces`)).toHaveLength(1));
        expect(primeira(servidor.chamadasPara("PUT", `${CAMINHO}/spaces`)).corpo).toEqual({
            department_id: "dep_1",
            space: "!abc:matrix-dev.local",
        });
        expect(await screen.findByText("Espaço existente ligado ao departamento.")).toBeInTheDocument();
    });

    it("cancelar o diálogo não grava nada", async () => {
        const servidor = servidorPadrao({ espacos: [] });
        montar();

        await userEvent.click(
            await screen.findByRole("button", { name: "Ligar espaço existente ao departamento Vendas" }),
        );
        const dialogo = await dialogoAberto();
        await userEvent.type(
            within(dialogo).getByRole("textbox", { name: /Alias ou room ID/ }),
            "#vendas:matrix-dev.local",
        );
        await userEvent.click(within(dialogo).getByRole("button", { name: "Cancelar" }));

        await waitFor(() => expect(document.querySelector('[role="dialog"]')).toBeNull());
        expect(servidor.chamadasPara("PUT", `${CAMINHO}/spaces`)).toHaveLength(0);
    });

    it("'Ligar espaço' fica desabilitado enquanto o campo está vazio (vazio criaria o automático)", async () => {
        servidorPadrao({ espacos: [] });
        montar();

        await userEvent.click(
            await screen.findByRole("button", { name: "Ligar espaço existente ao departamento Vendas" }),
        );
        const dialogo = await dialogoAberto();
        expect(within(dialogo).getByRole("button", { name: "Ligar espaço" })).toBeDisabled();
    });

    it("409 do servidor aparece dentro do diálogo com a instrução do que fazer no Element", async () => {
        new ServidorFalso()
            .responder("GET", `${CAMINHO}/spaces`, { status: 200, corpo: { items: [] } })
            .responder("GET", "/departments", { status: 200, corpo: DEPARTAMENTOS })
            .responder("PUT", `${CAMINHO}/spaces`, erroApi(409, "conflito", "convide o bot do módulo e dê a ele poder 50"))
            .instalar();
        montar();

        await userEvent.click(
            await screen.findByRole("button", { name: "Ligar espaço existente ao departamento Suporte" }),
        );
        const dialogo = await dialogoAberto();
        await userEvent.type(
            within(dialogo).getByRole("textbox", { name: /Alias ou room ID/ }),
            "#suporte:matrix-dev.local",
        );
        await userEvent.click(within(dialogo).getByRole("button", { name: "Ligar espaço" }));

        expect(
            await within(dialogo).findByText("convide o bot do módulo e dê a ele poder 50"),
        ).toBeInTheDocument();
    });

    it("desligar pede confirmação, chama o DELETE do departamento e diz que nada foi apagado", async () => {
        const servidor = servidorPadrao({ espacos: [ESPACO_SUPORTE] });
        montar();

        await userEvent.click(
            await screen.findByRole("button", { name: "Desligar espaço do departamento Suporte" }),
        );
        const dialogo = await dialogoAberto();
        await esperarFocoDoModal();
        expect(servidor.chamadasPara("DELETE", `${CAMINHO}/spaces/dep_2`)).toHaveLength(0);
        expect(within(dialogo).getByText(/Nada é apagado no Element/)).toBeInTheDocument();

        await userEvent.click(within(dialogo).getByRole("button", { name: "Desligar espaço" }));

        await waitFor(() => expect(servidor.chamadasPara("DELETE", `${CAMINHO}/spaces/dep_2`)).toHaveLength(1));
        expect(await screen.findByText("Espaço desligado. Nada foi apagado no Element.")).toBeInTheDocument();
    });

    it("espaço de departamento arquivado aparece marcado, sem botão de criar", async () => {
        servidorPadrao({
            espacos: [{ ...ESPACO_SUPORTE, department_id: "dep_3", department_name: "Antigo" }],
            departamentos: { items: [DEPARTAMENTOS.items[1]], next_cursor: null },
        });

        montar();

        const linha = await waitFor(() => linhaDo("Antigo"));
        expect(within(linha).getByText("Departamento arquivado")).toBeInTheDocument();
        expect(within(linha).queryByRole("button", { name: /Criar espaço automático/ })).toBeNull();
    });
});

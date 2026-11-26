/**
 * Testes da tela de operação (HR-48a, HR-48c), com o `fetch` falso.
 *
 * Cobrem: saúde OK e saúde degradada (503 com corpo); erro de uma seção sem derrubar a outra, com
 * "Tentar de novo"; métricas com e sem valores; lista de plugins declarados (no ar, indisponível,
 * vazia, carregando, erro 403 com "Tentar de novo"); detalhe aberto pela linha da lista (achado e
 * 503); botão Atualizar; atualização automática a cada 15 s; formatação de tempo, memória e versão.
 */

import { afterEach, describe, expect, it, vi, type Mock } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { FakeServer, apiError } from "../testing/server";
import {
    Operation,
    formatarMemoria,
    formatarTempoNoAr,
    formatarVersaoDoBanco,
    INTERVALO_DE_ATUALIZACAO_MS,
    mensagemDeFalha,
} from "./Operacao";
import { ApiError } from "../api/client";
import { setLocale } from "../i18n";
import { waitForModalFocus } from "../testing/modal";

const SAUDE_OK = {
    status: "ok",
    version: "0.9.1",
    uptime_seconds: 90_000,
    databases: { harness: 12, rag: 3 },
};

const METRICAS = [
    "# HELP poph_fila_turnos Turnos esperando na fila.",
    "poph_fila_turnos 4",
    "poph_fila_saida 0",
    "poph_conversas_ativas 2",
    "poph_agentes_carregados 1",
    "poph_memoria_processo_bytes 67108864",
    "poph_harness_db_bytes 2097152",
    "poph_rag_db_bytes 1048576",
    "poph_chamadas_llm_total 15",
    "poph_erros_llm_total 1",
    "poph_chamadas_jev_total 9",
    "poph_erros_jev_total 0",
    "poph_chamadas_evolution_total 0",
    "poph_erros_evolution_total 0",
    "poph_tokens_total 4800",
    "",
].join("\n");

const LISTA_DOIS_PLUGINS = {
    items: [
        { kind: "eco", situacao: "ok", nome: "Plugin Eco", versao: "0.1.0" },
        {
            kind: "quebrado",
            situacao: "indisponivel",
            mensagem:
                "o plugin deste tipo não respondeu ao handshake; verifique o programa declarado em `[plugins].integracoes`",
        },
    ],
};

const MANIFESTO_ECO = {
    nome: "Plugin Eco",
    kind: "eco",
    versao: "0.1.0",
    configuracao: { type: "object", properties: { remetente: {}, modo: {} } },
    segredos: { type: "object", properties: { token: {} } },
};

function montar(caminho = "/orgs/org_1/operacao") {
    return render(
        <MemoryRouter initialEntries={[caminho]}>
            <Routes>
                <Route path="/orgs/:orgId/operacao" element={<Operation />} />
            </Routes>
        </MemoryRouter>,
    );
}

function secao(titulo: string): HTMLElement {
    const cabecalho = screen.getByRole("heading", { level: 2, name: titulo });
    const bloco = cabecalho.closest("section");
    if (!bloco) throw new Error(`seção sem bloco: ${titulo}`);
    return bloco;
}

/**
 * Servidor com saúde e métricas. A lista de plugins vai por último: a rota casa por trecho, então
 * as rotas de detalhe (`/manifests/{kind}`) têm de ser declaradas antes dela.
 */
function servidorPadrao(
    saude: { status: number; corpo?: unknown } = { status: 200, corpo: SAUDE_OK },
    lista: { status: number; corpo?: unknown } = { status: 200, corpo: { items: [] } },
) {
    return new FakeServer()
        .responder("GET", "/healthz", saude)
        .responder("GET", "/metrics", { status: 200, texto: METRICAS })
        .responder("GET", "/manifests", lista)
        .instalar();
}

describe("formatação", () => {
    it("tempo no ar em linguagem de operador", () => {
        expect(formatarTempoNoAr(30)).toBe("menos de 1 min");
        expect(formatarTempoNoAr(300)).toBe("5 min");
        expect(formatarTempoNoAr(3_725)).toBe("1 h e 2 min");
        expect(formatarTempoNoAr(90_000)).toBe("1 dia e 1 h");
        expect(formatarTempoNoAr(2 * 86_400 + 7_200)).toBe("2 dias e 2 h");
    });

    it("memória em KB, MB ou GB com vírgula brasileira", () => {
        expect(formatarMemoria(512 * 1024)).toBe("512 KB");
        expect(formatarMemoria(1_572_864)).toBe("1,5 MB");
        expect(formatarMemoria(2 * 1024 ** 3)).toBe("2 GB");
    });

    it("versão de banco zero quer dizer que não foi possível ler", () => {
        expect(formatarVersaoDoBanco(0)).toBe("sem versão gravada ainda");
        expect(formatarVersaoDoBanco(12)).toBe("esquema 12");
    });

    it("mensagens de erro em português, sem corpo cru", () => {
        expect(mensagemDeFalha(new ApiError(403, "x", "cru"), "o plugin")).toMatch(/não alcança esta organização/);
        expect(mensagemDeFalha(new ApiError(404, "x", "cru"), "o plugin")).toBe("Não encontrei o plugin.");
        expect(mensagemDeFalha(new ApiError(500, "x", "cru"), "o plugin")).toMatch(/respondeu com erro \(500\)/);
        expect(mensagemDeFalha(new ApiError(0, "rede", "cru"), "o plugin")).toMatch(/sem conexão|Confira se/);
    });
});

describe("Operacao: saúde e métricas", () => {
    it("mostra o servidor no ar, o tempo no ar, as filas e os recursos", async () => {
        servidorPadrao();

        montar();

        expect(await screen.findByRole("heading", { level: 1, name: "Saúde da instância" })).toBeInTheDocument();
        const servidor = await waitFor(() => {
            const bloco = secao("Servidor");
            expect(within(bloco).getByText("No ar, com os bancos respondendo")).toBeInTheDocument();
            return bloco;
        });
        expect(within(servidor).getByText("0.9.1")).toBeInTheDocument();
        expect(within(servidor).getByText("1 dia e 1 h")).toBeInTheDocument();
        expect(within(servidor).getByText("esquema 12")).toBeInTheDocument();

        const filas = secao("Filas e recursos");
        expect(within(filas).getByText("Turnos esperando na fila")).toBeInTheDocument();
        expect(within(filas).getByText("4")).toBeInTheDocument();
        expect(within(filas).getByText("64 MB")).toBeInTheDocument();
        expect(within(filas).getByText("15 · com erro: 1")).toBeInTheDocument();
        expect(within(filas).getByText("4.800")).toBeInTheDocument();
    });

    it("saúde 503 com corpo aparece como no ar, com um banco sem resposta", async () => {
        servidorPadrao({ status: 503, corpo: { ...SAUDE_OK, status: "degraded" } });

        montar();

        expect(await screen.findByText("No ar, mas um dos bancos não respondeu")).toBeInTheDocument();
        expect(within(secao("Servidor")).getByText("0.9.1")).toBeInTheDocument();
    });

    it("erro no servidor mostra 'Tentar de novo' e não derruba as métricas", async () => {
        servidorPadrao({ status: 500, corpo: { code: "interno", message: "falha" } });

        montar();

        const bloco = await waitFor(() => {
            const b = secao("Servidor");
            expect(within(b).getByText("Não consegui ler o estado do servidor")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
        // As métricas seguem na tela, independentes da saúde.
        expect(within(secao("Filas e recursos")).getByText("Turnos esperando na fila")).toBeInTheDocument();
    });

    it("'Tentar de novo' refaz a consulta da seção com erro", async () => {
        const servidor = new FakeServer()
            .responder("GET", "/healthz", apiError(500, "interno", "falha"))
            .responder("GET", "/metrics", { status: 200, texto: METRICAS })
            .responder("GET", "/manifests", { status: 200, corpo: { items: [] } })
            .instalar();
        montar();

        const bloco = await waitFor(() => {
            const b = secao("Servidor");
            expect(within(b).getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
            return b;
        });
        const antes = servidor.chamadasPara("GET", "/healthz").length;
        await userEvent.click(within(bloco).getByRole("button", { name: "Tentar de novo" }));

        await waitFor(() => expect(servidor.chamadasPara("GET", "/healthz").length).toBeGreaterThan(antes));
    });

    it("métricas sem nenhum valor conhecido mostram 'não informado', sem zero inventado", async () => {
        new FakeServer()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK })
            .responder("GET", "/metrics", { status: 200, texto: "# só comentário\n" })
            .responder("GET", "/manifests", { status: 200, corpo: { items: [] } })
            .instalar();

        montar();

        const filas = await waitFor(() => {
            const b = secao("Filas e recursos");
            expect(within(b).getAllByText("não informado").length).toBeGreaterThan(0);
            return b;
        });
        expect(within(filas).queryByText("0")).toBeNull();
    });

    it("erro nas métricas mostra 'Tentar de novo' e segue com a saúde", async () => {
        new FakeServer()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK })
            .responder("GET", "/metrics", apiError(500, "interno", "falha"))
            .responder("GET", "/manifests", { status: 200, corpo: { items: [] } })
            .instalar();

        montar();

        const filas = await waitFor(() => {
            const b = secao("Filas e recursos");
            expect(within(b).getByText("Não consegui ler as métricas")).toBeInTheDocument();
            return b;
        });
        expect(within(filas).getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
        expect(within(secao("Servidor")).getByText("No ar, com os bancos respondendo")).toBeInTheDocument();
    });

    it("o botão Atualizar refaz as consultas", async () => {
        const servidor = servidorPadrao();
        montar();
        await screen.findByText("No ar, com os bancos respondendo");
        const antes = servidor.chamadasPara("GET", "/healthz").length;

        await userEvent.click(screen.getByRole("button", { name: "Atualizar" }));

        await waitFor(() => expect(servidor.chamadasPara("GET", "/healthz").length).toBeGreaterThan(antes));
        expect(servidor.chamadasPara("GET", "/metrics").length).toBeGreaterThanOrEqual(2);
        expect(servidor.chamadasPara("GET", "/manifests").length).toBeGreaterThanOrEqual(2);
    });

    it("atualiza sozinha a cada 15 s, e não quando a aba está escondida", async () => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        try {
            expect(INTERVALO_DE_ATUALIZACAO_MS).toBe(15_000);
            const servidor = servidorPadrao();
            montar();
            await screen.findByText("No ar, com os bancos respondendo");
            const inicio = servidor.chamadasPara("GET", "/healthz").length;

            let visivel = "hidden";
            Object.defineProperty(document, "visibilityState", {
                configurable: true,
                get: () => visivel,
            });
            await act(async () => {
                await vi.advanceTimersByTimeAsync(INTERVALO_DE_ATUALIZACAO_MS);
            });
            expect(servidor.chamadasPara("GET", "/healthz").length).toBe(inicio);

            visivel = "visible";
            await act(async () => {
                await vi.advanceTimersByTimeAsync(INTERVALO_DE_ATUALIZACAO_MS);
            });
            expect(servidor.chamadasPara("GET", "/healthz").length).toBeGreaterThan(inicio);
        } finally {
            delete (document as unknown as Record<string, unknown>).visibilityState;
            vi.useRealTimers();
        }
    });
});

describe("Operacao: lista de plugins declarados (HR-48c)", () => {
    it("mostra a lista: o plugin no ar com botão de detalhe, e o indisponível marcado sem botão", async () => {
        servidorPadrao(undefined, { status: 200, corpo: LISTA_DOIS_PLUGINS });
        montar();

        const lista = await screen.findByRole("list", { name: "Plugins declarados" });
        const itens = within(lista).getAllByRole("listitem");
        expect(itens).toHaveLength(2);

        const eco = itens[0] as HTMLElement;
        expect(within(eco).getByText("Plugin Eco")).toBeInTheDocument();
        expect(within(eco).getByText("No ar")).toBeInTheDocument();
        expect(within(eco).getByText(/versão 0\.1\.0/)).toBeInTheDocument();
        expect(within(eco).getByRole("button", { name: "Ver detalhes do plugin eco" })).toBeInTheDocument();

        const quebrado = itens[1] as HTMLElement;
        expect(within(quebrado).getByText("Indisponível")).toBeInTheDocument();
        expect(within(quebrado).getByText(/não respondeu ao handshake/)).toBeInTheDocument();
        // A crase do texto do servidor é Markdown; na tela ela não aparece.
        expect(quebrado.textContent).toContain("[plugins].integracoes");
        expect(quebrado.textContent).not.toContain("`");
        expect(within(quebrado).queryByRole("button")).toBeNull();
    });

    it("lista vazia mostra a explicação de como declarar, sem tratar como erro", async () => {
        servidorPadrao(undefined, { status: 200, corpo: { items: [] } });
        montar();

        expect(
            await screen.findByText(/Nenhum plugin declarado nesta instância/),
        ).toBeInTheDocument();
        expect(screen.getByText("[plugins].integracoes")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Tentar de novo" })).toBeNull();
    });

    it("enquanto a lista carrega, avisa que a primeira consulta pode demorar", async () => {
        servidorPadrao(undefined, { status: 200, corpo: LISTA_DOIS_PLUGINS });
        montar();

        expect(screen.getByText(/Consultando plugins/)).toBeInTheDocument();
        await screen.findByRole("list", { name: "Plugins declarados" });
        expect(screen.queryByText(/Consultando plugins/)).toBeNull();
    });

    it("erro 403 na lista explica o token e 'Tentar de novo' recupera a lista", async () => {
        let chamadas = 0;
        new FakeServer()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK })
            .responder("GET", "/metrics", { status: 200, texto: METRICAS })
            .responder("GET", "/manifests", () => {
                chamadas += 1;
                return chamadas === 1
                    ? apiError(403, "forbidden", "outra org")
                    : { status: 200, corpo: LISTA_DOIS_PLUGINS };
            })
            .instalar();
        montar();

        const bloco = await waitFor(() => {
            const b = secao("Plugins declarados");
            expect(within(b).getByText("Não consegui ler a lista de plugins")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByText(/não alcança esta organização/)).toBeInTheDocument();

        await userEvent.click(within(bloco).getByRole("button", { name: "Tentar de novo" }));

        expect(await screen.findByRole("button", { name: "Ver detalhes do plugin eco" })).toBeInTheDocument();
        expect(screen.queryByText("Não consegui ler a lista de plugins")).toBeNull();
    });

    it("erro na lista não derruba a saúde nem as métricas", async () => {
        servidorPadrao(undefined, apiError(500, "interno", "falha"));
        montar();

        await screen.findByText("No ar, com os bancos respondendo");
        expect(within(secao("Filas e recursos")).getByText("Turnos esperando na fila")).toBeInTheDocument();
        expect(within(secao("Plugins declarados")).getByText("Não consegui ler a lista de plugins")).toBeInTheDocument();
    });

    it("o detalhe abre pela linha da lista e mostra nome, versão e a contagem de campos", async () => {
        const servidor = new FakeServer()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK })
            .responder("GET", "/metrics", { status: 200, texto: METRICAS })
            .responder("GET", "/admin/v1/orgs/org_1/manifests/eco", { status: 200, corpo: MANIFESTO_ECO })
            .responder("GET", "/manifests", { status: 200, corpo: LISTA_DOIS_PLUGINS })
            .instalar();
        montar();

        await userEvent.click(await screen.findByRole("button", { name: "Ver detalhes do plugin eco" }));

        const detalhe = await waitFor(() => {
            const bloco = document.querySelector('dl[aria-label="Detalhes do plugin eco"]');
            expect(bloco).not.toBeNull();
            return bloco as HTMLElement;
        });
        expect(within(detalhe).getByText("Plugin Eco")).toBeInTheDocument();
        expect(within(detalhe).getByText("0.1.0")).toBeInTheDocument();
        expect(within(detalhe).getByText("2")).toBeInTheDocument();
        expect(within(detalhe).getByText("1")).toBeInTheDocument();
        expect(servidor.chamadasPara("GET", "/manifests/eco")).toHaveLength(1);
        expect(
            screen.getByRole("button", { name: "Ver detalhes do plugin eco" }),
        ).toHaveAttribute("aria-pressed", "true");
    });

    it("plugin que não responde no detalhe (503) explica e oferece 'Tentar de novo'", async () => {
        new FakeServer()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK })
            .responder("GET", "/metrics", { status: 200, texto: METRICAS })
            .responder("GET", "/admin/v1/orgs/org_1/manifests/eco", apiError(503, "manifesto_indisponivel", "sem resposta"))
            .responder("GET", "/manifests", { status: 200, corpo: LISTA_DOIS_PLUGINS })
            .instalar();
        montar();

        await userEvent.click(await screen.findByRole("button", { name: "Ver detalhes do plugin eco" }));

        expect(
            await screen.findByText("O plugin está declarado, mas não respondeu ao teste de conexão."),
        ).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
    });
});

describe("Operacao: exportar e importar a organização (HR-48e-2)", () => {
    it("a tela de operação mostra a seção, e a atualização automática não baixa nem envia nada", async () => {
        const servidor = servidorPadrao(undefined, { status: 200, corpo: { items: [] } });
        montar();

        expect(await screen.findByRole("heading", { level: 2, name: "Exportar e importar a organização" })).toBeInTheDocument();
        await screen.findByText("No ar, com os bancos respondendo");
        expect(servidor.chamadasPara("GET", "/backup")).toHaveLength(0);
        expect(servidor.chamadasPara("POST", "/backup/import")).toHaveLength(0);
    });
});

describe("Operacao: cluster, recarga do mapa (HR-48f)", () => {
    const MAPA_UNICO = {
        modo: "instancia_unica",
        instance_id: "",
        default_instance: null,
        segredo_configurado: false,
        instancias: [],
        orgs: {},
    };
    const MAPA_DOIS = {
        modo: "cluster",
        instance_id: "a",
        default_instance: "a",
        segredo_configurado: true,
        instancias: [
            { id: "a", url: "http://a.internal:8080", local: true },
            { id: "b", url: "http://b.internal:8080", local: false },
        ],
        orgs: { org_1: "b", org_2: "b" },
    };
    const RELATORIO_COM_MUDANCA = {
        mudou: true,
        fingerprint_antes: "1111aaaa2222bbbb3333",
        fingerprint_depois: "9999cccc8888dddd7777",
        antes: MAPA_UNICO,
        depois: MAPA_DOIS,
        orgs_que_trocaram: [
            { org: "org_2", de: null, para: "b" },
            { org: "org_1", de: "a", para: "b" },
        ],
        instancias_adicionadas: ["b"],
        instancias_removidas: ["c"],
    };
    const RELATORIO_SEM_MUDANCA = {
        mudou: false,
        fingerprint_antes: "5555",
        fingerprint_depois: "5555",
        antes: MAPA_DOIS,
        depois: MAPA_DOIS,
        orgs_que_trocaram: [],
        instancias_adicionadas: [],
        instancias_removidas: [],
    };
    const RELATORIO_INSTANCIA_UNICA = {
        mudou: false,
        fingerprint_antes: "0000",
        fingerprint_depois: "0000",
        antes: MAPA_UNICO,
        depois: MAPA_UNICO,
        orgs_que_trocaram: [],
        instancias_adicionadas: [],
        instancias_removidas: [],
    };

    function servidorComRecarga(resposta: Parameters<FakeServer["responder"]>[2]) {
        return new FakeServer()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK })
            .responder("GET", "/metrics", { status: 200, texto: METRICAS })
            .responder("GET", "/manifests", { status: 200, corpo: { items: [] } })
            .responder("POST", "/admin/v1/cluster/reload", resposta)
            .instalar();
    }

    /** Abre a confirmação pelo botão e espera o diálogo assentar. */
    async function abrirConfirmacao(): Promise<HTMLElement> {
        await userEvent.click(within(secao("Cluster")).getByRole("button", { name: "Recarregar mapa do cluster" }));
        const dialogo = await screen.findByRole("dialog");
        await waitForModalFocus();
        return dialogo;
    }

    async function confirmarRecarga() {
        const dialogo = await abrirConfirmacao();
        await userEvent.click(within(dialogo).getByRole("button", { name: "Recarregar agora" }));
    }

    it("a seção aparece, e o botão só abre a confirmação: nada é enviado antes de confirmar", async () => {
        const servidor = servidorComRecarga({ status: 200, corpo: RELATORIO_SEM_MUDANCA });
        montar();
        await screen.findByText("No ar, com os bancos respondendo");

        const dialogo = await abrirConfirmacao();

        expect(within(dialogo).getByText(/relê o arquivo de cluster/i)).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", "/admin/v1/cluster/reload")).toHaveLength(0);
    });

    it("cancelar a confirmação não envia nada", async () => {
        const servidor = servidorComRecarga({ status: 200, corpo: RELATORIO_SEM_MUDANCA });
        montar();
        await screen.findByText("No ar, com os bancos respondendo");
        const dialogo = await abrirConfirmacao();

        await userEvent.click(within(dialogo).getByRole("button", { name: "Cancelar" }));

        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
        expect(servidor.chamadasPara("POST", "/admin/v1/cluster/reload")).toHaveLength(0);
        expect(screen.getByText(/Nenhuma recarga feita nesta sessão ainda/)).toBeInTheDocument();
    });

    it("200 com mudança: mostra o que mudou, a impressão digital completa no title e as orgs que trocaram", async () => {
        const servidor = servidorComRecarga({ status: 200, corpo: RELATORIO_COM_MUDANCA });
        montar();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmarRecarga();

        const bloco = await waitFor(() => {
            const b = secao("Cluster");
            expect(within(b).getByText("Mapa recarregado: houve mudança")).toBeInTheDocument();
            return b;
        });
        expect(servidor.chamadasPara("POST", "/admin/v1/cluster/reload")).toHaveLength(1);
        expect(within(bloco).getByText("1111aaaa2222…")).toHaveAttribute("title", "1111aaaa2222bbbb3333");
        expect(within(bloco).getByText("9999cccc8888…")).toHaveAttribute("title", "9999cccc8888dddd7777");
        const tabela = within(bloco).getByRole("table", { name: "Mapa antes e depois da recarga" });
        expect(within(tabela).getByRole("rowheader", { name: "Modo" }).closest("tr")).toHaveTextContent("Instância únicaCluster");
        const orgs = within(bloco).getByRole("list", { name: "Orgs que trocaram de instância" });
        expect(within(orgs).getByText("org_2: de sem destino para b")).toBeInTheDocument();
        expect(within(orgs).getByText("org_1: de a para b")).toBeInTheDocument();
        expect(within(bloco).getByText(/Instâncias removidas:/).textContent).toContain("c");
    });

    it("sem segredo na tela: aparece só 'configurado' ou 'não configurado'", async () => {
        servidorComRecarga({ status: 200, corpo: RELATORIO_COM_MUDANCA });
        montar();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmarRecarga();

        await waitFor(() => expect(within(secao("Cluster")).getByText("configurado")).toBeInTheDocument());
        expect(document.body.textContent).not.toMatch(/segredo_configurado/);
        expect(document.body.textContent).not.toMatch(/internal:8080/);
    });

    it("200 sem mudança: diz que nada mudou e que o conteúdo já estava em uso", async () => {
        servidorComRecarga({ status: 200, corpo: RELATORIO_SEM_MUDANCA });
        montar();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmarRecarga();

        const bloco = await waitFor(() => {
            const b = secao("Cluster");
            expect(within(b).getByText("Mapa recarregado: nada mudou")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByText("O arquivo tem o mesmo conteúdo do mapa que já estava em uso.")).toBeInTheDocument();
        expect(within(bloco).getByText("Nenhuma org trocou de instância.")).toBeInTheDocument();
    });

    it("modo instância única aparece como situação normal, não como erro", async () => {
        servidorComRecarga({ status: 200, corpo: RELATORIO_INSTANCIA_UNICA });
        montar();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmarRecarga();

        const bloco = await waitFor(() => {
            const b = secao("Cluster");
            expect(within(b).getByText(/modo instância única/)).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByText(/Não é erro/)).toBeInTheDocument();
        expect(within(bloco).queryByRole("alert")).toBeNull();
    });

    it("400 cluster_invalido: lista os problemas, diz que o mapa anterior segue, e mostra o arquivo", async () => {
        servidorComRecarga(
            apiError(400, "cluster_invalido", "cluster.toml inválido: instância 'b' sem endereço", {
                arquivo: "/etc/poph/cluster.toml",
                problemas: ["instância 'b' sem endereço", "org 'org_9' aponta para instância inexistente"],
            }),
        );
        montar();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmarRecarga();

        const bloco = await waitFor(() => {
            const b = secao("Cluster");
            expect(within(b).getByText("O arquivo de cluster tem erros")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByText(/Nada mudou: a instância segue com o mapa que já estava em uso/)).toBeInTheDocument();
        expect(within(bloco).getByText("Arquivo: /etc/poph/cluster.toml")).toBeInTheDocument();
        const lista = within(bloco).getByRole("list", { name: "Problemas encontrados no arquivo de cluster" });
        expect(within(lista).getAllByRole("listitem")).toHaveLength(2);
        expect(within(lista).getByText("org 'org_9' aponta para instância inexistente")).toBeInTheDocument();
    });

    it("400 depois de uma recarga que deu certo: mantém o último resultado, marcado como anterior", async () => {
        let chamadas = 0;
        servidorComRecarga(() => {
            chamadas += 1;
            return chamadas === 1
                ? { status: 200, corpo: RELATORIO_COM_MUDANCA }
                : apiError(400, "cluster_invalido", "cluster.toml inválido", { arquivo: "/x/cluster.toml", problemas: ["linha 3: erro"] });
        });
        montar();
        await screen.findByText("No ar, com os bancos respondendo");
        await confirmarRecarga();
        await waitFor(() => expect(within(secao("Cluster")).getByText("Resultado da recarga")).toBeInTheDocument());

        await confirmarRecarga();

        const bloco = await waitFor(() => {
            const b = secao("Cluster");
            expect(within(b).getByText("O arquivo de cluster tem erros")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByText("Última recarga que deu certo nesta sessão")).toBeInTheDocument();
        expect(within(bloco).getByText("Mapa recarregado: houve mudança")).toBeInTheDocument();
    });

    it("403: diz que a ação exige token global, em vez de erro genérico", async () => {
        servidorComRecarga(apiError(403, "forbidden", "token de org não pode esta ação"));
        montar();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmarRecarga();

        const bloco = await waitFor(() => {
            const b = secao("Cluster");
            expect(within(b).getByText("Esta ação exige token global")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByText(/o token de uma organização não alcança/)).toBeInTheDocument();
    });

    it("enquanto a recarga roda, o botão fica bloqueado e um segundo clique não envia outro pedido", async () => {
        const servidor = servidorComRecarga({ status: 200, corpo: RELATORIO_SEM_MUDANCA });
        const fetchFalso = fetch as unknown as Mock;
        const original = fetchFalso.getMockImplementation();
        if (!original) throw new Error("fetch falso sem implementação");
        let liberar: () => void = () => undefined;
        const segurar = new Promise<void>((resolver) => {
            liberar = resolver;
        });
        fetchFalso.mockImplementation(async (entrada: RequestInfo | URL, init?: RequestInit) => {
            if ((init?.method ?? "GET").toUpperCase() === "POST") await segurar;
            return original(entrada, init);
        });
        montar();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmarRecarga();

        const botao = await waitFor(() => {
            const b = within(secao("Cluster")).getByRole("button", { name: /Recarregar mapa do cluster/ });
            expect(b).toBeDisabled();
            return b;
        });
        await userEvent.click(botao);
        expect(within(secao("Cluster")).getByText("Recarregando o mapa…")).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", "/admin/v1/cluster/reload")).toHaveLength(0);

        liberar();

        await waitFor(() => expect(within(secao("Cluster")).getByText("Mapa recarregado: nada mudou")).toBeInTheDocument());
        expect(servidor.chamadasPara("POST", "/admin/v1/cluster/reload")).toHaveLength(1);
        expect(within(secao("Cluster")).getByRole("button", { name: /Recarregar mapa do cluster/ })).toBeEnabled();
    });

    it("falha de rede não quebra a tela: avisa que não sabe se foi aplicada e oferece 'Tentar de novo'", async () => {
        const servidor = servidorComRecarga({ status: 200, corpo: RELATORIO_SEM_MUDANCA });
        const fetchFalso = fetch as unknown as Mock;
        const original = fetchFalso.getMockImplementation();
        if (!original) throw new Error("fetch falso sem implementação");
        fetchFalso.mockImplementation(async (entrada: RequestInfo | URL, init?: RequestInit) => {
            if ((init?.method ?? "GET").toUpperCase() === "POST") throw new TypeError("Failed to fetch");
            return original(entrada, init);
        });
        montar();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmarRecarga();

        const bloco = await waitFor(() => {
            const b = secao("Cluster");
            expect(within(b).getByText("Não consegui recarregar o mapa")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByText(/não sei se a recarga chegou a ser aplicada/)).toBeInTheDocument();
        expect(within(secao("Servidor")).getByText("No ar, com os bancos respondendo")).toBeInTheDocument();

        await userEvent.click(within(bloco).getByRole("button", { name: "Tentar de novo" }));
        expect(await screen.findByRole("dialog")).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", "/admin/v1/cluster/reload")).toHaveLength(0);
    });
});

describe("Operacao em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("formatações e mensagens de erro acompanham o idioma", () => {
        setLocale("en-US");
        expect(formatarTempoNoAr(30)).toBe("less than 1 min");
        expect(formatarTempoNoAr(300)).toBe("5 min");
        expect(formatarTempoNoAr(3_725)).toBe("1 h and 2 min");
        expect(formatarTempoNoAr(90_000)).toBe("1 day and 1 h");
        expect(formatarMemoria(1_572_864)).toBe("1.5 MB");
        expect(formatarVersaoDoBanco(0)).toBe("no version stored yet");
        expect(formatarVersaoDoBanco(12)).toBe("schema 12");
        expect(mensagemDeFalha(new ApiError(404, "x", "cru"), "the plugin")).toBe("Could not find the plugin.");
        expect(mensagemDeFalha(new ApiError(403, "x", "cru"), "the plugin")).toMatch(
            /does not reach this organization/,
        );
        expect(mensagemDeFalha(new ApiError(500, "x", "cru"), "the plugin")).toMatch(/error \(500\)/);
    });

    it("tela traduzida: título, seções, situação, métricas e números no formato inglês", async () => {
        setLocale("en-US");
        servidorPadrao();
        montar();

        expect(await screen.findByRole("heading", { level: 1, name: "Instance health" })).toBeInTheDocument();
        const servidor = await waitFor(() => {
            const bloco = secao("Server");
            expect(within(bloco).getByText("Up, with the databases responding")).toBeInTheDocument();
            return bloco;
        });
        expect(within(servidor).getByText("1 day and 1 h")).toBeInTheDocument();
        expect(within(servidor).getByText("schema 12")).toBeInTheDocument();
        expect(within(servidor).getByText("Conversations database")).toBeInTheDocument();

        const filas = secao("Queues and resources");
        expect(within(filas).getByText("Turns waiting in the queue")).toBeInTheDocument();
        expect(within(filas).getByText("64 MB")).toBeInTheDocument();
        expect(within(filas).getByText("15 · with errors: 1")).toBeInTheDocument();
        expect(within(filas).getByText("4,800")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Refresh" })).toBeInTheDocument();
    });

    it("troca o idioma da tela aberta sem recarregar", async () => {
        servidorPadrao();
        montar();

        expect(await screen.findByRole("heading", { level: 1, name: "Saúde da instância" })).toBeInTheDocument();

        setLocale("en-US");

        expect(await screen.findByRole("heading", { level: 1, name: "Instance health" })).toBeInTheDocument();
        expect(screen.getByText("Up, with the databases responding")).toBeInTheDocument();
        expect(screen.getByRole("heading", { level: 2, name: "Declared plugins" })).toBeInTheDocument();
    });

    it("erro de seção em inglês, com o botão de tentar de novo", async () => {
        setLocale("en-US");
        servidorPadrao({ status: 500, corpo: { code: "interno", message: "falha" } });
        montar();

        const bloco = await waitFor(() => {
            const b = secao("Server");
            expect(within(b).getByText("Could not read the server state")).toBeInTheDocument();
            return b;
        });
        expect(within(bloco).getByRole("button", { name: "Try again" })).toBeInTheDocument();
        expect(within(bloco).getByText(/The instance answered with an error \(500\)/)).toBeInTheDocument();
    });

    it("lista de plugins e detalhe em inglês", async () => {
        setLocale("en-US");
        new FakeServer()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK })
            .responder("GET", "/metrics", { status: 200, texto: METRICAS })
            .responder("GET", "/admin/v1/orgs/org_1/manifests/eco", { status: 200, corpo: MANIFESTO_ECO })
            .responder("GET", "/manifests", { status: 200, corpo: LISTA_DOIS_PLUGINS })
            .instalar();
        montar();

        const lista = await screen.findByRole("list", { name: "Declared plugins" });
        const itens = within(lista).getAllByRole("listitem");
        expect(within(itens[0] as HTMLElement).getByText("Up")).toBeInTheDocument();
        expect(within(itens[0] as HTMLElement).getByText(/version 0\.1\.0/)).toBeInTheDocument();
        expect(within(itens[1] as HTMLElement).getByText("Unavailable")).toBeInTheDocument();

        await userEvent.click(screen.getByRole("button", { name: "See details of plugin eco" }));

        const detalhe = await waitFor(() => {
            const bloco = document.querySelector('dl[aria-label="Details of plugin eco"]');
            expect(bloco).not.toBeNull();
            return bloco as HTMLElement;
        });
        expect(within(detalhe).getByText("Plugin Eco")).toBeInTheDocument();
        expect(within(detalhe).getByText("Configuration fields")).toBeInTheDocument();
        expect(within(detalhe).getByText("Secret fields")).toBeInTheDocument();
    });

    it("cluster em inglês: confirmação, relatório e orgs que trocaram", async () => {
        setLocale("en-US");
        const mapaUnico = {
            modo: "instancia_unica",
            instance_id: "",
            default_instance: null,
            segredo_configurado: false,
            instancias: [],
            orgs: {},
        };
        const mapaDois = {
            modo: "cluster",
            instance_id: "a",
            default_instance: "a",
            segredo_configurado: true,
            instancias: [
                { id: "a", local: true },
                { id: "b", local: false },
            ],
            orgs: { org_1: "b" },
        };
        const relatorio = {
            mudou: true,
            fingerprint_antes: "1111aaaa2222bbbb",
            fingerprint_depois: "9999cccc8888dddd",
            antes: mapaUnico,
            depois: mapaDois,
            orgs_que_trocaram: [{ org: "org_1", de: null, para: "b" }],
            instancias_adicionadas: ["b"],
            instancias_removidas: [],
        };
        new FakeServer()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK })
            .responder("GET", "/metrics", { status: 200, texto: METRICAS })
            .responder("GET", "/manifests", { status: 200, corpo: { items: [] } })
            .responder("POST", "/admin/v1/cluster/reload", { status: 200, corpo: relatorio })
            .instalar();
        montar();
        await screen.findByText("Up, with the databases responding");

        await userEvent.click(within(secao("Cluster")).getByRole("button", { name: "Reload cluster map" }));
        const dialogo = await screen.findByRole("dialog");
        await waitForModalFocus();
        expect(within(dialogo).getByText("Reload the cluster map?")).toBeInTheDocument();

        await userEvent.click(within(dialogo).getByRole("button", { name: "Reload now" }));

        const bloco = await waitFor(() => {
            const b = secao("Cluster");
            expect(within(b).getByText("Map reloaded: there were changes")).toBeInTheDocument();
            return b;
        });
        const tabela = within(bloco).getByRole("table", { name: "Map before and after the reload" });
        expect(within(tabela).getByRole("rowheader", { name: "Mode" }).closest("tr")).toHaveTextContent(
            "Single instanceCluster",
        );
        const orgs = within(bloco).getByRole("list", { name: "Orgs that changed instance" });
        expect(within(orgs).getByText("org_1: from no destination to b")).toBeInTheDocument();
        expect(within(bloco).getByText("Instances added: b.")).toBeInTheDocument();
    });
});


import { afterEach, describe, expect, it, vi, type Mock } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { FakeServer, apiError } from "../testing/server";
import {
    Operation,
    formatMemory,
    formatUptime,
    formatDatabaseVersion,
    REFRESH_INTERVAL_MS,
    messageFailure,
} from "./Operation";
import { ApiError } from "../api/client";
import { setLocale } from "../i18n";
import { waitForModalFocus } from "../testing/modal";

const HEALTH_OK = {
    status: "ok",
    version: "0.9.1",
    uptime_seconds: 90_000,
    databases: { harness: 12, rag: 3 },
};

const METRICS = [
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

const LIST_TWO_PLUGINS = {
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

const MANIFEST_ECHO = {
    nome: "Plugin Eco",
    kind: "eco",
    versao: "0.1.0",
    configuracao: { type: "object", properties: { remetente: {}, modo: {} } },
    segredos: { type: "object", properties: { token: {} } },
};

function build(path = "/orgs/org_1/operacao") {
    return render(
        <MemoryRouter initialEntries={[path]}>
            <Routes>
                <Route path="/orgs/:orgId/operacao" element={<Operation />} />
            </Routes>
        </MemoryRouter>,
    );
}

function section(title: string): HTMLElement {
    const header = screen.getByRole("heading", { level: 2, name: title });
    const block = header.closest("section");
    if (!block) throw new Error(`seção sem bloco: ${title}`);
    return block;
}

function serverDefault(
    health: { status: number; body?: unknown } = { status: 200, body: HEALTH_OK },
    list: { status: number; body?: unknown } = { status: 200, body: { items: [] } },
) {
    return new FakeServer()
        .respond("GET", "/healthz", health)
        .respond("GET", "/metrics", { status: 200, texto: METRICS })
        .respond("GET", "/manifests", list)
        .install();
}

describe("formatação", () => {
    it("tempo no ar em linguagem de operador", () => {
        expect(formatUptime(30)).toBe("menos de 1 min");
        expect(formatUptime(300)).toBe("5 min");
        expect(formatUptime(3_725)).toBe("1 h e 2 min");
        expect(formatUptime(90_000)).toBe("1 dia e 1 h");
        expect(formatUptime(2 * 86_400 + 7_200)).toBe("2 dias e 2 h");
    });

    it("memória em KB, MB ou GB com vírgula brasileira", () => {
        expect(formatMemory(512 * 1024)).toBe("512 KB");
        expect(formatMemory(1_572_864)).toBe("1,5 MB");
        expect(formatMemory(2 * 1024 ** 3)).toBe("2 GB");
    });

    it("versão de banco zero quer dizer que não foi possível ler", () => {
        expect(formatDatabaseVersion(0)).toBe("sem versão gravada ainda");
        expect(formatDatabaseVersion(12)).toBe("esquema 12");
    });

    it("mensagens de erro em português, sem corpo cru", () => {
        expect(messageFailure(new ApiError(403, "x", "cru"), "o plugin")).toMatch(/não alcança esta organização/);
        expect(messageFailure(new ApiError(404, "x", "cru"), "o plugin")).toBe("Não encontrei o plugin.");
        expect(messageFailure(new ApiError(500, "x", "cru"), "o plugin")).toMatch(/respondeu com erro \(500\)/);
        expect(messageFailure(new ApiError(0, "rede", "cru"), "o plugin")).toMatch(/sem conexão|Confira se/);
    });
});

describe("Operacao: saúde e métricas", () => {
    it("mostra o servidor no ar, o tempo no ar, as filas e os recursos", async () => {
        serverDefault();

        build();

        expect(await screen.findByRole("heading", { level: 1, name: "Saúde da instância" })).toBeInTheDocument();
        const servidor = await waitFor(() => {
            const block = section("Servidor");
            expect(within(block).getByText("No ar, com os bancos respondendo")).toBeInTheDocument();
            return block;
        });
        expect(within(servidor).getByText("0.9.1")).toBeInTheDocument();
        expect(within(servidor).getByText("1 dia e 1 h")).toBeInTheDocument();
        expect(within(servidor).getByText("esquema 12")).toBeInTheDocument();

        const queues = section("Filas e recursos");
        expect(within(queues).getByText("Turnos esperando na fila")).toBeInTheDocument();
        expect(within(queues).getByText("4")).toBeInTheDocument();
        expect(within(queues).getByText("64 MB")).toBeInTheDocument();
        expect(within(queues).getByText("15 · com erro: 1")).toBeInTheDocument();
        expect(within(queues).getByText("4.800")).toBeInTheDocument();
    });

    it("saúde 503 com corpo aparece como no ar, com um banco sem resposta", async () => {
        serverDefault({ status: 503, body: { ...HEALTH_OK, status: "degraded" } });

        build();

        expect(await screen.findByText("No ar, mas um dos bancos não respondeu")).toBeInTheDocument();
        expect(within(section("Servidor")).getByText("0.9.1")).toBeInTheDocument();
    });

    it("erro no servidor mostra 'Tentar de novo' e não derruba as métricas", async () => {
        serverDefault({ status: 500, body: { code: "interno", message: "falha" } });

        build();

        const block = await waitFor(() => {
            const b = section("Servidor");
            expect(within(b).getByText("Não consegui ler o estado do servidor")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
        expect(within(section("Filas e recursos")).getByText("Turnos esperando na fila")).toBeInTheDocument();
    });

    it("'Tentar de novo' refaz a consulta da seção com erro", async () => {
        const servidor = new FakeServer()
            .respond("GET", "/healthz", apiError(500, "interno", "falha"))
            .respond("GET", "/metrics", { status: 200, texto: METRICS })
            .respond("GET", "/manifests", { status: 200, body: { items: [] } })
            .install();
        build();

        const block = await waitFor(() => {
            const b = section("Servidor");
            expect(within(b).getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
            return b;
        });
        const before = servidor.callsFor("GET", "/healthz").length;
        await userEvent.click(within(block).getByRole("button", { name: "Tentar de novo" }));

        await waitFor(() => expect(servidor.callsFor("GET", "/healthz").length).toBeGreaterThan(before));
    });

    it("métricas sem nenhum valor conhecido mostram 'não informado', sem zero inventado", async () => {
        new FakeServer()
            .respond("GET", "/healthz", { status: 200, body: HEALTH_OK })
            .respond("GET", "/metrics", { status: 200, texto: "# só comentário\n" })
            .respond("GET", "/manifests", { status: 200, body: { items: [] } })
            .install();

        build();

        const queues = await waitFor(() => {
            const b = section("Filas e recursos");
            expect(within(b).getAllByText("não informado").length).toBeGreaterThan(0);
            return b;
        });
        expect(within(queues).queryByText("0")).toBeNull();
    });

    it("erro nas métricas mostra 'Tentar de novo' e segue com a saúde", async () => {
        new FakeServer()
            .respond("GET", "/healthz", { status: 200, body: HEALTH_OK })
            .respond("GET", "/metrics", apiError(500, "interno", "falha"))
            .respond("GET", "/manifests", { status: 200, body: { items: [] } })
            .install();

        build();

        const queues = await waitFor(() => {
            const b = section("Filas e recursos");
            expect(within(b).getByText("Não consegui ler as métricas")).toBeInTheDocument();
            return b;
        });
        expect(within(queues).getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
        expect(within(section("Servidor")).getByText("No ar, com os bancos respondendo")).toBeInTheDocument();
    });

    it("o botão Atualizar refaz as consultas", async () => {
        const servidor = serverDefault();
        build();
        await screen.findByText("No ar, com os bancos respondendo");
        const before = servidor.callsFor("GET", "/healthz").length;

        await userEvent.click(screen.getByRole("button", { name: "Atualizar" }));

        await waitFor(() => expect(servidor.callsFor("GET", "/healthz").length).toBeGreaterThan(before));
        expect(servidor.callsFor("GET", "/metrics").length).toBeGreaterThanOrEqual(2);
        expect(servidor.callsFor("GET", "/manifests").length).toBeGreaterThanOrEqual(2);
    });

    it("atualiza sozinha a cada 15 s, e não quando a aba está escondida", async () => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        try {
            expect(REFRESH_INTERVAL_MS).toBe(15_000);
            const servidor = serverDefault();
            build();
            await screen.findByText("No ar, com os bancos respondendo");
            const start = servidor.callsFor("GET", "/healthz").length;

            let visible = "hidden";
            Object.defineProperty(document, "visibilityState", {
                configurable: true,
                get: () => visible,
            });
            await act(async () => {
                await vi.advanceTimersByTimeAsync(REFRESH_INTERVAL_MS);
            });
            expect(servidor.callsFor("GET", "/healthz").length).toBe(start);

            visible = "visible";
            await act(async () => {
                await vi.advanceTimersByTimeAsync(REFRESH_INTERVAL_MS);
            });
            expect(servidor.callsFor("GET", "/healthz").length).toBeGreaterThan(start);
        } finally {
            delete (document as unknown as Record<string, unknown>).visibilityState;
            vi.useRealTimers();
        }
    });
});

describe("Operacao: lista de plugins declarados (HR-48c)", () => {
    it("mostra a lista: o plugin no ar com botão de detalhe, e o indisponível marcado sem botão", async () => {
        serverDefault(undefined, { status: 200, body: LIST_TWO_PLUGINS });
        build();

        const list = await screen.findByRole("list", { name: "Plugins declarados" });
        const items = within(list).getAllByRole("listitem");
        expect(items).toHaveLength(2);

        const eco = items[0] as HTMLElement;
        expect(within(eco).getByText("Plugin Eco")).toBeInTheDocument();
        expect(within(eco).getByText("No ar")).toBeInTheDocument();
        expect(within(eco).getByText(/versão 0\.1\.0/)).toBeInTheDocument();
        expect(within(eco).getByRole("button", { name: "Ver detalhes do plugin eco" })).toBeInTheDocument();

        const quebrado = items[1] as HTMLElement;
        expect(within(quebrado).getByText("Indisponível")).toBeInTheDocument();
        expect(within(quebrado).getByText(/não respondeu ao handshake/)).toBeInTheDocument();
        expect(quebrado.textContent).toContain("[plugins].integracoes");
        expect(quebrado.textContent).not.toContain("`");
        expect(within(quebrado).queryByRole("button")).toBeNull();
    });

    it("lista vazia mostra a explicação de como declarar, sem tratar como erro", async () => {
        serverDefault(undefined, { status: 200, body: { items: [] } });
        build();

        expect(
            await screen.findByText(/Nenhum plugin declarado nesta instância/),
        ).toBeInTheDocument();
        expect(screen.getByText("[plugins].integracoes")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Tentar de novo" })).toBeNull();
    });

    it("enquanto a lista carrega, avisa que a primeira consulta pode demorar", async () => {
        serverDefault(undefined, { status: 200, body: LIST_TWO_PLUGINS });
        build();

        expect(screen.getByText(/Consultando plugins/)).toBeInTheDocument();
        await screen.findByRole("list", { name: "Plugins declarados" });
        expect(screen.queryByText(/Consultando plugins/)).toBeNull();
    });

    it("erro 403 na lista explica o token e 'Tentar de novo' recupera a lista", async () => {
        let calls = 0;
        new FakeServer()
            .respond("GET", "/healthz", { status: 200, body: HEALTH_OK })
            .respond("GET", "/metrics", { status: 200, texto: METRICS })
            .respond("GET", "/manifests", () => {
                calls += 1;
                return calls === 1
                    ? apiError(403, "forbidden", "outra org")
                    : { status: 200, body: LIST_TWO_PLUGINS };
            })
            .install();
        build();

        const block = await waitFor(() => {
            const b = section("Plugins declarados");
            expect(within(b).getByText("Não consegui ler a lista de plugins")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByText(/não alcança esta organização/)).toBeInTheDocument();

        await userEvent.click(within(block).getByRole("button", { name: "Tentar de novo" }));

        expect(await screen.findByRole("button", { name: "Ver detalhes do plugin eco" })).toBeInTheDocument();
        expect(screen.queryByText("Não consegui ler a lista de plugins")).toBeNull();
    });

    it("erro na lista não derruba a saúde nem as métricas", async () => {
        serverDefault(undefined, apiError(500, "interno", "falha"));
        build();

        await screen.findByText("No ar, com os bancos respondendo");
        expect(within(section("Filas e recursos")).getByText("Turnos esperando na fila")).toBeInTheDocument();
        expect(within(section("Plugins declarados")).getByText("Não consegui ler a lista de plugins")).toBeInTheDocument();
    });

    it("o detalhe abre pela linha da lista e mostra nome, versão e a contagem de campos", async () => {
        const servidor = new FakeServer()
            .respond("GET", "/healthz", { status: 200, body: HEALTH_OK })
            .respond("GET", "/metrics", { status: 200, texto: METRICS })
            .respond("GET", "/admin/v1/orgs/org_1/manifests/eco", { status: 200, body: MANIFEST_ECHO })
            .respond("GET", "/manifests", { status: 200, body: LIST_TWO_PLUGINS })
            .install();
        build();

        await userEvent.click(await screen.findByRole("button", { name: "Ver detalhes do plugin eco" }));

        const detail = await waitFor(() => {
            const block = document.querySelector('dl[aria-label="Detalhes do plugin eco"]');
            expect(block).not.toBeNull();
            return block as HTMLElement;
        });
        expect(within(detail).getByText("Plugin Eco")).toBeInTheDocument();
        expect(within(detail).getByText("0.1.0")).toBeInTheDocument();
        expect(within(detail).getByText("2")).toBeInTheDocument();
        expect(within(detail).getByText("1")).toBeInTheDocument();
        expect(servidor.callsFor("GET", "/manifests/eco")).toHaveLength(1);
        expect(
            screen.getByRole("button", { name: "Ver detalhes do plugin eco" }),
        ).toHaveAttribute("aria-pressed", "true");
    });

    it("plugin que não responde no detalhe (503) explica e oferece 'Tentar de novo'", async () => {
        new FakeServer()
            .respond("GET", "/healthz", { status: 200, body: HEALTH_OK })
            .respond("GET", "/metrics", { status: 200, texto: METRICS })
            .respond("GET", "/admin/v1/orgs/org_1/manifests/eco", apiError(503, "manifesto_indisponivel", "sem resposta"))
            .respond("GET", "/manifests", { status: 200, body: LIST_TWO_PLUGINS })
            .install();
        build();

        await userEvent.click(await screen.findByRole("button", { name: "Ver detalhes do plugin eco" }));

        expect(
            await screen.findByText("O plugin está declarado, mas não respondeu ao teste de conexão."),
        ).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
    });
});

describe("Operacao: exportar e importar a organização (HR-48e-2)", () => {
    it("a tela de operação mostra a seção, e a atualização automática não baixa nem envia nada", async () => {
        const servidor = serverDefault(undefined, { status: 200, body: { items: [] } });
        build();

        expect(await screen.findByRole("heading", { level: 2, name: "Exportar e importar a organização" })).toBeInTheDocument();
        await screen.findByText("No ar, com os bancos respondendo");
        expect(servidor.callsFor("GET", "/backup")).toHaveLength(0);
        expect(servidor.callsFor("POST", "/backup/import")).toHaveLength(0);
    });
});

describe("Operacao: cluster, recarga do mapa (HR-48f)", () => {
    const MAP_UNIQUE = {
        modo: "instancia_unica",
        instance_id: "",
        default_instance: null,
        segredo_configurado: false,
        instancias: [],
        orgs: {},
    };
    const MAP_TWO = {
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
    const REPORT_WITH_CHANGE = {
        mudou: true,
        fingerprint_antes: "1111aaaa2222bbbb3333",
        fingerprint_depois: "9999cccc8888dddd7777",
        antes: MAP_UNIQUE,
        depois: MAP_TWO,
        orgs_que_trocaram: [
            { org: "org_2", de: null, para: "b" },
            { org: "org_1", de: "a", para: "b" },
        ],
        instancias_adicionadas: ["b"],
        instancias_removidas: ["c"],
    };
    const REPORT_NO_CHANGE = {
        mudou: false,
        fingerprint_antes: "5555",
        fingerprint_depois: "5555",
        antes: MAP_TWO,
        depois: MAP_TWO,
        orgs_que_trocaram: [],
        instancias_adicionadas: [],
        instancias_removidas: [],
    };
    const REPORT_SINGLE_INSTANCE = {
        mudou: false,
        fingerprint_antes: "0000",
        fingerprint_depois: "0000",
        antes: MAP_UNIQUE,
        depois: MAP_UNIQUE,
        orgs_que_trocaram: [],
        instancias_adicionadas: [],
        instancias_removidas: [],
    };

    function serverWithReload(response: Parameters<FakeServer["respond"]>[2]) {
        return new FakeServer()
            .respond("GET", "/healthz", { status: 200, body: HEALTH_OK })
            .respond("GET", "/metrics", { status: 200, texto: METRICS })
            .respond("GET", "/manifests", { status: 200, body: { items: [] } })
            .respond("POST", "/admin/v1/cluster/reload", response)
            .install();
    }

    async function openConfirmation(): Promise<HTMLElement> {
        await userEvent.click(within(section("Cluster")).getByRole("button", { name: "Recarregar mapa do cluster" }));
        const dialog = await screen.findByRole("dialog");
        await waitForModalFocus();
        return dialog;
    }

    async function confirmReload() {
        const dialog = await openConfirmation();
        await userEvent.click(within(dialog).getByRole("button", { name: "Recarregar agora" }));
    }

    it("a seção aparece, e o botão só abre a confirmação: nada é enviado antes de confirmar", async () => {
        const servidor = serverWithReload({ status: 200, body: REPORT_NO_CHANGE });
        build();
        await screen.findByText("No ar, com os bancos respondendo");

        const dialog = await openConfirmation();

        expect(within(dialog).getByText(/relê o arquivo de cluster/i)).toBeInTheDocument();
        expect(servidor.callsFor("POST", "/admin/v1/cluster/reload")).toHaveLength(0);
    });

    it("cancelar a confirmação não envia nada", async () => {
        const servidor = serverWithReload({ status: 200, body: REPORT_NO_CHANGE });
        build();
        await screen.findByText("No ar, com os bancos respondendo");
        const dialog = await openConfirmation();

        await userEvent.click(within(dialog).getByRole("button", { name: "Cancelar" }));

        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
        expect(servidor.callsFor("POST", "/admin/v1/cluster/reload")).toHaveLength(0);
        expect(screen.getByText(/Nenhuma recarga feita nesta sessão ainda/)).toBeInTheDocument();
    });

    it("200 com mudança: mostra o que mudou, a impressão digital completa no title e as orgs que trocaram", async () => {
        const servidor = serverWithReload({ status: 200, body: REPORT_WITH_CHANGE });
        build();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmReload();

        const block = await waitFor(() => {
            const b = section("Cluster");
            expect(within(b).getByText("Mapa recarregado: houve mudança")).toBeInTheDocument();
            return b;
        });
        expect(servidor.callsFor("POST", "/admin/v1/cluster/reload")).toHaveLength(1);
        expect(within(block).getByText("1111aaaa2222…")).toHaveAttribute("title", "1111aaaa2222bbbb3333");
        expect(within(block).getByText("9999cccc8888…")).toHaveAttribute("title", "9999cccc8888dddd7777");
        const table = within(block).getByRole("table", { name: "Mapa antes e depois da recarga" });
        expect(within(table).getByRole("rowheader", { name: "Modo" }).closest("tr")).toHaveTextContent("Instância únicaCluster");
        const orgs = within(block).getByRole("list", { name: "Orgs que trocaram de instância" });
        expect(within(orgs).getByText("org_2: de sem destino para b")).toBeInTheDocument();
        expect(within(orgs).getByText("org_1: de a para b")).toBeInTheDocument();
        expect(within(block).getByText(/Instâncias removidas:/).textContent).toContain("c");
    });

    it("sem segredo na tela: aparece só 'configurado' ou 'não configurado'", async () => {
        serverWithReload({ status: 200, body: REPORT_WITH_CHANGE });
        build();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmReload();

        await waitFor(() => expect(within(section("Cluster")).getByText("configurado")).toBeInTheDocument());
        expect(document.body.textContent).not.toMatch(/segredo_configurado/);
        expect(document.body.textContent).not.toMatch(/internal:8080/);
    });

    it("200 sem mudança: diz que nada mudou e que o conteúdo já estava em uso", async () => {
        serverWithReload({ status: 200, body: REPORT_NO_CHANGE });
        build();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmReload();

        const block = await waitFor(() => {
            const b = section("Cluster");
            expect(within(b).getByText("Mapa recarregado: nada mudou")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByText("O arquivo tem o mesmo conteúdo do mapa que já estava em uso.")).toBeInTheDocument();
        expect(within(block).getByText("Nenhuma org trocou de instância.")).toBeInTheDocument();
    });

    it("modo instância única aparece como situação normal, não como erro", async () => {
        serverWithReload({ status: 200, body: REPORT_SINGLE_INSTANCE });
        build();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmReload();

        const block = await waitFor(() => {
            const b = section("Cluster");
            expect(within(b).getByText(/modo instância única/)).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByText(/Não é erro/)).toBeInTheDocument();
        expect(within(block).queryByRole("alert")).toBeNull();
    });

    it("400 cluster_invalido: lista os problemas, diz que o mapa anterior segue, e mostra o arquivo", async () => {
        serverWithReload(
            apiError(400, "cluster_invalido", "cluster.toml inválido: instância 'b' sem endereço", {
                arquivo: "/etc/poph/cluster.toml",
                problemas: ["instância 'b' sem endereço", "org 'org_9' aponta para instância inexistente"],
            }),
        );
        build();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmReload();

        const block = await waitFor(() => {
            const b = section("Cluster");
            expect(within(b).getByText("O arquivo de cluster tem erros")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByText(/Nada mudou: a instância segue com o mapa que já estava em uso/)).toBeInTheDocument();
        expect(within(block).getByText("Arquivo: /etc/poph/cluster.toml")).toBeInTheDocument();
        const list = within(block).getByRole("list", { name: "Problemas encontrados no arquivo de cluster" });
        expect(within(list).getAllByRole("listitem")).toHaveLength(2);
        expect(within(list).getByText("org 'org_9' aponta para instância inexistente")).toBeInTheDocument();
    });

    it("400 depois de uma recarga que deu certo: mantém o último resultado, marcado como anterior", async () => {
        let calls = 0;
        serverWithReload(() => {
            calls += 1;
            return calls === 1
                ? { status: 200, body: REPORT_WITH_CHANGE }
                : apiError(400, "cluster_invalido", "cluster.toml inválido", { arquivo: "/x/cluster.toml", problemas: ["linha 3: erro"] });
        });
        build();
        await screen.findByText("No ar, com os bancos respondendo");
        await confirmReload();
        await waitFor(() => expect(within(section("Cluster")).getByText("Resultado da recarga")).toBeInTheDocument());

        await confirmReload();

        const block = await waitFor(() => {
            const b = section("Cluster");
            expect(within(b).getByText("O arquivo de cluster tem erros")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByText("Última recarga que deu certo nesta sessão")).toBeInTheDocument();
        expect(within(block).getByText("Mapa recarregado: houve mudança")).toBeInTheDocument();
    });

    it("403: diz que a ação exige token global, em vez de erro genérico", async () => {
        serverWithReload(apiError(403, "forbidden", "token de org não pode esta ação"));
        build();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmReload();

        const block = await waitFor(() => {
            const b = section("Cluster");
            expect(within(b).getByText("Esta ação exige token global")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByText(/o token de uma organização não alcança/)).toBeInTheDocument();
    });

    it("enquanto a recarga roda, o botão fica bloqueado e um segundo clique não envia outro pedido", async () => {
        const servidor = serverWithReload({ status: 200, body: REPORT_NO_CHANGE });
        const fetchFake = fetch as unknown as Mock;
        const original = fetchFake.getMockImplementation();
        if (!original) throw new Error("fetch falso sem implementação");
        let liberar: () => void = () => undefined;
        const hold = new Promise<void>((resolver) => {
            liberar = resolver;
        });
        fetchFake.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
            if ((init?.method ?? "GET").toUpperCase() === "POST") await hold;
            return original(input, init);
        });
        build();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmReload();

        const button = await waitFor(() => {
            const b = within(section("Cluster")).getByRole("button", { name: /Recarregar mapa do cluster/ });
            expect(b).toBeDisabled();
            return b;
        });
        await userEvent.click(button);
        expect(within(section("Cluster")).getByText("Recarregando o mapa…")).toBeInTheDocument();
        expect(servidor.callsFor("POST", "/admin/v1/cluster/reload")).toHaveLength(0);

        liberar();

        await waitFor(() => expect(within(section("Cluster")).getByText("Mapa recarregado: nada mudou")).toBeInTheDocument());
        expect(servidor.callsFor("POST", "/admin/v1/cluster/reload")).toHaveLength(1);
        expect(within(section("Cluster")).getByRole("button", { name: /Recarregar mapa do cluster/ })).toBeEnabled();
    });

    it("falha de rede não quebra a tela: avisa que não sabe se foi aplicada e oferece 'Tentar de novo'", async () => {
        const servidor = serverWithReload({ status: 200, body: REPORT_NO_CHANGE });
        const fetchFake = fetch as unknown as Mock;
        const original = fetchFake.getMockImplementation();
        if (!original) throw new Error("fetch falso sem implementação");
        fetchFake.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
            if ((init?.method ?? "GET").toUpperCase() === "POST") throw new TypeError("Failed to fetch");
            return original(input, init);
        });
        build();
        await screen.findByText("No ar, com os bancos respondendo");

        await confirmReload();

        const block = await waitFor(() => {
            const b = section("Cluster");
            expect(within(b).getByText("Não consegui recarregar o mapa")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByText(/não sei se a recarga chegou a ser aplicada/)).toBeInTheDocument();
        expect(within(section("Servidor")).getByText("No ar, com os bancos respondendo")).toBeInTheDocument();

        await userEvent.click(within(block).getByRole("button", { name: "Tentar de novo" }));
        expect(await screen.findByRole("dialog")).toBeInTheDocument();
        expect(servidor.callsFor("POST", "/admin/v1/cluster/reload")).toHaveLength(0);
    });
});

describe("Operacao em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("formatações e mensagens de erro acompanham o idioma", () => {
        setLocale("en-US");
        expect(formatUptime(30)).toBe("less than 1 min");
        expect(formatUptime(300)).toBe("5 min");
        expect(formatUptime(3_725)).toBe("1 h and 2 min");
        expect(formatUptime(90_000)).toBe("1 day and 1 h");
        expect(formatMemory(1_572_864)).toBe("1.5 MB");
        expect(formatDatabaseVersion(0)).toBe("no version stored yet");
        expect(formatDatabaseVersion(12)).toBe("schema 12");
        expect(messageFailure(new ApiError(404, "x", "cru"), "the plugin")).toBe("Could not find the plugin.");
        expect(messageFailure(new ApiError(403, "x", "cru"), "the plugin")).toMatch(
            /does not reach this organization/,
        );
        expect(messageFailure(new ApiError(500, "x", "cru"), "the plugin")).toMatch(/error \(500\)/);
    });

    it("tela traduzida: título, seções, situação, métricas e números no formato inglês", async () => {
        setLocale("en-US");
        serverDefault();
        build();

        expect(await screen.findByRole("heading", { level: 1, name: "Instance health" })).toBeInTheDocument();
        const servidor = await waitFor(() => {
            const block = section("Server");
            expect(within(block).getByText("Up, with the databases responding")).toBeInTheDocument();
            return block;
        });
        expect(within(servidor).getByText("1 day and 1 h")).toBeInTheDocument();
        expect(within(servidor).getByText("schema 12")).toBeInTheDocument();
        expect(within(servidor).getByText("Conversations database")).toBeInTheDocument();

        const queues = section("Queues and resources");
        expect(within(queues).getByText("Turns waiting in the queue")).toBeInTheDocument();
        expect(within(queues).getByText("64 MB")).toBeInTheDocument();
        expect(within(queues).getByText("15 · with errors: 1")).toBeInTheDocument();
        expect(within(queues).getByText("4,800")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Refresh" })).toBeInTheDocument();
    });

    it("troca o idioma da tela aberta sem recarregar", async () => {
        serverDefault();
        build();

        expect(await screen.findByRole("heading", { level: 1, name: "Saúde da instância" })).toBeInTheDocument();

        setLocale("en-US");

        expect(await screen.findByRole("heading", { level: 1, name: "Instance health" })).toBeInTheDocument();
        expect(screen.getByText("Up, with the databases responding")).toBeInTheDocument();
        expect(screen.getByRole("heading", { level: 2, name: "Declared plugins" })).toBeInTheDocument();
    });

    it("erro de seção em inglês, com o botão de tentar de novo", async () => {
        setLocale("en-US");
        serverDefault({ status: 500, body: { code: "interno", message: "falha" } });
        build();

        const block = await waitFor(() => {
            const b = section("Server");
            expect(within(b).getByText("Could not read the server state")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByRole("button", { name: "Try again" })).toBeInTheDocument();
        expect(within(block).getByText(/The instance answered with an error \(500\)/)).toBeInTheDocument();
    });

    it("lista de plugins e detalhe em inglês", async () => {
        setLocale("en-US");
        new FakeServer()
            .respond("GET", "/healthz", { status: 200, body: HEALTH_OK })
            .respond("GET", "/metrics", { status: 200, texto: METRICS })
            .respond("GET", "/admin/v1/orgs/org_1/manifests/eco", { status: 200, body: MANIFEST_ECHO })
            .respond("GET", "/manifests", { status: 200, body: LIST_TWO_PLUGINS })
            .install();
        build();

        const list = await screen.findByRole("list", { name: "Declared plugins" });
        const items = within(list).getAllByRole("listitem");
        expect(within(items[0] as HTMLElement).getByText("Up")).toBeInTheDocument();
        expect(within(items[0] as HTMLElement).getByText(/version 0\.1\.0/)).toBeInTheDocument();
        expect(within(items[1] as HTMLElement).getByText("Unavailable")).toBeInTheDocument();

        await userEvent.click(screen.getByRole("button", { name: "See details of plugin eco" }));

        const detail = await waitFor(() => {
            const block = document.querySelector('dl[aria-label="Details of plugin eco"]');
            expect(block).not.toBeNull();
            return block as HTMLElement;
        });
        expect(within(detail).getByText("Plugin Eco")).toBeInTheDocument();
        expect(within(detail).getByText("Configuration fields")).toBeInTheDocument();
        expect(within(detail).getByText("Secret fields")).toBeInTheDocument();
    });

    it("cluster em inglês: confirmação, relatório e orgs que trocaram", async () => {
        setLocale("en-US");
        const uniqueMap = {
            modo: "instancia_unica",
            instance_id: "",
            default_instance: null,
            segredo_configurado: false,
            instancias: [],
            orgs: {},
        };
        const mapTwo = {
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
        const report = {
            mudou: true,
            fingerprint_antes: "1111aaaa2222bbbb",
            fingerprint_depois: "9999cccc8888dddd",
            antes: uniqueMap,
            depois: mapTwo,
            orgs_que_trocaram: [{ org: "org_1", de: null, para: "b" }],
            instancias_adicionadas: ["b"],
            instancias_removidas: [],
        };
        new FakeServer()
            .respond("GET", "/healthz", { status: 200, body: HEALTH_OK })
            .respond("GET", "/metrics", { status: 200, texto: METRICS })
            .respond("GET", "/manifests", { status: 200, body: { items: [] } })
            .respond("POST", "/admin/v1/cluster/reload", { status: 200, body: report })
            .install();
        build();
        await screen.findByText("Up, with the databases responding");

        await userEvent.click(within(section("Cluster")).getByRole("button", { name: "Reload cluster map" }));
        const dialog = await screen.findByRole("dialog");
        await waitForModalFocus();
        expect(within(dialog).getByText("Reload the cluster map?")).toBeInTheDocument();

        await userEvent.click(within(dialog).getByRole("button", { name: "Reload now" }));

        const block = await waitFor(() => {
            const b = section("Cluster");
            expect(within(b).getByText("Map reloaded: there were changes")).toBeInTheDocument();
            return b;
        });
        const table = within(block).getByRole("table", { name: "Map before and after the reload" });
        expect(within(table).getByRole("rowheader", { name: "Mode" }).closest("tr")).toHaveTextContent(
            "Single instanceCluster",
        );
        const orgs = within(block).getByRole("list", { name: "Orgs that changed instance" });
        expect(within(orgs).getByText("org_1: from no destination to b")).toBeInTheDocument();
        expect(within(block).getByText("Instances added: b.")).toBeInTheDocument();
    });
});

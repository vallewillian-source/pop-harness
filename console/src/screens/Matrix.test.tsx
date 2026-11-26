
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { FakeServer, apiError, type Call } from "../testing/server";
import { ApiError, setToken, whenExpired } from "../api/client";
import { setLocale } from "../i18n";
import { waitForModalFocus } from "../testing/modal";
import {
    Matrix,
    conflictMessage,
    messageSpace,
    buildRows,
    statusConnection,
    textOrigin,
    valueFailures,
    valueLag,
} from "./Matrix";

const ORG = "org_1";
const PATH = `/admin/v1/orgs/${ORG}/matrix`;

const MODULE_OFF = {
    org_id: ORG,
    enabled: false,
    enabled_at: null,
    updated_at: 1700,
    connected: true,
    backlog: { messages: 0, oldest_seconds: 0 },
    pending_failures: 0,
    last_failure: null,
};

const MODULE_ON = {
    org_id: ORG,
    enabled: true,
    enabled_at: 1_700_000_000_000,
    updated_at: 2400,
    connected: true,
    backlog: { messages: 3, oldest_seconds: 65 },
    pending_failures: 1,
    last_failure: "o WhatsApp recusou o envio",
};

const DEPARTMENTS = {
    items: [
        { id: "dep_1", org_id: ORG, name: "Vendas", status: "active", version: 1, updated_at: 1 },
        { id: "dep_2", org_id: ORG, name: "Suporte", status: "active", version: 1, updated_at: 1 },
        { id: "dep_3", org_id: ORG, name: "Antigo", status: "archived", version: 1, updated_at: 1 },
    ],
    next_cursor: null,
};

const SPACE_SUPPORT = {
    alias: "#dep_dep2:matrix-dev.local",
    created_at: 1,
    department_id: "dep_2",
    department_name: "Suporte",
    origin: "auto",
    space_room_id: "!suporte:matrix-dev.local",
    updated_at: 1,
};

function build(path = `/orgs/${ORG}/matrix`) {
    return render(
        <MemoryRouter initialEntries={[path]}>
            <Routes>
                <Route path="/orgs/:orgId/matrix" element={<Matrix />} />
            </Routes>
        </MemoryRouter>,
    );
}

function serverDefault(
    options: {
        module?: unknown;
        moduleStatus?: number;
        spaces?: unknown[];
        spacesStatus?: number;
        departments?: unknown;
    } = {},
) {
    return new FakeServer()
        .respond("GET", `${PATH}/spaces`, {
            status: options.spacesStatus ?? 200,
            body: options.spacesStatus === 500
                ? { code: "interno", message: "falha" }
                : { items: options.spaces ?? [] },
        })
        .respond("PUT", `${PATH}/spaces`, {
            status: 200,
            body: { ...SPACE_SUPPORT, department_id: "dep_1", space_room_id: "!novo:matrix-dev.local" },
        })
        .respond("DELETE", `${PATH}/spaces/`, {
            status: 200,
            body: { department_id: "dep_2", removed: true, rooms_forgotten: 1 },
        })
        .respond("GET", "/departments", {
            status: 200,
            body: options.departments ?? DEPARTMENTS,
        })
        .respond("GET", PATH, {
            status: options.moduleStatus ?? 200,
            body: options.moduleStatus && options.moduleStatus !== 200
                ? { code: "interno", message: "falha" }
                : options.module ?? MODULE_OFF,
        })
        .respond("PUT", PATH, (call) => ({
            status: 200,
            body: { ...MODULE_ON, enabled: (call.body as { enabled: boolean }).enabled },
        }))
        .install();
}

function first(calls: Call[]): Call {
    const [firstCall] = calls;
    if (!firstCall) throw new Error("a chamada esperada não aconteceu");
    return firstCall;
}

function rowFrom(nome: string): HTMLElement {
    const table = screen.getByRole("table", { name: "Espaços por departamento" });
    const row = within(table)
        .getAllByRole("row")
        .find((cell) => within(cell).queryByText(nome, { exact: true }) !== null);
    if (!row) throw new Error(`sem linha para ${nome}`);
    return row;
}

async function dialogOpened(): Promise<HTMLElement> {
    return waitFor(() => {
        const d = document.querySelector('[role="dialog"]');
        if (!d) throw new Error("sem diálogo");
        return d as HTMLElement;
    });
}

function section(title: string): HTMLElement {
    const header = screen.getByRole("heading", { level: 2, name: title });
    const block = header.closest("section");
    if (!block) throw new Error(`seção sem bloco: ${title}`);
    return block;
}

afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setToken(null);
    whenExpired(null);
});

describe("textos da tela", () => {
    it("conexão: conectado, desconectado e desconhecido quando vem nulo", () => {
        expect(statusConnection(true)).toEqual({ value: "Conectado" });
        expect(statusConnection(false).value).toBe("Desconectado");
        expect(statusConnection(null).value).toBe("Desconhecido");
        expect(statusConnection(undefined).value).toBe("Desconhecido");
    });

    it("atraso e falhas em abreviação de painel", () => {
        expect(valueLag(0)).toBe("0 msgs atrás");
        expect(valueLag(1)).toBe("1 msg atrás");
        expect(valueLag(142)).toBe("142 msgs atrás");
        expect(valueFailures(0)).toBe("0 pendentes");
        expect(valueFailures(1)).toBe("1 pendente");
        expect(textOrigin("auto")).toBe("Automático (módulo)");
        expect(textOrigin("linked")).toBe("Vinculado (Element)");
    });

    it("erro 409 e 400 mostram a instrução da API; o resto passa pela tradução comum", () => {
        expect(messageSpace(new ApiError(409, "x", "convide o bot e dê poder 50"), "o espaço")).toBe(
            "convide o bot e dê poder 50",
        );
        expect(messageSpace(new ApiError(404, "x", "cru"), "o departamento")).toBe(
            "Não encontrei o departamento.",
        );
    });

    it("linhas: todo departamento ativo aparece; espaço de departamento arquivado não some", () => {
        const activeList = DEPARTMENTS.items as Parameters<typeof buildRows>[0];
        const rows = buildRows(activeList, [SPACE_SUPPORT]);
        expect(rows.map((row) => [row.nome, row.espaco !== null])).toEqual([
            ["Vendas", false],
            ["Suporte", true],
        ]);
        const orphan = buildRows(activeList, [{ ...SPACE_SUPPORT, department_id: "dep_3", department_name: "Antigo" }]);
        expect(orphan.find((row) => row.nome === "Antigo")?.archived).toBe(true);
    });
});

describe("módulo nesta organização", () => {
    it("faixa de indicadores: ligado, corte, Synapse, atraso, falhas e última falha, sem dado de conversa", async () => {
        serverDefault({ module: MODULE_ON });

        build();

        const block = await waitFor(() => {
            const b = section("Módulo nesta organização");
            expect(within(b).getByText("Ligado")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByText("Conectado")).toBeInTheDocument();
        expect(within(block).getByText("3 msgs atrás")).toBeInTheDocument();
        expect(within(block).getByText("a mais antiga há 1 min")).toBeInTheDocument();
        expect(within(block).getByText("1 pendente")).toBeInTheDocument();
        expect(within(block).getByText("o WhatsApp recusou o envio")).toBeInTheDocument();
        expect(within(block).getByText(/ativo · desde/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Desligar módulo" })).toBeInTheDocument();
    });

    it("conexão nula aparece como desconhecido e módulo nunca ligado diz isso", async () => {
        serverDefault({ module: { ...MODULE_OFF, connected: null } });

        build();

        const block = await waitFor(() => {
            const b = section("Módulo nesta organização");
            expect(within(b).getByText("Desconhecido")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByText("não configurado nesta instância")).toBeInTheDocument();
        expect(within(block).getByText("pausado · nunca ligado")).toBeInTheDocument();
        expect(within(block).getByText("—")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Ligar módulo Matrix" })).toBeInTheDocument();
    });

    it("módulo desligado: diz que ligar não traz o histórico anterior", async () => {
        serverDefault({ module: MODULE_OFF });

        build();

        expect(await screen.findByText("Módulo Matrix desativado para esta organização")).toBeInTheDocument();
        expect(screen.getByText(/o histórico anterior não é trazido/)).toBeInTheDocument();
    });

    it("Synapse fora com módulo ligado: alerta de espelhamento parado e 'Tentar reconectar'", async () => {
        const servidor = serverDefault({ module: { ...MODULE_ON, connected: false } });

        build();

        expect(
            await screen.findByText(/o envio para o Element está acumulando fila/),
        ).toBeInTheDocument();
        expect(screen.getByText(/continuam operando normalmente, sem perda de mensagens/)).toBeInTheDocument();
        const before = servidor.callsFor("GET", PATH).filter((c) => !c.url.includes("/spaces")).length;
        await userEvent.click(screen.getByRole("button", { name: "Tentar reconectar" }));
        await waitFor(() =>
            expect(
                servidor.callsFor("GET", PATH).filter((c) => !c.url.includes("/spaces")).length,
            ).toBeGreaterThan(before),
        );
    });

    it("carregando: mostra o esqueleto antes de chegar a resposta", async () => {
        vi.stubGlobal("fetch", vi.fn(() => new Promise(() => {})));

        build();

        const block = section("Módulo nesta organização");
        expect(block.querySelector(".sofa-skeleton")).not.toBeNull();
        expect(within(block).queryByText("Ligado")).toBeNull();
        expect(within(block).queryByText("Desligado")).toBeNull();
    });

    it("erro de leitura mostra 'Tentar de novo' e refaz a consulta", async () => {
        const servidor = serverDefault({ moduleStatus: 500 });

        build();

        const block = await waitFor(() => {
            const b = section("Módulo nesta organização");
            expect(within(b).getByText("Não consegui ler o estado do módulo")).toBeInTheDocument();
            return b;
        });
        const before = servidor.callsFor("GET", PATH).filter((c) => !c.url.includes("/spaces")).length;
        await userEvent.click(within(block).getByRole("button", { name: "Tentar de novo" }));

        await waitFor(() =>
            expect(
                servidor.callsFor("GET", PATH).filter((c) => !c.url.includes("/spaces")).length,
            ).toBeGreaterThan(before),
        );
    });

    it("erro 403 na leitura diz que o token não alcança a organização", async () => {
        new FakeServer()
            .respond("GET", `${PATH}/spaces`, { status: 200, body: { items: [] } })
            .respond("GET", "/departments", { status: 200, body: DEPARTMENTS })
            .respond("GET", PATH, apiError(403, "forbidden", "token de outra org"))
            .install();

        build();

        expect(
            await screen.findByText(/não alcança esta organização/),
        ).toBeInTheDocument();
    });

    it("ligar envia o If-Match com a versão lida e mostra o módulo ligado", async () => {
        const servidor = serverDefault({ module: MODULE_OFF });
        build();

        await userEvent.click(await screen.findByRole("button", { name: "Ligar módulo Matrix" }));

        await waitFor(() => expect(servidor.callsFor("PUT", PATH)).toHaveLength(1));
        const call = first(servidor.callsFor("PUT", PATH));
        expect(call.headers["if-match"]).toBe("1700");
        expect(call.body).toEqual({ enabled: true });
        expect(await screen.findByText("Módulo ligado. Só o que chegar daqui em diante é espelhado.")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Desligar módulo" })).toBeInTheDocument();
    });

    it("desligar pede confirmação e só envia depois dela, com o If-Match da leitura", async () => {
        const servidor = serverDefault({ module: MODULE_ON });
        build();

        await userEvent.click(await screen.findByRole("button", { name: "Desligar módulo" }));
        const dialog = await dialogOpened();
        await waitForModalFocus();
        expect(servidor.callsFor("PUT", PATH)).toHaveLength(0);

        await userEvent.click(within(dialog).getByRole("button", { name: "Desligar módulo" }));

        await waitFor(() => expect(servidor.callsFor("PUT", PATH)).toHaveLength(1));
        const call = first(servidor.callsFor("PUT", PATH));
        expect(call.headers["if-match"]).toBe("2400");
        expect(call.body).toEqual({ enabled: false });
    });

    it("412 mostra a frase de conflito e relê o estado", async () => {
        const servidor = new FakeServer()
            .respond("GET", `${PATH}/spaces`, { status: 200, body: { items: [] } })
            .respond("GET", "/departments", { status: 200, body: DEPARTMENTS })
            .respond("PUT", PATH, apiError(412, "version_conflict", "mudou"))
            .respond("GET", PATH, { status: 200, body: MODULE_OFF })
            .install();
        build();

        await userEvent.click(await screen.findByRole("button", { name: "Ligar módulo Matrix" }));

        expect(await screen.findByText(conflictMessage())).toBeInTheDocument();
        await waitFor(() =>
            expect(
                servidor.callsFor("GET", PATH).filter((c) => !c.url.includes("/spaces")).length,
            ).toBeGreaterThanOrEqual(2),
        );
    });

    it("botão fica travado enquanto a gravação não volta", async () => {
        let liberar: () => void = () => {};
        const pending = new Promise<void>((resolver) => {
            liberar = resolver;
        });
        vi.stubGlobal(
            "fetch",
            vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
                const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
                const method = (init?.method ?? "GET").toUpperCase();
                if (method === "PUT" && url.endsWith(PATH)) {
                    await pending;
                    return { status: 200, ok: true, headers: new Headers(), text: async () => JSON.stringify(MODULE_ON) } as Response;
                }
                const body = url.includes("/spaces")
                    ? { items: [] }
                    : url.includes("/departments")
                      ? DEPARTMENTS
                      : MODULE_OFF;
                return { status: 200, ok: true, headers: new Headers(), text: async () => JSON.stringify(body) } as Response;
            }),
        );
        build();

        const button = await screen.findByRole("button", { name: "Ligar módulo Matrix" });
        await userEvent.click(button);

        expect(await screen.findByRole("button", { name: "Ligar módulo Matrix" })).toBeDisabled();
        await act(async () => {
            liberar();
        });
        expect(await screen.findByRole("button", { name: "Desligar módulo" })).toBeInTheDocument();
    });

    it("401 numa sessão aberta dispara o encerramento da sessão", async () => {
        const end = vi.fn();
        setToken("poph_token_da_sessao");
        whenExpired(end);
        new FakeServer()
            .respond("GET", `${PATH}/spaces`, { status: 200, body: { items: [] } })
            .respond("GET", "/departments", { status: 200, body: DEPARTMENTS })
            .respond("GET", PATH, apiError(401, "invalid_token", "sessão caiu"))
            .install();

        build();

        await waitFor(() => expect(end).toHaveBeenCalled());
    });
});

describe("espaços por departamento", () => {
    it("sem espaço: cada departamento ativo vira linha 'Sem espaço' com as duas ações", async () => {
        serverDefault({ spaces: [] });

        build();

        const table = await screen.findByRole("table", { name: "Espaços por departamento" });
        expect(within(table).getAllByText("Nenhum espaço vinculado")).toHaveLength(2);
        expect(within(table).getAllByText("Sem espaço")).toHaveLength(2);
        expect(within(table).getAllByRole("button", { name: /Criar espaço automático/ })).toHaveLength(2);
        expect(within(table).getAllByRole("button", { name: /Ligar espaço existente/ })).toHaveLength(2);
        expect(within(table).queryByText("Antigo")).toBeNull();
    });

    it("sem nenhum departamento ativo mostra o vazio", async () => {
        serverDefault({ spaces: [], departments: { items: [], next_cursor: null } });

        build();

        expect(await screen.findByText("Nenhum departamento ativo")).toBeInTheDocument();
    });

    it("linha com espaço mostra alias, origem, status e o botão de desligar", async () => {
        serverDefault({ spaces: [SPACE_SUPPORT] });

        build();

        const row = await waitFor(() => rowFrom("Suporte"));
        expect(within(row).getByText("#dep_dep2:matrix-dev.local")).toBeInTheDocument();
        expect(within(row).getByText("Automático (módulo)")).toBeInTheDocument();
        expect(within(row).getByText("Ativo")).toBeInTheDocument();
        expect(within(row).getByRole("button", { name: "Desligar espaço do departamento Suporte" })).toBeInTheDocument();
        expect(within(rowFrom("Vendas")).getByText("Sem espaço")).toBeInTheDocument();
    });

    it("regra de operação fixa: desligar só remove o vínculo; sala e histórico ficam no Synapse", async () => {
        serverDefault({ spaces: [SPACE_SUPPORT] });

        build();

        expect(await screen.findByText("Regra de operação")).toBeInTheDocument();
        expect(
            screen.getByText(/remove apenas o vínculo do departamento no Harness/),
        ).toBeInTheDocument();
    });

    it("erro ao listar mostra 'Tentar de novo' no lugar da tabela", async () => {
        serverDefault({ spacesStatus: 500 });

        build();

        const block = await waitFor(() => {
            const b = section("Espaços por departamento");
            expect(within(b).getByText("Não consegui ler os espaços")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
    });

    it("'Criar auto' envia só o departamento (espaço automático) e cria uma vez", async () => {
        const servidor = serverDefault({ spaces: [] });
        build();

        await userEvent.click(
            await screen.findByRole("button", { name: "Criar espaço automático do departamento Vendas" }),
        );

        await waitFor(() => expect(servidor.callsFor("PUT", `${PATH}/spaces`)).toHaveLength(1));
        const call = first(servidor.callsFor("PUT", `${PATH}/spaces`));
        expect(call.body).toEqual({ department_id: "dep_1" });
        expect(await screen.findByText("Espaço do departamento criado.")).toBeInTheDocument();
    });

    it("'Ligar existente' abre o diálogo; o id vai no PUT e nada cria segundo espaço", async () => {
        const servidor = serverDefault({ spaces: [] });
        build();

        await userEvent.click(
            await screen.findByRole("button", { name: "Ligar espaço existente ao departamento Vendas" }),
        );
        const dialog = await dialogOpened();
        expect(within(dialog).getByText("Vendas (dep_1)")).toBeInTheDocument();
        await userEvent.type(
            within(dialog).getByRole("textbox", { name: /Alias ou room ID/ }),
            "!abc:matrix-dev.local",
        );
        await userEvent.click(within(dialog).getByRole("button", { name: "Ligar espaço" }));

        await waitFor(() => expect(servidor.callsFor("PUT", `${PATH}/spaces`)).toHaveLength(1));
        expect(first(servidor.callsFor("PUT", `${PATH}/spaces`)).body).toEqual({
            department_id: "dep_1",
            space: "!abc:matrix-dev.local",
        });
        expect(await screen.findByText("Espaço existente ligado ao departamento.")).toBeInTheDocument();
    });

    it("cancelar o diálogo não grava nada", async () => {
        const servidor = serverDefault({ spaces: [] });
        build();

        await userEvent.click(
            await screen.findByRole("button", { name: "Ligar espaço existente ao departamento Vendas" }),
        );
        const dialog = await dialogOpened();
        await userEvent.type(
            within(dialog).getByRole("textbox", { name: /Alias ou room ID/ }),
            "#vendas:matrix-dev.local",
        );
        await userEvent.click(within(dialog).getByRole("button", { name: "Cancelar" }));

        await waitFor(() => expect(document.querySelector('[role="dialog"]')).toBeNull());
        expect(servidor.callsFor("PUT", `${PATH}/spaces`)).toHaveLength(0);
    });

    it("'Ligar espaço' fica desabilitado enquanto o campo está vazio (vazio criaria o automático)", async () => {
        serverDefault({ spaces: [] });
        build();

        await userEvent.click(
            await screen.findByRole("button", { name: "Ligar espaço existente ao departamento Vendas" }),
        );
        const dialog = await dialogOpened();
        expect(within(dialog).getByRole("button", { name: "Ligar espaço" })).toBeDisabled();
    });

    it("409 do servidor aparece dentro do diálogo com a instrução do que fazer no Element", async () => {
        new FakeServer()
            .respond("GET", `${PATH}/spaces`, { status: 200, body: { items: [] } })
            .respond("GET", "/departments", { status: 200, body: DEPARTMENTS })
            .respond("PUT", `${PATH}/spaces`, apiError(409, "conflito", "convide o bot do módulo e dê a ele poder 50"))
            .install();
        build();

        await userEvent.click(
            await screen.findByRole("button", { name: "Ligar espaço existente ao departamento Suporte" }),
        );
        const dialog = await dialogOpened();
        await userEvent.type(
            within(dialog).getByRole("textbox", { name: /Alias ou room ID/ }),
            "#suporte:matrix-dev.local",
        );
        await userEvent.click(within(dialog).getByRole("button", { name: "Ligar espaço" }));

        expect(
            await within(dialog).findByText("convide o bot do módulo e dê a ele poder 50"),
        ).toBeInTheDocument();
    });

    it("desligar pede confirmação, chama o DELETE do departamento e diz que nada foi apagado", async () => {
        const servidor = serverDefault({ spaces: [SPACE_SUPPORT] });
        build();

        await userEvent.click(
            await screen.findByRole("button", { name: "Desligar espaço do departamento Suporte" }),
        );
        const dialog = await dialogOpened();
        await waitForModalFocus();
        expect(servidor.callsFor("DELETE", `${PATH}/spaces/dep_2`)).toHaveLength(0);
        expect(within(dialog).getByText(/Nada é apagado no Element/)).toBeInTheDocument();

        await userEvent.click(within(dialog).getByRole("button", { name: "Desligar espaço" }));

        await waitFor(() => expect(servidor.callsFor("DELETE", `${PATH}/spaces/dep_2`)).toHaveLength(1));
        expect(await screen.findByText("Espaço desligado. Nada foi apagado no Element.")).toBeInTheDocument();
    });

    it("espaço de departamento arquivado aparece marcado, sem botão de criar", async () => {
        serverDefault({
            spaces: [{ ...SPACE_SUPPORT, department_id: "dep_3", department_name: "Antigo" }],
            departments: { items: [DEPARTMENTS.items[1]], next_cursor: null },
        });

        build();

        const row = await waitFor(() => rowFrom("Antigo"));
        expect(within(row).getByText("Departamento arquivado")).toBeInTheDocument();
        expect(within(row).queryByRole("button", { name: /Criar espaço automático/ })).toBeNull();
    });
});

describe("Matrix em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("textos derivados acompanham o idioma ativo", () => {
        setLocale("en-US");
        expect(statusConnection(true)).toEqual({ value: "Connected" });
        expect(statusConnection(false).detalhe).toBe("the module retries on its own");
        expect(statusConnection(undefined).detalhe).toBe("not configured in this instance");
        expect(valueLag(1)).toBe("1 msg behind");
        expect(valueLag(142)).toBe("142 msgs behind");
        expect(valueFailures(2)).toBe("2 pending");
        expect(textOrigin("linked")).toBe("Linked (Element)");
        expect(conflictMessage()).toBe("The state changed between reading and saving, try again.");
        expect(messageSpace(new ApiError(404, "x", "cru"), "the department")).toBe(
            "Could not find the department.",
        );
    });

    it("tela traduzida: indicadores, colunas, status e ações", async () => {
        setLocale("en-US");
        serverDefault({ module: MODULE_ON, spaces: [SPACE_SUPPORT] });

        build();

        const block = await waitFor(() => {
            const b = section("Module in this organization");
            expect(within(b).getByText("Enabled")).toBeInTheDocument();
            return b;
        });
        expect(within(block).getByText("Connected")).toBeInTheDocument();
        expect(within(block).getByText("3 msgs behind")).toBeInTheDocument();
        expect(within(block).getByText("1 pending")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Disable module" })).toBeInTheDocument();

        const table = await screen.findByRole("table", { name: "Spaces by department" });
        expect(within(table).getByRole("columnheader", { name: "Department" })).toBeInTheDocument();
        expect(within(table).getByRole("columnheader", { name: "Origin" })).toBeInTheDocument();
        expect(within(table).getByRole("columnheader", { name: "Actions" })).toBeInTheDocument();
        expect(within(table).getByText("Active")).toBeInTheDocument();
        expect(within(table).getByText("Automatic (module)")).toBeInTheDocument();
        expect(
            within(table).getByRole("button", { name: "Disconnect the space of department Suporte" }),
        ).toBeInTheDocument();
        expect(
            screen.getByRole("heading", { level: 2, name: "Spaces by department" }),
        ).toBeInTheDocument();
    });

    it("troca o idioma da tela aberta sem recarregar", async () => {
        serverDefault({ module: MODULE_ON });

        build();

        expect(await screen.findByText("Ligado")).toBeInTheDocument();

        setLocale("en-US");

        expect(await screen.findByText("Enabled")).toBeInTheDocument();
        expect(screen.queryByText("Ligado")).toBeNull();
    });

    it("desligar pede confirmação em inglês e o aviso sai traduzido", async () => {
        setLocale("en-US");
        serverDefault({ module: MODULE_ON });

        build();

        await userEvent.click(await screen.findByRole("button", { name: "Disable module" }));
        const dialog = await dialogOpened();
        await waitForModalFocus();
        expect(
            within(dialog).getByText("Disable the Matrix module in this organization?"),
        ).toBeInTheDocument();
        expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeInTheDocument();

        await userEvent.click(within(dialog).getByRole("button", { name: "Disable module" }));

        expect(
            await screen.findByText(
                "Module disabled. Nothing was deleted: the rooms and spaces stay in Element.",
            ),
        ).toBeInTheDocument();
    });

    it("módulo desligado e Synapse fora aparecem em inglês", async () => {
        setLocale("en-US");
        serverDefault({ module: { ...MODULE_ON, connected: false } });

        build();

        expect(
            await screen.findByText(/sending to Element is piling up in the queue/),
        ).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Try reconnecting" })).toBeInTheDocument();
    });

    it("erro de leitura em inglês e 'Try again' refaz a consulta", async () => {
        setLocale("en-US");
        const servidor = serverDefault({ moduleStatus: 500 });

        build();

        const block = await waitFor(() => {
            const b = section("Module in this organization");
            expect(within(b).getByText("Could not read the module state")).toBeInTheDocument();
            return b;
        });
        const before = servidor.callsFor("GET", PATH).filter((c) => !c.url.includes("/spaces")).length;
        await userEvent.click(within(block).getByRole("button", { name: "Try again" }));

        await waitFor(() =>
            expect(
                servidor.callsFor("GET", PATH).filter((c) => !c.url.includes("/spaces")).length,
            ).toBeGreaterThan(before),
        );
    });

    it("criar espaço e ligar existente em inglês", async () => {
        setLocale("en-US");
        serverDefault({ module: MODULE_ON });

        build();

        const table = await screen.findByRole("table", { name: "Spaces by department" });
        const row = within(table)
            .getAllByRole("row")
            .find((cell) => within(cell).queryByText("Vendas", { exact: true }) !== null);
        if (!row) throw new Error("sem linha para Vendas");

        await userEvent.click(
            within(row).getByRole("button", { name: "Create automatic space for department Vendas" }),
        );

        expect(await screen.findByText("Department space created.")).toBeInTheDocument();

        await userEvent.click(
            within(row).getByRole("button", { name: "Link an existing space to department Vendas" }),
        );
        const dialog = await dialogOpened();
        await waitForModalFocus();
        expect(within(dialog).getByText("Link a department to an existing space")).toBeInTheDocument();
        expect(
            within(dialog).getByLabelText("Alias or room ID of the Matrix space"),
        ).toHaveAttribute("placeholder", "#alias:server or !id:server");
    });
});

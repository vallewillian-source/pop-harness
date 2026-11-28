
import { useEffect } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import type { Agente } from "../api/agents";
import type { ListInstance, Org, OrgInList } from "../api/client";
import { GuardChangesProvider, useGuardChanges } from "../components/GuardChanges";
import { SessionProvider } from "../session/Session";
import { FakeServer } from "../testing/server";
import { Orgs } from "./Orgs";
import { setLocale } from "../i18n";

function org(partial: Partial<OrgInList> & Pick<OrgInList, "id" | "name" | "slug">): OrgInList {
    return {
        status: "active",
        version: 3,
        config: { retention_days: 30 } as unknown as Org["config"],
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...partial,
    };
}

const ACTIVE = org({ id: "org_1", name: "Loja Antiga", slug: "loja-antiga" });

const INACTIVE = org({
    id: "org_2",
    name: "Patinhas Pet Shop",
    slug: "org_patinhas",
    status: "inactive",
    version: 7,
});

const ARCHIVED = org({
    id: "org_3",
    name: "Demonstração Legada",
    slug: "org_demo_arquivada",
    status: "archived",
    version: 1,
    config: {} as unknown as Org["config"],
});

function prepareSession(token: string, scope: "global" | "org", items: Org[]) {
    window.sessionStorage.setItem(
        "poph-console.sessao",
        JSON.stringify({ token, scope, orgs: items }),
    );
}

function Destination() {
    const local = useLocation();
    return <output data-testid="path">{local.pathname}</output>;
}

function EditPending() {
    const guard = useGuardChanges();
    useEffect(() => {
        guard.register({ dirty: true, save: async () => true });
    }, [guard]);
    return null;
}

function build(initial = "/orgs", options: { dirty?: boolean } = {}) {
    return render(
        <MemoryRouter initialEntries={[initial]}>
            <SessionProvider>
                <GuardChangesProvider>
                    {options.dirty ? <EditPending /> : null}
                    <Routes>
                        <Route path="/orgs" element={<Orgs />} />
                        <Route path="/orgs/:orgId" element={<Destination />} />
                    </Routes>
                </GuardChangesProvider>
            </SessionProvider>
        </MemoryRouter>,
    );
}

const AGENTE: Agente = {
    id: "agt_1",
    org_id: "org_1",
    name: "Atendente Demo",
    slug: "atendente-demo",
    description: "",
    status: "active",
    provider: "gemini",
    model: "gemini-3.8-flash",
    default_department_id: null,
    human_handoff_department_id: null,
    version: 1,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

const MATRIX_CONNECTED = {
    org_id: "org_1",
    enabled: true,
    connected: true,
    enabled_at: 1_700_000_000_000,
    last_failure: null,
    pending_failures: 0,
    backlog: { messages: 0, oldest_seconds: 0 },
    updated_at: 1_700_000_000_000,
};

function serverWith(items: Org[]) {
    const servidor = new FakeServer();
    servidor
        .respond("GET", "/agents", () => ({
            status: 200,
            body: { items: [AGENTE], next_cursor: null },
        }))
        .respond("GET", "/matrix", { status: 200, body: MATRIX_CONNECTED })
        .respond("GET", "/admin/v1/orgs", { status: 200, body: { items, next_cursor: null } })
        .install();
    return servidor;
}

async function openMenu(user: ReturnType<typeof userEvent.setup>, nome: string) {
    const gatilho = screen.getByRole("button", { name: `Ações de ${nome}` });
    await user.click(gatilho);
    return screen.getByRole("menu");
}

describe("Orgs em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("traduz lista, abas, colunas, ações e rodapé", async () => {
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);
        setLocale("en-US");

        build();

        expect(await screen.findByText("Loja Antiga")).toBeInTheDocument();
        expect(screen.getByRole("heading", { level: 1, name: "Organizations" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /^All/ })).toHaveTextContent("All(1)");
        expect(screen.getByText("Active agents")).toBeInTheDocument();
        expect(screen.getByText("Showing 1 of 1 organization")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Open →" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "New organization" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Actions for Loja Antiga" })).toBeInTheDocument();
        expect(screen.getByText("Matrix Synapse mirror")).toBeInTheDocument();
    });

    it("troca o idioma da lista aberta sem recarregar", async () => {
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);
        setLocale("pt-BR");

        build();

        expect(await screen.findByRole("heading", { level: 1, name: "Organizações" })).toBeInTheDocument();

        setLocale("en-US");

        expect(await screen.findByRole("heading", { level: 1, name: "Organizations" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /^All/ })).toHaveTextContent("All(1)");
    });
});

describe("Orgs", () => {
    it("token global vê as ações de gestão e abre o formulário de criação", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);

        build();

        expect(await screen.findByText("Loja Antiga")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Acessar →" })).toBeInTheDocument();

        const gatilho = screen.getByRole("button", { name: "Ações de Loja Antiga" });
        expect(gatilho).toHaveAttribute("aria-expanded", "false");
        const menu = await openMenu(user, "Loja Antiga");
        expect(gatilho).toHaveAttribute("aria-expanded", "true");
        expect(menu).toHaveAccessibleName("Ações de Loja Antiga");
        expect(within(menu).getByRole("menuitem", { name: "Editar" })).toBeInTheDocument();
        expect(within(menu).getByRole("menuitem", { name: "Excluir" })).toBeInTheDocument();
        expect(within(menu).queryByRole("menuitem", { name: "Reativar" })).not.toBeInTheDocument();
        await user.keyboard("{Escape}");

        await user.click(screen.getByRole("button", { name: "Nova organização" }));
        expect(await screen.findByRole("dialog")).toHaveAccessibleName("Nova organização");
    });

    it("token de org não vê criação, edição, reativação nem exclusão", async () => {
        prepareSession("poph_org_teste", "org", [INACTIVE]);
        serverWith([INACTIVE]);

        build();

        expect(await screen.findByText("Patinhas Pet Shop")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Acessar →" })).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Nova organização" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Ações de Patinhas Pet Shop" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Editar" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Excluir" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Reativar" })).not.toBeInTheDocument();
    });

    it("filtra por aba e por texto, com as contagens corretas", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE, INACTIVE, ARCHIVED]);
        serverWith([ACTIVE, INACTIVE, ARCHIVED]);

        build();

        expect(await screen.findByText("Loja Antiga")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /^Todas/ })).toHaveTextContent("Todas(3)");
        expect(screen.getByRole("button", { name: /^Ativas/ })).toHaveTextContent("Ativas(1)");
        expect(screen.getByRole("button", { name: /^Inativas/ })).toHaveTextContent("Inativas(1)");
        expect(screen.getByRole("button", { name: /^Arquivadas/ })).toHaveTextContent("Arquivadas(1)");

        await user.click(screen.getByRole("button", { name: /^Inativas/ }));
        expect(screen.getByText("Patinhas Pet Shop")).toBeInTheDocument();
        expect(screen.queryByText("Loja Antiga")).not.toBeInTheDocument();
        expect(screen.queryByText("Demonstração Legada")).not.toBeInTheDocument();
        expect(await screen.findByText("Exibindo 1 de 3 organizações")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: /^Todas/ }));
        await user.type(screen.getByRole("searchbox"), "DEMO");
        expect(screen.getByText("Demonstração Legada")).toBeInTheDocument();
        expect(screen.queryByText("Loja Antiga")).not.toBeInTheDocument();
        expect(await screen.findByText("Exibindo 1 de 3 organizações")).toBeInTheDocument();

        await user.clear(screen.getByRole("searchbox"));
        await user.type(screen.getByRole("searchbox"), "patinhas");
        expect(screen.getByText("Patinhas Pet Shop")).toBeInTheDocument();

        await user.clear(screen.getByRole("searchbox"));
        await user.type(screen.getByRole("searchbox"), "inexistente");
        expect(await screen.findByText("Nenhuma organização encontrada")).toBeInTheDocument();
        await user.click(screen.getByRole("button", { name: "Limpar filtros" }));
        expect(await screen.findByText("Loja Antiga")).toBeInTheDocument();
    });

    it("esconde a aba de arquivadas quando não há organização arquivada", async () => {
        prepareSession("pophg_teste", "global", [ACTIVE, INACTIVE]);
        serverWith([ACTIVE, INACTIVE]);

        build();

        expect(await screen.findByText("Loja Antiga")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: /^Arquivadas/ })).not.toBeInTheDocument();
    });

    it("Acessar navega para o painel da organização", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);

        build();

        await user.click(await screen.findByRole("button", { name: "Acessar →" }));
        expect(await screen.findByTestId("path")).toHaveTextContent("/orgs/org_1");
    });

    it("o nome da organização é link para o painel", async () => {
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);

        build();

        const link = await screen.findByRole("link", { name: "Loja Antiga" });
        expect(link).toHaveAttribute("href", "/orgs/org_1");
    });

    it("Editar abre o formulário com a organização escolhida", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);

        build();

        const menu = await openMenu(user, "Loja Antiga");
        await user.click(within(menu).getByRole("menuitem", { name: "Editar" }));
        expect(await screen.findByRole("dialog")).toHaveAccessibleName("Editar organização");
    });

    it("Excluir abre a confirmação e manda o DELETE do slug digitado", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE]);
        const servidor = serverWith([ACTIVE]);
        servidor.respond("DELETE", "/admin/v1/orgs/org_1", { status: 204 });

        build();

        const menu = await openMenu(user, "Loja Antiga");
        await user.click(within(menu).getByRole("menuitem", { name: "Excluir" }));
        const dialog = await screen.findByRole("dialog");
        expect(dialog).toHaveAccessibleName("Excluir organização?");

        await user.type(within(dialog).getByRole("textbox"), "loja-antiga");
        await user.click(within(dialog).getByRole("button", { name: "Excluir organização" }));

        await waitFor(() => expect(servidor.callsFor("DELETE", "/admin/v1/orgs/org_1")).toHaveLength(1));
    });

    it("Reativar manda PATCH com status active e If-Match, e a linha sai de Inativas", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE, INACTIVE]);
        let reativada = false;
        const servidor = new FakeServer();
        servidor
            .respond("GET", "/admin/v1/orgs", () => ({
                status: 200,
                body: {
                    items: reativada
                        ? [ACTIVE, { ...INACTIVE, status: "active", version: 8 }]
                        : [ACTIVE, INACTIVE],
                    next_cursor: null,
                },
            }))
            .respond("PATCH", "/admin/v1/orgs/org_2", () => {
                reativada = true;
                return { status: 200, body: { ...INACTIVE, status: "active", version: 8 } };
            })
            .install();

        build();

        await user.click(await screen.findByRole("button", { name: /^Inativas/ }));
        expect(await screen.findByText("Patinhas Pet Shop")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Ações de Patinhas Pet Shop" }));
        await user.click(await screen.findByRole("menuitem", { name: "Reativar" }));

        await waitFor(() =>
            expect(servidor.callsFor("PATCH", "/admin/v1/orgs/org_2")).toHaveLength(1),
        );
        const call = servidor.callsFor("PATCH", "/admin/v1/orgs/org_2")[0]!;
        expect(call.body).toEqual({ status: "active" });
        expect(call.headers["if-match"]).toBe("7");

        await waitFor(() =>
            expect(screen.getByRole("button", { name: /^Inativas/ })).toHaveTextContent("Inativas(0)"),
        );
        expect(screen.queryByText("Patinhas Pet Shop")).not.toBeInTheDocument();
    });

    it("Reativar com 412 mostra o conflito e relê a lista", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [INACTIVE]);
        const servidor = serverWith([INACTIVE]);
        servidor.respond("PATCH", "/admin/v1/orgs/org_2", {
            status: 412,
            body: {
                code: "version_conflict",
                message: "a versão mudou",
                details: { expected_version: 7, current_version: 9 },
            },
        });

        build();

        await user.click(await screen.findByRole("button", { name: /^Todas/ }));
        await user.click(await screen.findByRole("button", { name: "Ações de Patinhas Pet Shop" }));
        await user.click(await screen.findByRole("menuitem", { name: "Reativar" }));

        expect(await screen.findByText("Versão desatualizada")).toBeInTheDocument();
        expect(screen.getByRole("alert")).toHaveTextContent("agora na versão 9");
    });

    it("estado vazio de token global oferece criar a primeira organização", async () => {
        prepareSession("pophg_teste", "global", []);
        serverWith([]);

        build();

        expect(await screen.findByText("Nenhuma organização cadastrada")).toBeInTheDocument();
        expect(
            screen.getByRole("button", { name: "Criar primeira organização" }),
        ).toBeInTheDocument();
    });

    it("mostra o estado de erro com tentativa de novo quando a API falha", async () => {
        prepareSession("pophg_teste", "global", []);
        new FakeServer()
            .respond("GET", "/admin/v1/orgs", {
                status: 500,
                body: { code: "internal", message: "falha inesperada", details: null },
            })
            .install();
        vi.spyOn(console, "error").mockImplementation(() => undefined);

        build();

        expect(
            await screen.findByText("Não consegui carregar as organizações"),
        ).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
        await waitFor(() => expect(screen.getByText("falha inesperada")).toBeInTheDocument());
    });

    it("teclado: Tab chega ao nome e ao menu da linha, e Enter no Acessar navega", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);

        build();

        const link = await screen.findByRole("link", { name: "Loja Antiga" });
        const gatilho = screen.getByRole("button", { name: "Ações de Loja Antiga" });
        const doOpen = screen.getByRole("button", { name: "Acessar →" });

        const reached: HTMLElement[] = [];
        for (let step = 0; step < 20; step += 1) {
            await user.tab();
            const ativo = document.activeElement;
            if (ativo instanceof HTMLElement) reached.push(ativo);
            if (reached.includes(link) && reached.includes(gatilho)) break;
        }
        expect(reached).toContain(link);
        expect(reached).toContain(gatilho);

        doOpen.focus();
        await user.keyboard("{Enter}");
        expect(await screen.findByTestId("path")).toHaveTextContent("/orgs/org_1");
    });


    it("selecionar a linha abre o inspetor com os dados daquela organização", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);

        build();

        await user.click(await screen.findByText("30 dias"));

        const drawer = await screen.findByRole("complementary", {
            name: "Detalhes de Loja Antiga",
        });
        expect(within(drawer).getByText(/slug: loja-antiga · versão v3 \(ETag\/If-Match\)/)).toBeInTheDocument();
        expect(within(drawer).getByText("ID: org_1")).toBeInTheDocument();
        expect(within(drawer).getByText("30 dias")).toBeInTheDocument();
        expect(within(drawer).getByText("Ativa")).toBeInTheDocument();

        expect(screen.getByRole("button", { name: "Acessar →" })).toBeInTheDocument();

        expect(await within(drawer).findByText("Atendente Demo")).toBeInTheDocument();
        expect(within(drawer).getByText("gemini-3.8-flash · Ativo")).toBeInTheDocument();
    });

    it("trocar a seleção atualiza o inspetor para a outra organização", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE, INACTIVE]);
        serverWith([ACTIVE, INACTIVE]);

        build();

        await user.click(await screen.findByText("loja-antiga"));
        expect(
            await screen.findByRole("complementary", { name: "Detalhes de Loja Antiga" }),
        ).toBeInTheDocument();

        await user.click(screen.getByText("org_patinhas"));
        expect(
            await screen.findByRole("complementary", { name: "Detalhes de Patinhas Pet Shop" }),
        ).toBeInTheDocument();
        expect(
            screen.queryByRole("complementary", { name: "Detalhes de Loja Antiga" }),
        ).not.toBeInTheDocument();
    });

    it("Esc fecha o inspetor e devolve o foco ao nome da linha", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);

        build();

        await user.click(await screen.findByText("loja-antiga"));
        expect(await screen.findByRole("complementary")).toBeInTheDocument();

        await user.keyboard("{Escape}");

        await waitFor(() =>
            expect(screen.queryByRole("complementary")).not.toBeInTheDocument(),
        );
        expect(document.activeElement).toBe(screen.getByRole("link", { name: "Loja Antiga" }));
    });

    it("o ✕ também fecha o inspetor", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);

        build();

        await user.click(await screen.findByText("loja-antiga"));
        const drawer = await screen.findByRole("complementary");

        await user.click(within(drawer).getByRole("button", { name: "Fechar inspetor" }));
        await waitFor(() =>
            expect(screen.queryByRole("complementary")).not.toBeInTheDocument(),
        );
    });

    it("falha na consulta de agentes aparece só na linha de Agentes", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE]);
        const servidor = new FakeServer();
        servidor
            .respond("GET", "/agents", {
                status: 500,
                body: { code: "internal", message: "falha ao listar agentes", details: null },
            })
            .respond("GET", "/admin/v1/orgs", {
                status: 200,
                body: { items: [ACTIVE], next_cursor: null },
            })
            .install();

        build();

        await user.click(await screen.findByText("loja-antiga"));
        const drawer = await screen.findByRole("complementary");

        expect(await within(drawer).findByText("não consegui carregar os agentes")).toBeInTheDocument();
        expect(within(drawer).getByText("Retenção de histórico")).toBeInTheDocument();
        expect(within(drawer).getByText("ID: org_1")).toBeInTheDocument();
        expect(within(drawer).getByRole("button", { name: "Acessar Organização →" })).toBeInTheDocument();
    });

    it("copiar põe o comando no clipboard e anuncia Copiado", async () => {
        const user = userEvent.setup();
        const escrever = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(navigator, "clipboard", {
            value: { writeText: escrever },
            configurable: true,
        });
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);

        build();

        await user.click(await screen.findByText("loja-antiga"));
        const drawer = await screen.findByRole("complementary");

        expect(
            await within(drawer).findByText("poph chat --org loja-antiga atendente-demo"),
        ).toBeInTheDocument();

        await user.click(within(drawer).getByRole("button", { name: "Copiar comando" }));

        expect(escrever).toHaveBeenCalledWith("poph chat --org loja-antiga atendente-demo");
        expect(await within(drawer).findByRole("status")).toHaveTextContent("Copiado");
    });

    it("com token de org o inspetor não mostra botão de gestão", async () => {
        const user = userEvent.setup();
        prepareSession("poph_org_teste", "org", [ACTIVE]);
        serverWith([ACTIVE]);

        build();

        await user.click(await screen.findByText("loja-antiga"));
        const drawer = await screen.findByRole("complementary", {
            name: "Detalhes de Loja Antiga",
        });

        expect(within(drawer).queryByRole("button", { name: "Editar" })).not.toBeInTheDocument();
        expect(
            within(drawer).queryByRole("button", { name: /Excluir/ }),
        ).not.toBeInTheDocument();
        expect(within(drawer).queryByRole("button", { name: "Reativar" })).not.toBeInTheDocument();
        expect(
            screen.queryByRole("button", { name: "Ações de Loja Antiga" }),
        ).not.toBeInTheDocument();
        expect(
            within(drawer).getByRole("button", { name: "Acessar Organização →" }),
        ).toBeInTheDocument();
    });

    it("Acessar organização passa pela guarda de alterações não salvas", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", [ACTIVE]);
        serverWith([ACTIVE]);

        build("/orgs", { dirty: true });

        await user.click(await screen.findByText("loja-antiga"));
        const drawer = await screen.findByRole("complementary");

        await user.click(
            within(drawer).getByRole("button", { name: "Acessar Organização →" }),
        );

        expect(await screen.findByRole("dialog")).toHaveAccessibleName("Alterações não salvas");
        expect(screen.queryByTestId("path")).not.toBeInTheDocument();
    });
});

describe("Orgs · layout da prancha 01 (CF-03b)", () => {
    function clusterServer(items: OrgInList[], instances: ListInstance[] | null) {
        const servidor = new FakeServer();
        servidor
            .respond("GET", "/agents", () => ({ status: 200, body: { items: [AGENTE], next_cursor: null } }))
            .respond("GET", "/backup", {
                status: 200,
                texto: "{}",
                headers: { "content-disposition": 'attachment; filename="loja-antiga.json"' },
            })
            .respond("GET", "/admin/v1/orgs", {
                status: 200,
                body: { items, next_cursor: null, ...(instances ? { instancias: instances } : {}) },
            })
            .respond("GET", "/healthz", {
                status: 200,
                body: { status: "ok", version: "0.1.0", uptime_seconds: 15120, databases: { harness: 1, rag: 1 } },
            })
            .respond("GET", "/metrics", { status: 200, texto: "poph_memoria_processo_bytes 131072000\n" })
            .install();
        return servidor;
    }

    function rowOf(nome: string): HTMLElement {
        const row = screen.getByText(nome).closest("tr");
        if (!row) throw new Error(`sem linha para ${nome}`);
        return row;
    }

    it("mostra a contagem de agentes ativos, e '—' com explicação quando a instância não informa", async () => {
        prepareSession("pophg_teste", "global", []);
        const withCount = org({ id: "org_1", name: "Loja Antiga", slug: "loja-antiga", agents_active_count: 3 });
        const noCount = org({ id: "org_2", name: "Patinhas Pet Shop", slug: "patinhas-petshop" });
        clusterServer([withCount, noCount], null);

        build();

        expect(await screen.findByText("3 agentes")).toBeInTheDocument();
        const rowNoCount = rowOf("Patinhas Pet Shop");
        expect(within(rowNoCount).getByTitle("A instância ainda não informa a contagem de agentes ativos")).toHaveTextContent("—");
    });

    it("pede a lista do cluster, mostra o nó de cada organização e não deixa abrir a que está em outro nó", async () => {
        prepareSession("pophg_teste", "global", []);
        const local = org({ id: "org_1", name: "Loja Antiga", slug: "loja-antiga", instancia: "local" });
        const remote = org({ id: "org_2", name: "Patinhas Pet Shop", slug: "patinhas-petshop", instancia: "node-2" });
        const servidor = clusterServer(
            [local, remote],
            [
                { id: "local", situacao: "local", orgs: 1 },
                { id: "node-2", situacao: "inacessivel", orgs: 0, mensagem: "sem resposta" },
            ],
        );

        build();

        expect(await screen.findByText("Loja Antiga")).toBeInTheDocument();
        expect(servidor.callsFor("GET", "/admin/v1/orgs")[0]?.url).toContain("cluster=true");
        expect(screen.getByText("local")).toBeInTheDocument();
        expect(screen.getAllByText("online").length).toBeGreaterThan(0);
        expect(screen.getByText("remoto · inacessível")).toBeInTheDocument();

        expect(screen.queryByRole("link", { name: "Patinhas Pet Shop" })).not.toBeInTheDocument();
        expect(within(rowOf("Patinhas Pet Shop")).getByRole("button", { name: "Acessar →" })).toBeDisabled();
        expect(within(rowOf("Loja Antiga")).getByRole("button", { name: "Acessar →" })).toBeEnabled();

        expect(await screen.findByText(/Uptime 4h 12m/)).toBeInTheDocument();
        expect(screen.getByText(/RAM 125 MB/)).toBeInTheDocument();
        expect(screen.getByText(/1 nó conectado · 1 nó inacessível/)).toBeInTheDocument();
    });

    it("Ver detalhes no menu abre o inspetor e destaca o nome da linha", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", []);
        clusterServer([org({ id: "org_1", name: "Loja Antiga", slug: "loja-antiga" })], null);

        build();

        await screen.findByText("Loja Antiga");
        const menu = await openMenu(user, "Loja Antiga");
        await user.click(within(menu).getByRole("menuitem", { name: "Ver detalhes" }));

        expect(await screen.findByRole("complementary", { name: "Detalhes de Loja Antiga" })).toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Loja Antiga" })).toHaveClass("poph-orgs__name--selected");
    });

    it("Exportar pelo menu baixa o arquivo sem conversas", async () => {
        const user = userEvent.setup();
        const createUrl = vi.fn(() => "blob:teste");
        const revokeUrl = vi.fn();
        const createOriginal = URL.createObjectURL;
        const revokeOriginal = URL.revokeObjectURL;
        URL.createObjectURL = createUrl;
        URL.revokeObjectURL = revokeUrl;
        try {
            prepareSession("pophg_teste", "global", []);
            const servidor = clusterServer([org({ id: "org_1", name: "Loja Antiga", slug: "loja-antiga" })], null);

            build();

            await screen.findByText("Loja Antiga");
            const menu = await openMenu(user, "Loja Antiga");
            await user.click(within(menu).getByRole("menuitem", { name: "Exportar" }));

            await waitFor(() => expect(createUrl).toHaveBeenCalledTimes(1));
            const order = servidor.callsFor("GET", "/backup")[0];
            expect(order?.url).toContain("/admin/v1/orgs/org_1/backup");
            expect(order?.url).not.toContain("com_conversas");
        } finally {
            URL.createObjectURL = createOriginal;
            URL.revokeObjectURL = revokeOriginal;
        }
    });

    it("os cards mostram o Matrix como indisponível e os comandos CLI que existem", async () => {
        prepareSession("pophg_teste", "global", []);
        clusterServer([org({ id: "org_1", name: "Loja Antiga", slug: "loja-antiga" })], null);

        build();

        await screen.findByText("Loja Antiga");
        expect(screen.getByText("Espelho Matrix Synapse")).toBeInTheDocument();
        expect(screen.getByText("indisponível")).toBeInTheDocument();
        expect(screen.getByText("poph org list")).toBeInTheDocument();
        expect(screen.getByText('poph org create --name "nome"')).toBeInTheDocument();
        expect(screen.getByText("poph doctor")).toBeInTheDocument();
    });
});

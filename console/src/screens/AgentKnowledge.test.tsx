
import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardChangesProvider } from "../components/GuardChanges";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { setLocale } from "../i18n";
import { AgentKnowledge } from "./AgentKnowledge";

const ORG = "org_1";
const AGENTE = "ag_1";

type Group = {
    id: string;
    org_id: string;
    name: string;
    description: string;
    status: string;
    version: number;
    created_at: number;
    updated_at: number;
    agents?: string[];
};

type Item = {
    id: string;
    org_id: string;
    group_id: string;
    title: string;
    content: string;
    tags: string[];
    position: number;
    status: string;
    version: number;
    created_at: number;
    updated_at: number;
};

function group(partial: Partial<Group> & { id: string; name: string }): Group {
    return {
        org_id: ORG,
        description: "",
        status: "active",
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        agents: [],
        ...partial,
    };
}

function item(partial: Partial<Item> & { id: string; title: string }): Item {
    return {
        org_id: ORG,
        group_id: "grupo-1",
        content: "O prazo de entrega para o interior é de até 8 dias úteis.",
        tags: [],
        position: 0,
        status: "active",
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...partial,
    };
}

const AGENT_JSON = {
    id: AGENTE,
    org_id: ORG,
    name: "Beatriz",
    slug: "beatriz",
    description: "Atendente da loja",
    status: "active",
    default_department_id: null,
    human_handoff_department_id: null,
    version: 1,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

function build() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/knowledge-base`]}>
            <GuardChangesProvider>
                <Routes>
                    <Route
                        path="/orgs/:orgId/agents/:agentId/knowledge-base"
                        element={<AgentKnowledge />}
                    />
                    <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                </Routes>
            </GuardChangesProvider>
        </MemoryRouter>,
    );
}

type Scenario = {
    groups: Group[];
    items: Record<string, Item[]>;
    errorCreateGroup?: { status: number; body: unknown } | null;
};

function serverWith(scenario: Scenario): FakeServer {
    const servidor = new FakeServer()
        .respond("GET", `/agents/${AGENTE}`, { status: 200, body: AGENT_JSON })
        .respond("GET", `/admin/v1/orgs/${ORG}/agents`, {
            status: 200,
            body: { items: [AGENT_JSON], next_cursor: null },
        });

    servidor.respond("GET", "/items", (call) => {
        const groupId = call.url.split("/knowledge/")[1]?.split("/")[0] ?? "";
        return {
            status: 200,
            body: { items: scenario.items[groupId] ?? [], next_cursor: null },
        };
    });
    servidor.respond("POST", "/items", (call) => {
        const body = call.body as {
            title: string;
            content: string;
            tags?: string[];
            position?: number;
        };
        const groupId = call.url.split("/knowledge/")[1]?.split("/")[0] ?? "grupo-1";
        const created = item({
            id: "item-novo",
            group_id: groupId,
            title: body.title,
            content: body.content,
            tags: body.tags ?? [],
            position: body.position ?? 0,
        });
        scenario.items[groupId] = [...(scenario.items[groupId] ?? []), created];
        return { status: 201, body: created };
    });

    servidor.respond("GET", "/knowledge/grupo-", (call) => {
        const id = call.url.split("/knowledge/")[1]?.split("?")[0] ?? "";
        const found = scenario.groups.find((current) => current.id === id);
        return found
            ? { status: 200, body: found }
            : apiError(404, "not_found", "grupo não encontrado");
    });

    servidor
        .respond("GET", `/admin/v1/orgs/${ORG}/knowledge`, () => ({
            status: 200,
            body: { items: scenario.groups, next_cursor: null },
        }))
        .respond("POST", `/admin/v1/orgs/${ORG}/knowledge`, (call) => {
            if (scenario.errorCreateGroup) return scenario.errorCreateGroup;
            const body = call.body as { name: string; description?: string };
            const created = group({
                id: "grupo-novo",
                name: body.name,
                description: body.description ?? "",
            });
            scenario.groups.push({ ...created, agents: [] });
            return { status: 201, body: created };
        })
        .respond("POST", `/agents/${AGENTE}/knowledge/reindex`, {
            status: 200,
            body: {
                agent: AGENTE,
                indexed: {
                    itens_novos: 2,
                    itens_alterados: 1,
                    itens_inalterados: 0,
                    itens_removidos: 0,
                    embeddings_gerados: 3,
                    trechos_gravados: 3,
                },
            },
        })
        .respond("DELETE", `/agents/${AGENTE}/knowledge/`, { status: 200, body: {} })
        .respond("POST", `/agents/${AGENTE}/knowledge/`, { status: 200, body: {} })
        .respond("DELETE", `/knowledge/grupo-1`, { status: 200, body: {} });

    servidor.respond("GET", `/admin/v1/orgs/${ORG}`, {
        status: 200,
        body: { id: ORG, name: "Exemplo Odontologia" },
    });

    return servidor.install();
}

function scenarioDefault(): Scenario {
    return {
        groups: [
            group({ id: "grupo-1", name: "Políticas da loja", description: "Entrega e frete", agents: [AGENTE] }),
            group({ id: "grupo-2", name: "Perguntas frequentes", description: "Dúvidas gerais" }),
        ],
        items: {
            "grupo-1": [item({ id: "item-1", title: "Prazo de entrega", tags: ["entrega"] })],
            "grupo-2": [],
        },
    };
}


async function chooseInMenu(
    user: ReturnType<typeof userEvent.setup>,
    row: string,
    acao: string,
) {
    await user.click(screen.getByRole("button", { name: row }));
    await user.click(await screen.findByRole("menuitem", { name: acao }));
}

describe("AgenteConhecimento", () => {
    it("carrega grupos e trechos e marca o grupo usado pelo agente", async () => {
        serverWith(scenarioDefault());
        build();

        expect(await screen.findByText("Políticas da loja")).toBeInTheDocument();
        expect(screen.getByText("Perguntas frequentes")).toBeInTheDocument();
        expect(await screen.findByText("Prazo de entrega")).toBeInTheDocument();
        expect(screen.getByText("usado por este agente")).toBeInTheDocument();
        expect(screen.getByText("sem vínculo")).toBeInTheDocument();
        expect(screen.getByText("Índice não verificado")).toBeInTheDocument();
    });

    it("mostra grupos e trechos em tabelas contínuas, sem cartão", async () => {
        serverWith(scenarioDefault());
        build();

        await screen.findByText("Políticas da loja");

        const groups = screen.getByRole("table", { name: "Grupos da base" });
        expect(within(groups).getByRole("columnheader", { name: "Grupo" })).toBeInTheDocument();
        expect(within(groups).getByRole("columnheader", { name: "Vínculo" })).toBeInTheDocument();
        expect(
            within(groups).getByRole("button", { name: "Ações de Políticas da loja" }),
        ).toBeInTheDocument();
        expect(await screen.findByRole("table", { name: "Trechos do grupo" })).toBeInTheDocument();
        expect(document.querySelector(".sofa-card")).toBeNull();
        expect(document.querySelector(".sofa-content-shell")).toBeNull();
    });

    it("trocar o grupo da lista troca os trechos mostrados", async () => {
        const user = userEvent.setup();
        serverWith(scenarioDefault());
        build();

        await screen.findByText("Prazo de entrega");
        expect(screen.getByText("Trechos · Políticas da loja")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Perguntas frequentes" }));

        expect(await screen.findByText("Grupo sem trechos")).toBeInTheDocument();
        expect(screen.getByText("Trechos · Perguntas frequentes")).toBeInTheDocument();
    });

    it("seleciona o grupo pelo teclado e abre o menu da linha pelo teclado", async () => {
        const user = userEvent.setup();
        serverWith(scenarioDefault());
        build();

        await screen.findByText("Prazo de entrega");
        const nome = screen.getByRole("button", { name: "Perguntas frequentes" });
        nome.focus();
        expect(nome).toHaveFocus();
        await user.keyboard("{Enter}");
        expect(await screen.findByText("Grupo sem trechos")).toBeInTheDocument();

        const gatilho = screen.getByRole("button", { name: "Ações de Políticas da loja" });
        gatilho.focus();
        await user.keyboard("{Enter}");
        const menu = await screen.findByRole("menu");
        expect(within(menu).getByRole("menuitem", { name: "Editar" })).toBeInTheDocument();
        expect(within(menu).getByRole("menuitem", { name: "Desvincular" })).toBeInTheDocument();
        expect(within(menu).getByRole("menuitem", { name: "Excluir" })).toBeInTheDocument();
    });

    it("cria grupo enviando o nome e o corpo que a API espera", async () => {
        const user = userEvent.setup();
        const scenario = scenarioDefault();
        const servidor = serverWith(scenario);
        build();

        await screen.findByText("Políticas da loja");
        await user.click(screen.getByRole("button", { name: "Novo grupo" }));
        await waitForModalFocus();
        await user.type(screen.getByRole("textbox", { name: /^Nome/ }), "Horários");
        await user.click(screen.getByRole("button", { name: "Criar grupo" }));

        await waitFor(() => expect(screen.getByText("Horários")).toBeInTheDocument());
        const [order] = servidor.callsFor("POST", `/admin/v1/orgs/${ORG}/knowledge`);
        expect(order?.body).toEqual({ name: "Horários" });
    });

    it("cria trecho no grupo aberto, com o contador de caracteres e as tags em lista", async () => {
        const user = userEvent.setup();
        const servidor = serverWith(scenarioDefault());
        build();

        await screen.findByText("Prazo de entrega");
        await user.click(screen.getByRole("button", { name: "Novo trecho" }));
        await waitForModalFocus();
        await user.type(
            screen.getByRole("textbox", { name: /^Título/ }),
            "Frete grátis",
        );
        await user.type(
            screen.getByRole("textbox", { name: /^Conteúdo/ }),
            "Acima de R$ 199,00 o frete é grátis.",
        );
        await user.type(screen.getByRole("textbox", { name: /^Tags/ }), "frete, preço, frete");
        expect(screen.getByText(/36 \/ 1000 caracteres/)).toBeInTheDocument();
        await user.click(screen.getByRole("button", { name: "Criar trecho" }));

        await waitFor(() => expect(screen.getByText("Frete grátis")).toBeInTheDocument());
        const [order] = servidor.callsFor(
            "POST",
            `/admin/v1/orgs/${ORG}/knowledge/grupo-1/items`,
        );
        expect(order?.body).toEqual({
            title: "Frete grátis",
            content: "Acima de R$ 199,00 o frete é grátis.",
            tags: ["frete", "preço"],
            position: 0,
        });
    });

    it("valida o trecho antes de chamar a API e põe o erro no campo", async () => {
        const user = userEvent.setup();
        const servidor = serverWith(scenarioDefault());
        build();

        await screen.findByText("Prazo de entrega");
        await user.click(screen.getByRole("button", { name: "Novo trecho" }));
        await waitForModalFocus();
        await user.click(screen.getByRole("button", { name: "Criar trecho" }));

        expect(await screen.findByText("informe o título do trecho")).toBeInTheDocument();
        expect(screen.getByText("informe o conteúdo do trecho")).toBeInTheDocument();
        expect(servidor.callsFor("POST", "/items")).toHaveLength(0);
    });

    it("mostra no campo o nome repetido que a API devolve", async () => {
        const user = userEvent.setup();
        const scenario: Scenario = {
            ...scenarioDefault(),
            errorCreateGroup: {
                status: 400,
                body: {
                    code: "validation_error",
                    message: "nome já existe",
                    details: { field: "name", reason: "já existe um grupo com este nome" },
                },
            },
        };
        serverWith(scenario);
        build();

        await screen.findByText("Políticas da loja");
        await user.click(screen.getByRole("button", { name: "Novo grupo" }));
        await waitForModalFocus();
        await user.type(screen.getByRole("textbox", { name: /^Nome/ }), "Políticas da loja");
        await user.click(screen.getByRole("button", { name: "Criar grupo" }));

        expect(
            await screen.findByText("já existe um grupo com este nome"),
        ).toBeInTheDocument();
        expect(screen.getByDisplayValue("Políticas da loja")).toBeInTheDocument();
    });

    it("exclui o grupo só depois de digitar o nome e manda force=true", async () => {
        const user = userEvent.setup();
        const servidor = serverWith(scenarioDefault());
        build();

        await screen.findByText("Políticas da loja");
        await chooseInMenu(user, "Ações de Políticas da loja", "Excluir");
        await waitForModalFocus();

        const dialog = await screen.findByRole("dialog");
        expect(await within(dialog).findByText(/O grupo tem 1 trecho/)).toBeInTheDocument();
        expect(await within(dialog).findByText(/agente usa este grupo/)).toBeInTheDocument();

        const button = within(dialog).getByRole("button", { name: "Excluir grupo" });
        expect(button).toBeDisabled();
        await user.type(
            within(dialog).getByRole("textbox", { name: /Digite/ }),
            "Políticas da loja",
        );
        await user.click(button);

        await waitFor(() =>
            expect(servidor.callsFor("DELETE", "/knowledge/grupo-1")).toHaveLength(1),
        );
        const [order] = servidor.callsFor("DELETE", "/knowledge/grupo-1");
        expect(order?.url).toContain("force=true");
    });

    it("desvincula o grupo do agente pela linha da lista", async () => {
        const user = userEvent.setup();
        const servidor = serverWith(scenarioDefault());
        build();

        await screen.findByText("Políticas da loja");
        await chooseInMenu(user, "Ações de Políticas da loja", "Desvincular");

        await waitFor(() =>
            expect(
                servidor.callsFor("DELETE", `/agents/${AGENTE}/knowledge/grupo-1`),
            ).toHaveLength(1),
        );
    });

    it("no 412 do grupo relê, adota a versão nova e mantém o nome digitado", async () => {
        const user = userEvent.setup();
        const scenario = scenarioDefault();
        scenario.groups[0]!.version = 2;
        const servidor = serverWith(scenario);
        servidor.respond("PATCH", "/knowledge/grupo-1", () => {
            scenario.groups[0]!.version = 3;
            return apiError(412, "version_conflict", "versão desatualizada", {
                expected_version: 2,
                current_version: 3,
            });
        });
        build();

        await screen.findByText("Políticas da loja");
        await chooseInMenu(user, "Ações de Políticas da loja", "Editar");
        await waitForModalFocus();
        const campo = screen.getByRole("textbox", { name: /^Nome/ });
        await user.clear(campo);
        await user.type(campo, "Políticas novas");
        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(
            await screen.findByText(/Outra pessoa salvou este grupo antes de você \(agora na versão 3\)/),
        ).toBeInTheDocument();
        expect(screen.getByDisplayValue("Políticas novas")).toBeInTheDocument();
        const [order] = servidor.callsFor("PATCH", "/knowledge/grupo-1");
        expect(order?.headers["if-match"]).toBe("2");
    });

    it("no 412 do trecho mantém o texto digitado e passa a usar a versão nova", async () => {
        const user = userEvent.setup();
        const scenario = scenarioDefault();
        const servidor = serverWith(scenario);
        servidor.respond("PATCH", "/items/item-1", () => {
            scenario.items["grupo-1"] = [
                { ...scenario.items["grupo-1"]![0]!, version: 2 },
            ];
            return apiError(412, "version_conflict", "versão desatualizada", {
                expected_version: 1,
                current_version: 2,
            });
        });
        build();

        await screen.findByText("Prazo de entrega");
        await chooseInMenu(user, "Ações de Prazo de entrega", "Editar");
        await waitForModalFocus();
        const content = screen.getByRole("textbox", { name: /^Conteúdo/ });
        await user.clear(content);
        await user.type(content, "O prazo mudou: 10 dias úteis.");
        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(
            await screen.findByText(/Outra pessoa salvou este trecho antes de você \(agora na versão 2\)/),
        ).toBeInTheDocument();
        expect(screen.getByDisplayValue("O prazo mudou: 10 dias úteis.")).toBeInTheDocument();
    });

    it("exclui trecho só depois da confirmação pelo título", async () => {
        const user = userEvent.setup();
        const servidor = serverWith(scenarioDefault());
        servidor.respond("DELETE", "/items/item-1", { status: 200, body: { removido: true } });
        build();

        await screen.findByText("Prazo de entrega");
        await chooseInMenu(user, "Ações de Prazo de entrega", "Excluir");
        await waitForModalFocus();

        const dialog = await screen.findByRole("dialog");
        const button = within(dialog).getByRole("button", { name: "Excluir trecho" });
        expect(button).toBeDisabled();
        await user.type(
            within(dialog).getByRole("textbox", { name: /Digite/ }),
            "Prazo de entrega",
        );
        await user.click(button);

        await waitFor(() =>
            expect(servidor.callsFor("DELETE", "/items/item-1")).toHaveLength(1),
        );
        await waitFor(() => expect(screen.queryByText("Prazo de entrega")).not.toBeInTheDocument());
    });

    it("mostra o vazio do grupo sem trechos com o CTA de criar", async () => {
        const scenario = scenarioDefault();
        scenario.items["grupo-1"] = [];
        serverWith(scenario);
        build();

        expect(await screen.findByText("Grupo sem trechos")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Criar primeiro trecho" })).toBeInTheDocument();
    });

    it("mostra o vazio de organização sem grupos com o CTA de criar o primeiro", async () => {
        const scenario = scenarioDefault();
        scenario.groups = [];
        scenario.items = {};
        serverWith(scenario);
        build();

        expect(await screen.findByText("Nenhum grupo de conhecimento")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Criar primeiro grupo" })).toBeInTheDocument();
    });

    it("mostra o erro de carga dos trechos com tentar de novo", async () => {
        const user = userEvent.setup();
        const scenario = scenarioDefault();
        let fail = true;
        new FakeServer()
            .respond("GET", `/agents/${AGENTE}`, { status: 200, body: AGENT_JSON })
            .respond("GET", `/admin/v1/orgs/${ORG}/agents`, {
                status: 200,
                body: { items: [AGENT_JSON], next_cursor: null },
            })
            .respond("GET", "/items", () =>
                fail
                    ? apiError(500, "internal", "falha inesperada")
                    : {
                          status: 200,
                          body: {
                              items: scenario.items["grupo-1"] ?? [],
                              next_cursor: null,
                          },
                      },
            )
            .respond("GET", "/knowledge/grupo-", {
                status: 200,
                body: scenario.groups[0],
            })
            .respond("GET", `/admin/v1/orgs/${ORG}/knowledge`, {
                status: 200,
                body: { items: scenario.groups, next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                body: { id: ORG, name: "Exemplo Odontologia" },
            })
            .install();
        build();

        expect(await screen.findByText("Não consegui carregar os trechos")).toBeInTheDocument();
        fail = false;
        await user.click(screen.getByRole("button", { name: "Tentar de novo" }));
        expect(await screen.findByText("Prazo de entrega")).toBeInTheDocument();
    });

    it("reindexa o agente e mostra o relatório do índice", async () => {
        const user = userEvent.setup();
        const servidor = serverWith(scenarioDefault());
        build();

        await screen.findByText("Índice não verificado");
        await user.click(screen.getByRole("button", { name: "Reindexar agente" }));

        expect(await screen.findByText("Índice atualizado")).toBeInTheDocument();
        expect(await screen.findByText(/2 novos/)).toBeInTheDocument();
        expect(screen.getByText(/3 embeddings gerados/)).toBeInTheDocument();
        expect(
            servidor.callsFor("POST", `/agents/${AGENTE}/knowledge/reindex`),
        ).toHaveLength(1);
    });

    it("explica quando o modelo de embedding não está instalado", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .respond("GET", `/agents/${AGENTE}`, { status: 200, body: AGENT_JSON })
            .respond("GET", `/admin/v1/orgs/${ORG}/agents`, {
                status: 200,
                body: { items: [AGENT_JSON], next_cursor: null },
            })
            .respond("GET", "/items", { status: 200, body: { items: [], next_cursor: null } })
            .respond("GET", `/knowledge/grupo-1`, {
                status: 200,
                body: group({ id: "grupo-1", name: "Políticas da loja", agents: [AGENTE] }),
            })
            .respond("GET", `/admin/v1/orgs/${ORG}/knowledge`, {
                status: 200,
                body: {
                    items: [group({ id: "grupo-1", name: "Políticas da loja", agents: [AGENTE] })],
                    next_cursor: null,
                },
            })
            .respond("POST", `/agents/${AGENTE}/knowledge/reindex`, () =>
                apiError(503, "embedding_unavailable", "modelo de embedding indisponível; instale o modelo"),
            )
            .respond("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                body: { id: ORG, name: "Exemplo Odontologia" },
            })
            .install();
        build();

        await screen.findByText("Políticas da loja");
        await user.click(screen.getByRole("button", { name: "Reindexar agente" }));

        expect(
            await screen.findByText(/O índice não mudou\. Instale o modelo de embedding/),
        ).toBeInTheDocument();
        expect(servidor.callsFor("POST", "/reindex")).toHaveLength(1);
    });
});

describe("I18N-08: Base de conhecimento em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("mostra grupos e trechos em inglês, com vínculo, situação e detalhes", async () => {
        serverWith(scenarioDefault());
        setLocale("en-US");
        build();

        expect(await screen.findByText("Políticas da loja")).toBeInTheDocument();
        expect(screen.getByText("Index not verified")).toBeInTheDocument();
        expect(screen.getByText("used by this agent")).toBeInTheDocument();
        expect(screen.getByText("not linked")).toBeInTheDocument();

        const groups = screen.getByRole("table", { name: "Knowledge groups" });
        expect(within(groups).getByRole("columnheader", { name: "Group" })).toBeInTheDocument();
        expect(within(groups).getByRole("columnheader", { name: "Link" })).toBeInTheDocument();
        expect(within(groups).getByRole("columnheader", { name: "Status" })).toBeInTheDocument();
        expect(within(groups).getByRole("columnheader", { name: "Version" })).toBeInTheDocument();
        expect(within(groups).getByRole("columnheader", { name: "Actions" })).toBeInTheDocument();
        expect(
            within(groups).getByRole("button", { name: "Actions for Políticas da loja" }),
        ).toBeInTheDocument();
        expect(screen.getAllByText("Active").length).toBeGreaterThan(0);

        const excerpts = await screen.findByRole("table", { name: "Group entries" });
        expect(within(excerpts).getByRole("columnheader", { name: "Entry" })).toBeInTheDocument();
        expect(within(excerpts).getByRole("columnheader", { name: "Details" })).toBeInTheDocument();
        expect(screen.getByText("Entries · Políticas da loja")).toBeInTheDocument();
        expect(screen.getByText("1 entry")).toBeInTheDocument();
        expect(screen.getByText(/characters · order 0 · v1/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "New entry" })).toBeInTheDocument();
    });

    it("reindexa em inglês e mostra o relatório do índice", async () => {
        const user = userEvent.setup();
        const servidor = serverWith(scenarioDefault());
        setLocale("en-US");
        build();

        await screen.findByText("Index not verified");
        await user.click(screen.getByRole("button", { name: "Reindex agent" }));

        expect(await screen.findByText("Index updated")).toBeInTheDocument();
        expect(await screen.findByText(/2 new/)).toBeInTheDocument();
        expect(screen.getByText(/3 embeddings generated/)).toBeInTheDocument();
        expect(
            servidor.callsFor("POST", `/agents/${AGENTE}/knowledge/reindex`),
        ).toHaveLength(1);
    });

    it("valida o trecho em inglês antes de chamar a API", async () => {
        const user = userEvent.setup();
        const servidor = serverWith(scenarioDefault());
        setLocale("en-US");
        build();

        await screen.findByText("Prazo de entrega");
        await user.click(screen.getByRole("button", { name: "New entry" }));
        await waitForModalFocus();
        await user.click(screen.getByRole("button", { name: "Create entry" }));

        expect(await screen.findByText("enter the entry title")).toBeInTheDocument();
        expect(screen.getByText("enter the entry content")).toBeInTheDocument();
        expect(screen.getByText(/0 \/ 1000 characters/)).toBeInTheDocument();
        expect(servidor.callsFor("POST", "/items")).toHaveLength(0);
    });

    it("exclui o grupo em inglês com a consequência e a confirmação pelo nome", async () => {
        const user = userEvent.setup();
        const servidor = serverWith(scenarioDefault());
        setLocale("en-US");
        build();

        await screen.findByText("Políticas da loja");
        await user.click(
            screen.getByRole("button", { name: "Actions for Políticas da loja" }),
        );
        await user.click(await screen.findByRole("menuitem", { name: "Delete" }));
        await waitForModalFocus();

        const dialog = await screen.findByRole("dialog");
        expect(within(dialog).getByText("Delete group?")).toBeInTheDocument();
        expect(await within(dialog).findByText(/The group has 1 entry/)).toBeInTheDocument();
        expect(
            await within(dialog).findByText(/1 agent uses this group and will be left without it: Beatriz/),
        ).toBeInTheDocument();

        const button = within(dialog).getByRole("button", { name: "Delete group" });
        expect(button).toBeDisabled();
        await user.type(within(dialog).getByRole("textbox"), "Políticas da loja");
        await user.click(button);

        await waitFor(() =>
            expect(servidor.callsFor("DELETE", "/knowledge/grupo-1")).toHaveLength(1),
        );
        expect(servidor.callsFor("DELETE", "/knowledge/grupo-1")[0]?.url).toContain(
            "force=true",
        );
    });

    it("abre o formulário do trecho em inglês com as dicas do idioma", async () => {
        const user = userEvent.setup();
        serverWith(scenarioDefault());
        setLocale("en-US");
        build();

        await screen.findByText("Prazo de entrega");
        await user.click(screen.getByRole("button", { name: "New entry" }));
        await waitForModalFocus();

        const dialog = await screen.findByRole("dialog");
        expect(within(dialog).getByText("New entry")).toBeInTheDocument();
        expect(within(dialog).getByText("Content")).toBeInTheDocument();
        expect(
            within(dialog).getByText(/This is the text the agent receives in the query/),
        ).toBeInTheDocument();
        expect(within(dialog).getByText("Order")).toBeInTheDocument();
        expect(within(dialog).getByText(/lower appears first/)).toBeInTheDocument();
    });
});

/**
 * Testes da aba Base de conhecimento do agente (HR-41).
 *
 * O que importa: carregar grupos e trechos com o vínculo do agente, criar grupo e trecho com o
 * corpo que sai para a API, validação local sem chamar a API, erro de campo vindo da API, o `412`
 * de concorrência sem perder o que foi digitado, a exclusão de grupo com a confirmação por nome
 * (o `DELETE` leva `force=true`) e o painel do índice com a reindexação.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardaAlteracoesProvider } from "../componentes/GuardaAlteracoes";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { setLocale } from "../i18n";
import { AgenteConhecimento } from "./AgenteConhecimento";

const ORG = "org_1";
const AGENTE = "ag_1";

type Grupo = {
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

function grupo(parcial: Partial<Grupo> & { id: string; name: string }): Grupo {
    return {
        org_id: ORG,
        description: "",
        status: "active",
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        agents: [],
        ...parcial,
    };
}

function item(parcial: Partial<Item> & { id: string; title: string }): Item {
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
        ...parcial,
    };
}

const AGENTE_JSON = {
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

function montar() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/knowledge-base`]}>
            <GuardaAlteracoesProvider>
                <Routes>
                    <Route
                        path="/orgs/:orgId/agents/:agentId/knowledge-base"
                        element={<AgenteConhecimento />}
                    />
                    <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                </Routes>
            </GuardaAlteracoesProvider>
        </MemoryRouter>,
    );
}

type Cenario = {
    grupos: Grupo[];
    itens: Record<string, Item[]>;
    /** Resposta de erro que a API devolve na criação de grupo (nome repetido, por exemplo). */
    erroAoCriarGrupo?: { status: number; corpo: unknown } | null;
};

function servidorCom(cenario: Cenario): FakeServer {
    const servidor = new FakeServer()
        .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: AGENTE_JSON })
        .responder("GET", `/admin/v1/orgs/${ORG}/agents`, {
            status: 200,
            corpo: { items: [AGENTE_JSON], next_cursor: null },
        });

    // Itens por grupo — rota mais específica antes da rota do grupo e antes da criação de grupo
    // (o caminho de `/items` contém `/knowledge`, e o casamento do servidor falso é por trecho).
    servidor.responder("GET", "/items", (chamada) => {
        const grupoId = chamada.url.split("/knowledge/")[1]?.split("/")[0] ?? "";
        return {
            status: 200,
            corpo: { items: cenario.itens[grupoId] ?? [], next_cursor: null },
        };
    });
    servidor.responder("POST", "/items", (chamada) => {
        const corpo = chamada.corpo as {
            title: string;
            content: string;
            tags?: string[];
            position?: number;
        };
        const grupoId = chamada.url.split("/knowledge/")[1]?.split("/")[0] ?? "grupo-1";
        const criado = item({
            id: "item-novo",
            group_id: grupoId,
            title: corpo.title,
            content: corpo.content,
            tags: corpo.tags ?? [],
            position: corpo.position ?? 0,
        });
        cenario.itens[grupoId] = [...(cenario.itens[grupoId] ?? []), criado];
        return { status: 201, corpo: criado };
    });

    // Detalhe de grupo (traz `agents`), antes da lista. Todos os ids do teste começam com
    // "grupo-", o que separa esta rota da listagem (`/knowledge?limit=…`).
    servidor.responder("GET", "/knowledge/grupo-", (chamada) => {
        const id = chamada.url.split("/knowledge/")[1]?.split("?")[0] ?? "";
        const encontrado = cenario.grupos.find((atual) => atual.id === id);
        return encontrado
            ? { status: 200, corpo: encontrado }
            : apiError(404, "not_found", "grupo não encontrado");
    });

    servidor
        .responder("GET", `/admin/v1/orgs/${ORG}/knowledge`, () => ({
            status: 200,
            corpo: { items: cenario.grupos, next_cursor: null },
        }))
        .responder("POST", `/admin/v1/orgs/${ORG}/knowledge`, (chamada) => {
            if (cenario.erroAoCriarGrupo) return cenario.erroAoCriarGrupo;
            const corpo = chamada.corpo as { name: string; description?: string };
            const criado = grupo({
                id: "grupo-novo",
                name: corpo.name,
                description: corpo.description ?? "",
            });
            cenario.grupos.push({ ...criado, agents: [] });
            return { status: 201, corpo: criado };
        })
        .responder("POST", `/agents/${AGENTE}/knowledge/reindex`, {
            status: 200,
            corpo: {
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
        .responder("DELETE", `/agents/${AGENTE}/knowledge/`, { status: 200, corpo: {} })
        .responder("POST", `/agents/${AGENTE}/knowledge/`, { status: 200, corpo: {} })
        .responder("DELETE", `/knowledge/grupo-1`, { status: 200, corpo: {} });

    // A rota da organização casa por prefixo e engoliria `/knowledge`; por isso entra por último.
    servidor.responder("GET", `/admin/v1/orgs/${ORG}`, {
        status: 200,
        corpo: { id: ORG, name: "Exemplo Odontologia" },
    });

    return servidor.instalar();
}

function cenarioPadrao(): Cenario {
    return {
        grupos: [
            grupo({ id: "grupo-1", name: "Políticas da loja", description: "Entrega e frete", agents: [AGENTE] }),
            grupo({ id: "grupo-2", name: "Perguntas frequentes", description: "Dúvidas gerais" }),
        ],
        itens: {
            "grupo-1": [item({ id: "item-1", title: "Prazo de entrega", tags: ["entrega"] })],
            "grupo-2": [],
        },
    };
}


/** Abre o menu "⋯" da linha e escolhe a ação (as ações por linha vivem no `SofaMenu`). */
async function escolherNoMenu(
    usuario: ReturnType<typeof userEvent.setup>,
    linha: string,
    acao: string,
) {
    await usuario.click(screen.getByRole("button", { name: linha }));
    await usuario.click(await screen.findByRole("menuitem", { name: acao }));
}

describe("AgenteConhecimento", () => {
    it("carrega grupos e trechos e marca o grupo usado pelo agente", async () => {
        servidorCom(cenarioPadrao());
        montar();

        expect(await screen.findByText("Políticas da loja")).toBeInTheDocument();
        expect(screen.getByText("Perguntas frequentes")).toBeInTheDocument();
        // O grupo vinculado é o aberto por padrão e o trecho dele aparece.
        expect(await screen.findByText("Prazo de entrega")).toBeInTheDocument();
        expect(screen.getByText("usado por este agente")).toBeInTheDocument();
        expect(screen.getByText("sem vínculo")).toBeInTheDocument();
        // O estado do índice começa honesto: a API não expõe a rota.
        expect(screen.getByText("Índice não verificado")).toBeInTheDocument();
    });

    it("mostra grupos e trechos em tabelas contínuas, sem cartão", async () => {
        servidorCom(cenarioPadrao());
        montar();

        await screen.findByText("Políticas da loja");

        const grupos = screen.getByRole("table", { name: "Grupos da base" });
        expect(within(grupos).getByRole("columnheader", { name: "Grupo" })).toBeInTheDocument();
        expect(within(grupos).getByRole("columnheader", { name: "Vínculo" })).toBeInTheDocument();
        // Cada linha tem o menu "⋯" da RL-05 no lugar dos botões soltos.
        expect(
            within(grupos).getByRole("button", { name: "Ações de Políticas da loja" }),
        ).toBeInTheDocument();
        // Os trechos chegam numa consulta própria, depois dos grupos: espera a tabela aparecer.
        expect(await screen.findByRole("table", { name: "Trechos do grupo" })).toBeInTheDocument();
        // Sem cartão em nenhum ponto da tela.
        expect(document.querySelector(".sofa-card")).toBeNull();
        expect(document.querySelector(".sofa-content-shell")).toBeNull();
    });

    it("trocar o grupo da lista troca os trechos mostrados", async () => {
        const usuario = userEvent.setup();
        servidorCom(cenarioPadrao());
        montar();

        await screen.findByText("Prazo de entrega");
        expect(screen.getByText("Trechos · Políticas da loja")).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Perguntas frequentes" }));

        expect(await screen.findByText("Grupo sem trechos")).toBeInTheDocument();
        expect(screen.getByText("Trechos · Perguntas frequentes")).toBeInTheDocument();
    });

    it("seleciona o grupo pelo teclado e abre o menu da linha pelo teclado", async () => {
        const usuario = userEvent.setup();
        servidorCom(cenarioPadrao());
        montar();

        await screen.findByText("Prazo de entrega");
        const nome = screen.getByRole("button", { name: "Perguntas frequentes" });
        nome.focus();
        expect(nome).toHaveFocus();
        await usuario.keyboard("{Enter}");
        expect(await screen.findByText("Grupo sem trechos")).toBeInTheDocument();

        const gatilho = screen.getByRole("button", { name: "Ações de Políticas da loja" });
        gatilho.focus();
        await usuario.keyboard("{Enter}");
        const menu = await screen.findByRole("menu");
        expect(within(menu).getByRole("menuitem", { name: "Editar" })).toBeInTheDocument();
        expect(within(menu).getByRole("menuitem", { name: "Desvincular" })).toBeInTheDocument();
        expect(within(menu).getByRole("menuitem", { name: "Excluir" })).toBeInTheDocument();
    });

    it("cria grupo enviando o nome e o corpo que a API espera", async () => {
        const usuario = userEvent.setup();
        const cenario = cenarioPadrao();
        const servidor = servidorCom(cenario);
        montar();

        await screen.findByText("Políticas da loja");
        await usuario.click(screen.getByRole("button", { name: "Novo grupo" }));
        await waitForModalFocus();
        await usuario.type(screen.getByRole("textbox", { name: /^Nome/ }), "Horários");
        await usuario.click(screen.getByRole("button", { name: "Criar grupo" }));

        await waitFor(() => expect(screen.getByText("Horários")).toBeInTheDocument());
        const [pedido] = servidor.chamadasPara("POST", `/admin/v1/orgs/${ORG}/knowledge`);
        expect(pedido?.corpo).toEqual({ name: "Horários" });
    });

    it("cria trecho no grupo aberto, com o contador de caracteres e as tags em lista", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorCom(cenarioPadrao());
        montar();

        await screen.findByText("Prazo de entrega");
        await usuario.click(screen.getByRole("button", { name: "Novo trecho" }));
        await waitForModalFocus();
        await usuario.type(
            screen.getByRole("textbox", { name: /^Título/ }),
            "Frete grátis",
        );
        await usuario.type(
            screen.getByRole("textbox", { name: /^Conteúdo/ }),
            "Acima de R$ 199,00 o frete é grátis.",
        );
        await usuario.type(screen.getByRole("textbox", { name: /^Tags/ }), "frete, preço, frete");
        expect(screen.getByText(/36 \/ 1000 caracteres/)).toBeInTheDocument();
        await usuario.click(screen.getByRole("button", { name: "Criar trecho" }));

        await waitFor(() => expect(screen.getByText("Frete grátis")).toBeInTheDocument());
        const [pedido] = servidor.chamadasPara(
            "POST",
            `/admin/v1/orgs/${ORG}/knowledge/grupo-1/items`,
        );
        expect(pedido?.corpo).toEqual({
            title: "Frete grátis",
            content: "Acima de R$ 199,00 o frete é grátis.",
            tags: ["frete", "preço"],
            position: 0,
        });
    });

    it("valida o trecho antes de chamar a API e põe o erro no campo", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorCom(cenarioPadrao());
        montar();

        await screen.findByText("Prazo de entrega");
        await usuario.click(screen.getByRole("button", { name: "Novo trecho" }));
        await waitForModalFocus();
        await usuario.click(screen.getByRole("button", { name: "Criar trecho" }));

        expect(await screen.findByText("informe o título do trecho")).toBeInTheDocument();
        expect(screen.getByText("informe o conteúdo do trecho")).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", "/items")).toHaveLength(0);
    });

    it("mostra no campo o nome repetido que a API devolve", async () => {
        const usuario = userEvent.setup();
        const cenario: Cenario = {
            ...cenarioPadrao(),
            erroAoCriarGrupo: {
                status: 400,
                corpo: {
                    code: "validation_error",
                    message: "nome já existe",
                    details: { field: "name", reason: "já existe um grupo com este nome" },
                },
            },
        };
        servidorCom(cenario);
        montar();

        await screen.findByText("Políticas da loja");
        await usuario.click(screen.getByRole("button", { name: "Novo grupo" }));
        await waitForModalFocus();
        await usuario.type(screen.getByRole("textbox", { name: /^Nome/ }), "Políticas da loja");
        await usuario.click(screen.getByRole("button", { name: "Criar grupo" }));

        expect(
            await screen.findByText("já existe um grupo com este nome"),
        ).toBeInTheDocument();
        // O formulário continua aberto e com o texto digitado.
        expect(screen.getByDisplayValue("Políticas da loja")).toBeInTheDocument();
    });

    it("exclui o grupo só depois de digitar o nome e manda force=true", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorCom(cenarioPadrao());
        montar();

        await screen.findByText("Políticas da loja");
        await escolherNoMenu(usuario, "Ações de Políticas da loja", "Excluir");
        await waitForModalFocus();

        const dialogo = await screen.findByRole("dialog");
        // O diálogo conta os trechos numa consulta própria: enquanto ela não volta, mostra skeleton.
        expect(await within(dialogo).findByText(/O grupo tem 1 trecho/)).toBeInTheDocument();
        expect(await within(dialogo).findByText(/agente usa este grupo/)).toBeInTheDocument();

        const botao = within(dialogo).getByRole("button", { name: "Excluir grupo" });
        expect(botao).toBeDisabled();
        await usuario.type(
            within(dialogo).getByRole("textbox", { name: /Digite/ }),
            "Políticas da loja",
        );
        await usuario.click(botao);

        await waitFor(() =>
            expect(servidor.chamadasPara("DELETE", "/knowledge/grupo-1")).toHaveLength(1),
        );
        const [pedido] = servidor.chamadasPara("DELETE", "/knowledge/grupo-1");
        expect(pedido?.url).toContain("force=true");
    });

    it("desvincula o grupo do agente pela linha da lista", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorCom(cenarioPadrao());
        montar();

        await screen.findByText("Políticas da loja");
        await escolherNoMenu(usuario, "Ações de Políticas da loja", "Desvincular");

        await waitFor(() =>
            expect(
                servidor.chamadasPara("DELETE", `/agents/${AGENTE}/knowledge/grupo-1`),
            ).toHaveLength(1),
        );
    });

    it("no 412 do grupo relê, adota a versão nova e mantém o nome digitado", async () => {
        const usuario = userEvent.setup();
        const cenario = cenarioPadrao();
        cenario.grupos[0]!.version = 2;
        const servidor = servidorCom(cenario);
        servidor.responder("PATCH", "/knowledge/grupo-1", () => {
            // Outra pessoa salvou primeiro: a próxima leitura já traz a versão 3.
            cenario.grupos[0]!.version = 3;
            return apiError(412, "version_conflict", "versão desatualizada", {
                expected_version: 2,
                current_version: 3,
            });
        });
        montar();

        await screen.findByText("Políticas da loja");
        await escolherNoMenu(usuario, "Ações de Políticas da loja", "Editar");
        await waitForModalFocus();
        const campo = screen.getByRole("textbox", { name: /^Nome/ });
        await usuario.clear(campo);
        await usuario.type(campo, "Políticas novas");
        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(
            await screen.findByText(/Outra pessoa salvou este grupo antes de você \(agora na versão 3\)/),
        ).toBeInTheDocument();
        // O rascunho continua na tela e a próxima gravação usa a versão atual.
        expect(screen.getByDisplayValue("Políticas novas")).toBeInTheDocument();
        const [pedido] = servidor.chamadasPara("PATCH", "/knowledge/grupo-1");
        expect(pedido?.cabecalhos["if-match"]).toBe("2");
    });

    it("no 412 do trecho mantém o texto digitado e passa a usar a versão nova", async () => {
        const usuario = userEvent.setup();
        const cenario = cenarioPadrao();
        const servidor = servidorCom(cenario);
        servidor.responder("PATCH", "/items/item-1", () => {
            cenario.itens["grupo-1"] = [
                { ...cenario.itens["grupo-1"]![0]!, version: 2 },
            ];
            return apiError(412, "version_conflict", "versão desatualizada", {
                expected_version: 1,
                current_version: 2,
            });
        });
        montar();

        await screen.findByText("Prazo de entrega");
        await escolherNoMenu(usuario, "Ações de Prazo de entrega", "Editar");
        await waitForModalFocus();
        const conteudo = screen.getByRole("textbox", { name: /^Conteúdo/ });
        await usuario.clear(conteudo);
        await usuario.type(conteudo, "O prazo mudou: 10 dias úteis.");
        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(
            await screen.findByText(/Outra pessoa salvou este trecho antes de você \(agora na versão 2\)/),
        ).toBeInTheDocument();
        expect(screen.getByDisplayValue("O prazo mudou: 10 dias úteis.")).toBeInTheDocument();
    });

    it("exclui trecho só depois da confirmação pelo título", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorCom(cenarioPadrao());
        servidor.responder("DELETE", "/items/item-1", { status: 200, corpo: { removido: true } });
        montar();

        await screen.findByText("Prazo de entrega");
        await escolherNoMenu(usuario, "Ações de Prazo de entrega", "Excluir");
        await waitForModalFocus();

        const dialogo = await screen.findByRole("dialog");
        const botao = within(dialogo).getByRole("button", { name: "Excluir trecho" });
        expect(botao).toBeDisabled();
        await usuario.type(
            within(dialogo).getByRole("textbox", { name: /Digite/ }),
            "Prazo de entrega",
        );
        await usuario.click(botao);

        await waitFor(() =>
            expect(servidor.chamadasPara("DELETE", "/items/item-1")).toHaveLength(1),
        );
        await waitFor(() => expect(screen.queryByText("Prazo de entrega")).not.toBeInTheDocument());
    });

    it("mostra o vazio do grupo sem trechos com o CTA de criar", async () => {
        const cenario = cenarioPadrao();
        cenario.itens["grupo-1"] = [];
        servidorCom(cenario);
        montar();

        expect(await screen.findByText("Grupo sem trechos")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Criar primeiro trecho" })).toBeInTheDocument();
    });

    it("mostra o vazio de organização sem grupos com o CTA de criar o primeiro", async () => {
        const cenario = cenarioPadrao();
        cenario.grupos = [];
        cenario.itens = {};
        servidorCom(cenario);
        montar();

        expect(await screen.findByText("Nenhum grupo de conhecimento")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Criar primeiro grupo" })).toBeInTheDocument();
    });

    it("mostra o erro de carga dos trechos com tentar de novo", async () => {
        const usuario = userEvent.setup();
        const cenario = cenarioPadrao();
        let falhar = true;
        // Rota registrada antes da de sucesso: o servidor falso casa a primeira que bater.
        new FakeServer()
            .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: AGENTE_JSON })
            .responder("GET", `/admin/v1/orgs/${ORG}/agents`, {
                status: 200,
                corpo: { items: [AGENTE_JSON], next_cursor: null },
            })
            .responder("GET", "/items", () =>
                falhar
                    ? apiError(500, "internal", "falha inesperada")
                    : {
                          status: 200,
                          corpo: {
                              items: cenario.itens["grupo-1"] ?? [],
                              next_cursor: null,
                          },
                      },
            )
            .responder("GET", "/knowledge/grupo-", {
                status: 200,
                corpo: cenario.grupos[0],
            })
            .responder("GET", `/admin/v1/orgs/${ORG}/knowledge`, {
                status: 200,
                corpo: { items: cenario.grupos, next_cursor: null },
            })
            .responder("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo Odontologia" },
            })
            .instalar();
        montar();

        expect(await screen.findByText("Não consegui carregar os trechos")).toBeInTheDocument();
        falhar = false;
        await usuario.click(screen.getByRole("button", { name: "Tentar de novo" }));
        expect(await screen.findByText("Prazo de entrega")).toBeInTheDocument();
    });

    it("reindexa o agente e mostra o relatório do índice", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorCom(cenarioPadrao());
        montar();

        await screen.findByText("Índice não verificado");
        await usuario.click(screen.getByRole("button", { name: "Reindexar agente" }));

        expect(await screen.findByText("Índice atualizado")).toBeInTheDocument();
        expect(await screen.findByText(/2 novos/)).toBeInTheDocument();
        expect(screen.getByText(/3 embeddings gerados/)).toBeInTheDocument();
        expect(
            servidor.chamadasPara("POST", `/agents/${AGENTE}/knowledge/reindex`),
        ).toHaveLength(1);
    });

    it("explica quando o modelo de embedding não está instalado", async () => {
        const usuario = userEvent.setup();
        const servidor = new FakeServer()
            .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: AGENTE_JSON })
            .responder("GET", `/admin/v1/orgs/${ORG}/agents`, {
                status: 200,
                corpo: { items: [AGENTE_JSON], next_cursor: null },
            })
            .responder("GET", "/items", { status: 200, corpo: { items: [], next_cursor: null } })
            .responder("GET", `/knowledge/grupo-1`, {
                status: 200,
                corpo: grupo({ id: "grupo-1", name: "Políticas da loja", agents: [AGENTE] }),
            })
            .responder("GET", `/admin/v1/orgs/${ORG}/knowledge`, {
                status: 200,
                corpo: {
                    items: [grupo({ id: "grupo-1", name: "Políticas da loja", agents: [AGENTE] })],
                    next_cursor: null,
                },
            })
            .responder("POST", `/agents/${AGENTE}/knowledge/reindex`, () =>
                apiError(503, "embedding_unavailable", "modelo de embedding indisponível; instale o modelo"),
            )
            .responder("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo Odontologia" },
            })
            .instalar();
        montar();

        await screen.findByText("Políticas da loja");
        await usuario.click(screen.getByRole("button", { name: "Reindexar agente" }));

        expect(
            await screen.findByText(/O índice não mudou\. Instale o modelo de embedding/),
        ).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", "/reindex")).toHaveLength(1);
    });
});

describe("I18N-08: Base de conhecimento em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("mostra grupos e trechos em inglês, com vínculo, situação e detalhes", async () => {
        servidorCom(cenarioPadrao());
        setLocale("en-US");
        montar();

        expect(await screen.findByText("Políticas da loja")).toBeInTheDocument();
        expect(screen.getByText("Index not verified")).toBeInTheDocument();
        expect(screen.getByText("used by this agent")).toBeInTheDocument();
        expect(screen.getByText("not linked")).toBeInTheDocument();

        const grupos = screen.getByRole("table", { name: "Knowledge groups" });
        expect(within(grupos).getByRole("columnheader", { name: "Group" })).toBeInTheDocument();
        expect(within(grupos).getByRole("columnheader", { name: "Link" })).toBeInTheDocument();
        expect(within(grupos).getByRole("columnheader", { name: "Status" })).toBeInTheDocument();
        expect(within(grupos).getByRole("columnheader", { name: "Version" })).toBeInTheDocument();
        expect(within(grupos).getByRole("columnheader", { name: "Actions" })).toBeInTheDocument();
        expect(
            within(grupos).getByRole("button", { name: "Actions for Políticas da loja" }),
        ).toBeInTheDocument();
        expect(screen.getAllByText("Active").length).toBeGreaterThan(0);

        const trechos = await screen.findByRole("table", { name: "Group entries" });
        expect(within(trechos).getByRole("columnheader", { name: "Entry" })).toBeInTheDocument();
        expect(within(trechos).getByRole("columnheader", { name: "Details" })).toBeInTheDocument();
        expect(screen.getByText("Entries · Políticas da loja")).toBeInTheDocument();
        expect(screen.getByText("1 entry")).toBeInTheDocument();
        expect(screen.getByText(/characters · order 0 · v1/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "New entry" })).toBeInTheDocument();
    });

    it("reindexa em inglês e mostra o relatório do índice", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorCom(cenarioPadrao());
        setLocale("en-US");
        montar();

        await screen.findByText("Index not verified");
        await usuario.click(screen.getByRole("button", { name: "Reindex agent" }));

        expect(await screen.findByText("Index updated")).toBeInTheDocument();
        expect(await screen.findByText(/2 new/)).toBeInTheDocument();
        expect(screen.getByText(/3 embeddings generated/)).toBeInTheDocument();
        expect(
            servidor.chamadasPara("POST", `/agents/${AGENTE}/knowledge/reindex`),
        ).toHaveLength(1);
    });

    it("valida o trecho em inglês antes de chamar a API", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorCom(cenarioPadrao());
        setLocale("en-US");
        montar();

        await screen.findByText("Prazo de entrega");
        await usuario.click(screen.getByRole("button", { name: "New entry" }));
        await waitForModalFocus();
        await usuario.click(screen.getByRole("button", { name: "Create entry" }));

        expect(await screen.findByText("enter the entry title")).toBeInTheDocument();
        expect(screen.getByText("enter the entry content")).toBeInTheDocument();
        expect(screen.getByText(/0 \/ 1000 characters/)).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", "/items")).toHaveLength(0);
    });

    it("exclui o grupo em inglês com a consequência e a confirmação pelo nome", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorCom(cenarioPadrao());
        setLocale("en-US");
        montar();

        await screen.findByText("Políticas da loja");
        await usuario.click(
            screen.getByRole("button", { name: "Actions for Políticas da loja" }),
        );
        await usuario.click(await screen.findByRole("menuitem", { name: "Delete" }));
        await waitForModalFocus();

        const dialogo = await screen.findByRole("dialog");
        expect(within(dialogo).getByText("Delete group?")).toBeInTheDocument();
        expect(await within(dialogo).findByText(/The group has 1 entry/)).toBeInTheDocument();
        expect(
            await within(dialogo).findByText(/1 agent uses this group and will be left without it: Beatriz/),
        ).toBeInTheDocument();

        const botao = within(dialogo).getByRole("button", { name: "Delete group" });
        expect(botao).toBeDisabled();
        await usuario.type(within(dialogo).getByRole("textbox"), "Políticas da loja");
        await usuario.click(botao);

        await waitFor(() =>
            expect(servidor.chamadasPara("DELETE", "/knowledge/grupo-1")).toHaveLength(1),
        );
        expect(servidor.chamadasPara("DELETE", "/knowledge/grupo-1")[0]?.url).toContain(
            "force=true",
        );
    });

    it("abre o formulário do trecho em inglês com as dicas do idioma", async () => {
        const usuario = userEvent.setup();
        servidorCom(cenarioPadrao());
        setLocale("en-US");
        montar();

        await screen.findByText("Prazo de entrega");
        await usuario.click(screen.getByRole("button", { name: "New entry" }));
        await waitForModalFocus();

        const dialogo = await screen.findByRole("dialog");
        expect(within(dialogo).getByText("New entry")).toBeInTheDocument();
        expect(within(dialogo).getByText("Content")).toBeInTheDocument();
        expect(
            within(dialogo).getByText(/This is the text the agent receives in the query/),
        ).toBeInTheDocument();
        expect(within(dialogo).getByText("Order")).toBeInTheDocument();
        expect(within(dialogo).getByText(/lower appears first/)).toBeInTheDocument();
    });
});

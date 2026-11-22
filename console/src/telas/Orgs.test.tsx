/**
 * Testes da lista de organizações no layout D1 (RL-05).
 *
 * O que a tela oferece depende do escopo do token: token de org não recebe as ações de gestão
 * (criar, editar, reativar, excluir — o servidor recusaria com 403); token global recebe.
 *
 * O filtro (abas e texto) é local: nenhuma chamada nova à API por aba ou por caractere. A
 * reativação é o único fluxo que escreve nesta tela além do modal e da exclusão, e ela vai por
 * `PATCH` com `If-Match` (por isso o teste confere método, cabeçalho e corpo).
 */

import { useEffect } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import type { Agente } from "../api/agentes";
import type { Org } from "../api/cliente";
import { GuardaAlteracoesProvider, useGuardaDeAlteracoes } from "../componentes/GuardaAlteracoes";
import { SessaoProvider } from "../sessao/Sessao";
import { ServidorFalso } from "../testes/servidor";
import { Orgs } from "./Orgs";

function org(parcial: Partial<Org> & Pick<Org, "id" | "name" | "slug">): Org {
    return {
        status: "active",
        version: 3,
        config: { retention_days: 30 } as unknown as Org["config"],
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...parcial,
    };
}

const ATIVA = org({ id: "org_1", name: "Loja Antiga", slug: "loja-antiga" });

const INATIVA = org({
    id: "org_2",
    name: "Patinhas Pet Shop",
    slug: "org_patinhas",
    status: "inactive",
    version: 7,
});

const ARQUIVADA = org({
    id: "org_3",
    name: "Demonstração Legada",
    slug: "org_demo_arquivada",
    status: "archived",
    version: 1,
    config: {} as unknown as Org["config"],
});

function prepararSessao(token: string, escopo: "global" | "org", items: Org[]) {
    window.sessionStorage.setItem(
        "poph-console.sessao",
        JSON.stringify({ token, escopo, orgs: items }),
    );
}

/** Mostra o caminho atual num elemento, para o teste conferir navegação. */
function Destino() {
    const local = useLocation();
    return <output data-testid="caminho">{local.pathname}</output>;
}

/** Registra uma edição pendente na guarda: é o que segura a navegação da gaveta no teste. */
function EdicaoPendente() {
    const guarda = useGuardaDeAlteracoes();
    useEffect(() => {
        guarda.registrar({ sujo: true, salvar: async () => true });
    }, [guarda]);
    return null;
}

function montar(inicial = "/orgs", opcoes: { sujo?: boolean } = {}) {
    return render(
        <MemoryRouter initialEntries={[inicial]}>
            <SessaoProvider>
                <GuardaAlteracoesProvider>
                    {opcoes.sujo ? <EdicaoPendente /> : null}
                    <Routes>
                        <Route path="/orgs" element={<Orgs />} />
                        <Route path="/orgs/:orgId" element={<Destino />} />
                    </Routes>
                </GuardaAlteracoesProvider>
            </SessaoProvider>
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
    default_department_id: null,
    human_handoff_department_id: null,
    version: 1,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

/** Servidor com a lista e a consulta de agentes que a gaveta faz ao abrir. */
function servidorCom(items: Org[]) {
    const servidor = new ServidorFalso();
    // A rota mais específica vem primeiro: o roteador falso casa por `includes`, em ordem.
    servidor
        .responder("GET", "/agents", () => ({
            status: 200,
            corpo: { items: [AGENTE], next_cursor: null },
        }))
        .responder("GET", "/admin/v1/orgs", { status: 200, corpo: { items, next_cursor: null } })
        .instalar();
    return servidor;
}

/** Abre o menu "⋯" da linha cujo nome começa com `nome`. */
async function abrirMenu(usuario: ReturnType<typeof userEvent.setup>, nome: string) {
    const gatilho = screen.getByRole("button", { name: `Ações de ${nome}` });
    await usuario.click(gatilho);
    return screen.getByRole("menu");
}

describe("Orgs", () => {
    it("token global vê as ações de gestão e abre o formulário de criação", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA]);
        servidorCom([ATIVA]);

        montar();

        expect(await screen.findByText("Loja Antiga")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Abrir" })).toBeInTheDocument();

        const gatilho = screen.getByRole("button", { name: "Ações de Loja Antiga" });
        expect(gatilho).toHaveAttribute("aria-expanded", "false");
        const menu = await abrirMenu(usuario, "Loja Antiga");
        // O menu é ancorado no gatilho da linha (o cabeçalho de acessibilidade vem dele).
        expect(gatilho).toHaveAttribute("aria-expanded", "true");
        expect(menu).toHaveAccessibleName("Ações de Loja Antiga");
        expect(within(menu).getByRole("menuitem", { name: "Editar" })).toBeInTheDocument();
        expect(within(menu).getByRole("menuitem", { name: "Excluir" })).toBeInTheDocument();
        // Organização ativa não oferece "Reativar".
        expect(within(menu).queryByRole("menuitem", { name: "Reativar" })).not.toBeInTheDocument();
        await usuario.keyboard("{Escape}");

        await usuario.click(screen.getByRole("button", { name: "Nova organização" }));
        expect(await screen.findByRole("dialog")).toHaveAccessibleName("Nova organização");
    });

    it("token de org não vê criação, edição, reativação nem exclusão", async () => {
        prepararSessao("poph_org_teste", "org", [INATIVA]);
        servidorCom([INATIVA]);

        montar();

        expect(await screen.findByText("Patinhas Pet Shop")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Abrir" })).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Nova organização" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Ações de Patinhas Pet Shop" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Editar" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Excluir" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Reativar" })).not.toBeInTheDocument();
    });

    it("filtra por aba e por texto, com as contagens corretas", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA, INATIVA, ARQUIVADA]);
        servidorCom([ATIVA, INATIVA, ARQUIVADA]);

        montar();

        expect(await screen.findByText("Loja Antiga")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /^Todas/ })).toHaveTextContent("Todas(3)");
        expect(screen.getByRole("button", { name: /^Ativas/ })).toHaveTextContent("Ativas(1)");
        expect(screen.getByRole("button", { name: /^Inativas/ })).toHaveTextContent("Inativas(1)");
        expect(screen.getByRole("button", { name: /^Arquivadas/ })).toHaveTextContent("Arquivadas(1)");

        // Aba Inativas: só a inativa.
        await usuario.click(screen.getByRole("button", { name: /^Inativas/ }));
        expect(screen.getByText("Patinhas Pet Shop")).toBeInTheDocument();
        expect(screen.queryByText("Loja Antiga")).not.toBeInTheDocument();
        expect(screen.queryByText("Demonstração Legada")).not.toBeInTheDocument();
        expect(await screen.findByText("Exibindo 1 de 3 organizações")).toBeInTheDocument();

        // Volta a Todas e filtra pelo slug, sem acento e sem caixa.
        await usuario.click(screen.getByRole("button", { name: /^Todas/ }));
        await usuario.type(screen.getByRole("searchbox"), "DEMO");
        expect(screen.getByText("Demonstração Legada")).toBeInTheDocument();
        expect(screen.queryByText("Loja Antiga")).not.toBeInTheDocument();
        expect(await screen.findByText("Exibindo 1 de 3 organizações")).toBeInTheDocument();

        // Texto pelo nome sem acento encontra a org mesmo sem o acento.
        await usuario.clear(screen.getByRole("searchbox"));
        await usuario.type(screen.getByRole("searchbox"), "patinhas");
        expect(screen.getByText("Patinhas Pet Shop")).toBeInTheDocument();

        // Sem resultado: mensagem e ação de limpar.
        await usuario.clear(screen.getByRole("searchbox"));
        await usuario.type(screen.getByRole("searchbox"), "inexistente");
        expect(await screen.findByText("Nenhuma organização encontrada")).toBeInTheDocument();
        await usuario.click(screen.getByRole("button", { name: "Limpar filtros" }));
        expect(await screen.findByText("Loja Antiga")).toBeInTheDocument();
    });

    it("esconde a aba de arquivadas quando não há organização arquivada", async () => {
        prepararSessao("pophg_teste", "global", [ATIVA, INATIVA]);
        servidorCom([ATIVA, INATIVA]);

        montar();

        expect(await screen.findByText("Loja Antiga")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: /^Arquivadas/ })).not.toBeInTheDocument();
    });

    it("Abrir navega para o painel da organização", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA]);
        servidorCom([ATIVA]);

        montar();

        await usuario.click(await screen.findByRole("button", { name: "Abrir" }));
        expect(await screen.findByTestId("caminho")).toHaveTextContent("/orgs/org_1");
    });

    it("o nome da organização é link para o painel", async () => {
        prepararSessao("pophg_teste", "global", [ATIVA]);
        servidorCom([ATIVA]);

        montar();

        const link = await screen.findByRole("link", { name: "Loja Antiga" });
        expect(link).toHaveAttribute("href", "/orgs/org_1");
    });

    it("Editar abre o formulário com a organização escolhida", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA]);
        servidorCom([ATIVA]);

        montar();

        const menu = await abrirMenu(usuario, "Loja Antiga");
        await usuario.click(within(menu).getByRole("menuitem", { name: "Editar" }));
        expect(await screen.findByRole("dialog")).toHaveAccessibleName("Editar organização");
    });

    it("Excluir abre a confirmação e manda o DELETE do slug digitado", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA]);
        const servidor = servidorCom([ATIVA]);
        servidor.responder("DELETE", "/admin/v1/orgs/org_1", { status: 204 });

        montar();

        const menu = await abrirMenu(usuario, "Loja Antiga");
        await usuario.click(within(menu).getByRole("menuitem", { name: "Excluir" }));
        const dialogo = await screen.findByRole("dialog");
        expect(dialogo).toHaveAccessibleName("Excluir organização?");

        await usuario.type(within(dialogo).getByRole("textbox"), "loja-antiga");
        await usuario.click(within(dialogo).getByRole("button", { name: "Excluir organização" }));

        await waitFor(() => expect(servidor.chamadasPara("DELETE", "/admin/v1/orgs/org_1")).toHaveLength(1));
    });

    it("Reativar manda PATCH com status active e If-Match, e a linha sai de Inativas", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA, INATIVA]);
        let reativada = false;
        const servidor = new ServidorFalso();
        // A lista reflete o PATCH: depois de reativar, a org volta ativa (como a instância faria).
        servidor
            .responder("GET", "/admin/v1/orgs", () => ({
                status: 200,
                corpo: {
                    items: reativada
                        ? [ATIVA, { ...INATIVA, status: "active", version: 8 }]
                        : [ATIVA, INATIVA],
                    next_cursor: null,
                },
            }))
            .responder("PATCH", "/admin/v1/orgs/org_2", () => {
                reativada = true;
                return { status: 200, corpo: { ...INATIVA, status: "active", version: 8 } };
            })
            .instalar();

        montar();

        await usuario.click(await screen.findByRole("button", { name: /^Inativas/ }));
        expect(await screen.findByText("Patinhas Pet Shop")).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Ações de Patinhas Pet Shop" }));
        await usuario.click(await screen.findByRole("menuitem", { name: "Reativar" }));

        await waitFor(() =>
            expect(servidor.chamadasPara("PATCH", "/admin/v1/orgs/org_2")).toHaveLength(1),
        );
        const chamada = servidor.chamadasPara("PATCH", "/admin/v1/orgs/org_2")[0]!;
        expect(chamada.corpo).toEqual({ status: "active" });
        expect(chamada.cabecalhos["if-match"]).toBe("7");

        // Saiu da aba Inativas (ficou vazia, com a contagem nova).
        await waitFor(() =>
            expect(screen.getByRole("button", { name: /^Inativas/ })).toHaveTextContent("Inativas(0)"),
        );
        expect(screen.queryByText("Patinhas Pet Shop")).not.toBeInTheDocument();
    });

    it("Reativar com 412 mostra o conflito e relê a lista", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [INATIVA]);
        const servidor = servidorCom([INATIVA]);
        servidor.responder("PATCH", "/admin/v1/orgs/org_2", {
            status: 412,
            corpo: {
                code: "version_conflict",
                message: "a versão mudou",
                details: { expected_version: 7, current_version: 9 },
            },
        });

        montar();

        await usuario.click(await screen.findByRole("button", { name: /^Todas/ }));
        await usuario.click(await screen.findByRole("button", { name: "Ações de Patinhas Pet Shop" }));
        await usuario.click(await screen.findByRole("menuitem", { name: "Reativar" }));

        expect(await screen.findByText("Versão desatualizada")).toBeInTheDocument();
        expect(screen.getByRole("alert")).toHaveTextContent("agora na versão 9");
    });

    it("estado vazio de token global oferece criar a primeira organização", async () => {
        prepararSessao("pophg_teste", "global", []);
        servidorCom([]);

        montar();

        expect(await screen.findByText("Nenhuma organização cadastrada")).toBeInTheDocument();
        expect(
            screen.getByRole("button", { name: "Criar primeira organização" }),
        ).toBeInTheDocument();
    });

    it("mostra o estado de erro com tentativa de novo quando a API falha", async () => {
        prepararSessao("pophg_teste", "global", []);
        new ServidorFalso()
            .responder("GET", "/admin/v1/orgs", {
                status: 500,
                corpo: { code: "internal", message: "falha inesperada", details: null },
            })
            .instalar();
        vi.spyOn(console, "error").mockImplementation(() => undefined);

        montar();

        expect(
            await screen.findByText("Não consegui carregar as organizações"),
        ).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
        await waitFor(() => expect(screen.getByText("falha inesperada")).toBeInTheDocument());
    });

    it("teclado: Tab chega ao nome e ao menu da linha, e Enter no Abrir navega", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA]);
        servidorCom([ATIVA]);

        montar();

        const link = await screen.findByRole("link", { name: "Loja Antiga" });
        const gatilho = screen.getByRole("button", { name: "Ações de Loja Antiga" });
        const abrir = screen.getByRole("button", { name: "Abrir" });

        // Tab percorre os controles da página até a linha (nome) e o menu dela.
        const alcancados: HTMLElement[] = [];
        for (let passo = 0; passo < 20; passo += 1) {
            await usuario.tab();
            const ativo = document.activeElement;
            if (ativo instanceof HTMLElement) alcancados.push(ativo);
            if (alcancados.includes(link) && alcancados.includes(gatilho)) break;
        }
        expect(alcancados).toContain(link);
        expect(alcancados).toContain(gatilho);

        abrir.focus();
        await usuario.keyboard("{Enter}");
        expect(await screen.findByTestId("caminho")).toHaveTextContent("/orgs/org_1");
    });

    // ── Inspetor lateral (RL-06) ───────────────────────────────────────────

    it("selecionar a linha abre o inspetor com os dados daquela organização", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA]);
        servidorCom([ATIVA]);

        montar();

        // A célula de retenção não é interativa: o clique é o da linha.
        await usuario.click(await screen.findByText("30 dias"));

        const gaveta = await screen.findByRole("complementary", {
            name: "Detalhes de Loja Antiga",
        });
        expect(within(gaveta).getByText("loja-antiga")).toBeInTheDocument();
        expect(within(gaveta).getByText("org_1")).toBeInTheDocument();
        expect(within(gaveta).getByText("v3")).toBeInTheDocument();
        expect(within(gaveta).getByText("30 dias")).toBeInTheDocument();
        expect(within(gaveta).getByText("Ativa")).toBeInTheDocument();

        // O botão Abrir da linha continua na tela: a gaveta encolhe a tabela, não a cobre.
        expect(screen.getByRole("button", { name: "Abrir" })).toBeInTheDocument();

        // Recursos: a consulta de agentes alimenta a linha de Agentes.
        expect(await within(gaveta).findByText("1 agente · Atendente Demo")).toBeInTheDocument();
    });

    it("trocar a seleção atualiza o inspetor para a outra organização", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA, INATIVA]);
        servidorCom([ATIVA, INATIVA]);

        montar();

        await usuario.click(await screen.findByText("loja-antiga"));
        expect(
            await screen.findByRole("complementary", { name: "Detalhes de Loja Antiga" }),
        ).toBeInTheDocument();

        await usuario.click(screen.getByText("org_patinhas"));
        expect(
            await screen.findByRole("complementary", { name: "Detalhes de Patinhas Pet Shop" }),
        ).toBeInTheDocument();
        expect(
            screen.queryByRole("complementary", { name: "Detalhes de Loja Antiga" }),
        ).not.toBeInTheDocument();
    });

    it("Esc fecha o inspetor e devolve o foco ao nome da linha", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA]);
        servidorCom([ATIVA]);

        montar();

        await usuario.click(await screen.findByText("loja-antiga"));
        expect(await screen.findByRole("complementary")).toBeInTheDocument();

        await usuario.keyboard("{Escape}");

        await waitFor(() =>
            expect(screen.queryByRole("complementary")).not.toBeInTheDocument(),
        );
        expect(document.activeElement).toBe(screen.getByRole("link", { name: "Loja Antiga" }));
    });

    it("o ✕ também fecha o inspetor", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA]);
        servidorCom([ATIVA]);

        montar();

        await usuario.click(await screen.findByText("loja-antiga"));
        const gaveta = await screen.findByRole("complementary");

        await usuario.click(within(gaveta).getByRole("button", { name: "Fechar inspetor" }));
        await waitFor(() =>
            expect(screen.queryByRole("complementary")).not.toBeInTheDocument(),
        );
    });

    it("falha na consulta de agentes aparece só na linha de Agentes", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA]);
        const servidor = new ServidorFalso();
        servidor
            .responder("GET", "/agents", {
                status: 500,
                corpo: { code: "internal", message: "falha ao listar agentes", details: null },
            })
            .responder("GET", "/admin/v1/orgs", {
                status: 200,
                corpo: { items: [ATIVA], next_cursor: null },
            })
            .instalar();

        montar();

        await usuario.click(await screen.findByText("loja-antiga"));
        const gaveta = await screen.findByRole("complementary");

        expect(await within(gaveta).findByText("não consegui carregar")).toBeInTheDocument();
        // O resto da gaveta continua inteiro.
        expect(within(gaveta).getByText("Propriedades do tenant")).toBeInTheDocument();
        expect(within(gaveta).getByText("org_1")).toBeInTheDocument();
        expect(within(gaveta).getByText(/para fechar o inspetor/)).toBeInTheDocument();
    });

    it("copiar põe o comando no clipboard e anuncia Copiado", async () => {
        const usuario = userEvent.setup();
        const escrever = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(navigator, "clipboard", {
            value: { writeText: escrever },
            configurable: true,
        });
        prepararSessao("pophg_teste", "global", [ATIVA]);
        servidorCom([ATIVA]);

        montar();

        await usuario.click(await screen.findByText("loja-antiga"));
        const gaveta = await screen.findByRole("complementary");

        // O comando real: `--org` é global e o `chat` exige o agente como argumento.
        expect(
            await within(gaveta).findByText("poph chat --org loja-antiga atendente-demo"),
        ).toBeInTheDocument();

        await usuario.click(within(gaveta).getByRole("button", { name: "Copiar comando" }));

        expect(escrever).toHaveBeenCalledWith("poph chat --org loja-antiga atendente-demo");
        expect(await within(gaveta).findByRole("status")).toHaveTextContent("Copiado");
    });

    it("com token de org o inspetor não mostra botão de gestão", async () => {
        const usuario = userEvent.setup();
        prepararSessao("poph_org_teste", "org", [ATIVA]);
        servidorCom([ATIVA]);

        montar();

        await usuario.click(await screen.findByText("loja-antiga"));
        const gaveta = await screen.findByRole("complementary", {
            name: "Detalhes de Loja Antiga",
        });

        expect(within(gaveta).queryByRole("button", { name: "Editar" })).not.toBeInTheDocument();
        expect(
            within(gaveta).queryByRole("button", { name: /Excluir/ }),
        ).not.toBeInTheDocument();
        expect(within(gaveta).queryByRole("button", { name: "Reativar" })).not.toBeInTheDocument();
        expect(
            screen.queryByRole("button", { name: "Ações de Loja Antiga" }),
        ).not.toBeInTheDocument();
        expect(
            within(gaveta).getByRole("button", { name: "Acessar organização →" }),
        ).toBeInTheDocument();
    });

    it("Acessar organização passa pela guarda de alterações não salvas", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", [ATIVA]);
        servidorCom([ATIVA]);

        montar("/orgs", { sujo: true });

        await usuario.click(await screen.findByText("loja-antiga"));
        const gaveta = await screen.findByRole("complementary");

        await usuario.click(
            within(gaveta).getByRole("button", { name: "Acessar organização →" }),
        );

        expect(await screen.findByRole("dialog")).toHaveAccessibleName("Alterações não salvas");
        expect(screen.queryByTestId("caminho")).not.toBeInTheDocument();
    });
});

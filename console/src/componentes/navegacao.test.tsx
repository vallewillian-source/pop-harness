/**
 * Testes do menu lateral (RL-01 e RL-03).
 *
 * Cobrem a lista de navegação como o rail a usa: grupos e itens, o que exige organização ou
 * agente na URL, o item ativo, o contador de Organizações, o seletor de organização (com token de
 * org e passando pela guarda de alterações) e o rodapé.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation } from "react-router-dom";
import { useEffect } from "react";
import { ItenDeNavegacao, Rail } from "./Rail";
import type { Agente } from "../api/agentes";
import {
    ITENS_DE_NAVEGACAO,
    ITENS_DO_COCKPIT_DO_AGENTE,
    type ContextoDeNavegacao,
    type ItemDeNavegacao,
} from "./navegacao";
import { GuardaAlteracoesProvider, useGuardaDeAlteracoes } from "./GuardaAlteracoes";
import { setLocale } from "../i18n";
import { SessaoProvider } from "../sessao/Sessao";
import { ServidorFalso, erroApi } from "../testes/servidor";
import AppFonte from "../App.tsx?raw";

// Rotas declaradas no `App.tsx`, lidas do próprio arquivo: o cockpit só aponta para tela que existe.
// O rail consulta a saúde da instância (rede). Nos testes ela devolve uma versão fixa; os testes do
// chip do cluster trocam o estado (degradada, sem conexão) por `estadoDaSaude.atual`.
const estadoDaSaude = vi.hoisted(() => ({
    atual: {
        saude: { version: "0.9.1" } as { version: string } | null,
        semConexao: false,
        degradada: false,
    },
}));
vi.mock("./saude", () => ({ useSaude: () => estadoDaSaude.atual }));

const CHAVE_SESSAO = "poph-console.sessao";
const CONTEXTO_VAZIO: ContextoDeNavegacao = { orgId: null, agentId: null, totalDeOrgs: null };

function item(rotulo: string): ItemDeNavegacao {
    const achado = ITENS_DE_NAVEGACAO.find((candidato) => candidato.rotulo === rotulo);
    if (!achado) throw new Error(`item de navegação ausente: ${rotulo}`);
    return achado;
}

/** Mostra o caminho atual para provar se um clique navegou. */
function Sonda() {
    const { pathname } = useLocation();
    return <span data-testid="caminho">{pathname}</span>;
}

/** Declara uma edição pendente na tela, como o formulário de Instruções faz quando está sujo. */
function Sujo() {
    const { registrar } = useGuardaDeAlteracoes();
    useEffect(() => {
        registrar({ sujo: true, salvar: async () => true });
        return () => registrar(null);
    }, [registrar]);
    return null;
}

function prepararSessao(
    escopo: "global" | "org",
    orgs: { id: string; name: string; slug?: string; status?: string }[],
) {
    window.sessionStorage.setItem(
        CHAVE_SESSAO,
        JSON.stringify({
            token: escopo === "org" ? "poph_teste" : "pophg_teste",
            escopo,
            orgs,
        }),
    );
}

function montarRail(
    caminho: string,
    opcoes: {
        sujo?: boolean;
        aoAbrirAtalhos?: () => void;
        agents?: Agente[];
        compacta?: boolean;
    } = {},
) {
    return render(
        <MemoryRouter initialEntries={[caminho]}>
            <SessaoProvider>
                <GuardaAlteracoesProvider>
                    <Rail
                        aoAbrirAtalhos={opcoes.aoAbrirAtalhos ?? (() => {})}
                        agents={opcoes.agents}
                        compacta={opcoes.compacta}
                    />
                    {opcoes.sujo && <Sujo />}
                    <Sonda />
                </GuardaAlteracoesProvider>
            </SessaoProvider>
        </MemoryRouter>,
    );
}

const DUAS_ORGS = [
    { id: "org_1", name: "Loja Antiga", slug: "loja-antiga", status: "active" },
    { id: "org_2", name: "Clínica Nova", slug: "clinica-nova", status: "inactive" },
];

beforeEach(() => {
    window.sessionStorage.clear();
});

afterEach(() => {
    window.sessionStorage.clear();
    vi.unstubAllGlobals();
});

describe("lista de navegação", () => {
    it("tem os grupos e itens do PRD, na ordem do desenho", () => {
        expect(ITENS_DE_NAVEGACAO.map((entrada) => entrada.rotulo)).toEqual([
            "Organizações",
            "Estrutura",
            "Produtos e catálogo",
            "Agentes",
            "Modelo e Jev",
            "Inspeção de turnos",
            "Chat de teste",
            "WhatsApp e canais",
            "Matrix (Element)",
            "Plugins e extensões",
            "Saúde da instância",
        ]);
    });

    it("não tem mais Cluster nem Importar / exportar", () => {
        const rotulos = ITENS_DE_NAVEGACAO.map((entrada) => entrada.rotulo);
        expect(rotulos).not.toContain("Cluster");
        expect(rotulos).not.toContain("Importar / exportar");
    });

    it("agrupa os itens em TENANTS E RECURSOS, RUNTIME E MOTOR e SISTEMA", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs");

        expect(screen.getByText("TENANTS E RECURSOS")).toBeInTheDocument();
        expect(screen.getByText("RUNTIME E MOTOR")).toBeInTheDocument();
        expect(screen.getByText("SISTEMA")).toBeInTheDocument();
        expect(screen.queryByText("Cluster")).toBeNull();
        expect(screen.queryByText("Importar / exportar")).toBeNull();
    });
});

describe("itens do menu", () => {
    it("desabilita item que exige organização quando não há organização na URL", () => {
        render(
            <MemoryRouter>
                <ItenDeNavegacao item={item("Estrutura")} contexto={CONTEXTO_VAZIO} />
            </MemoryRouter>,
        );

        expect(screen.queryByRole("link", { name: /Estrutura/ })).toBeNull();
        const botao = screen.getByRole("button", { name: /Estrutura/ });
        expect(botao).toBeDisabled();
        // Item ativo sem organização não é "em breve": a tela existe, só falta o contexto.
        expect(botao).not.toHaveTextContent("em breve");
    });

    it("vira link para a rota da organização quando ela está na URL", () => {
        const casos: [string, string][] = [
            ["Estrutura", "/orgs/org_1/structure"],
            ["Produtos e catálogo", "/orgs/org_1/products"],
            ["Agentes", "/orgs/org_1/agents"],
        ];
        for (const [rotulo, rota] of casos) {
            const renderizado = render(
                <MemoryRouter>
                    <ItenDeNavegacao
                        item={item(rotulo)}
                        contexto={{ orgId: "org_1", agentId: null, totalDeOrgs: null }}
                    />
                </MemoryRouter>,
            );
            expect(screen.getByRole("link", { name: rotulo })).toHaveAttribute("href", rota);
            renderizado.unmount();
        }
    });

    it("Modelo e Jev fica desabilitado sem agente na URL", () => {
        render(
            <MemoryRouter>
                <ItenDeNavegacao
                    item={item("Modelo e Jev")}
                    contexto={{ orgId: "org_1", agentId: null, totalDeOrgs: null }}
                />
            </MemoryRouter>,
        );

        expect(screen.queryByRole("link", { name: /Modelo e Jev/ })).toBeNull();
        expect(screen.getByRole("button", { name: /Modelo e Jev/ })).toBeDisabled();
    });

    it("Modelo e Jev com agente na URL é link para a aba real do agente", () => {
        render(
            <MemoryRouter>
                <ItenDeNavegacao
                    item={item("Modelo e Jev")}
                    contexto={{ orgId: "org_1", agentId: "agt_1", totalDeOrgs: null }}
                />
            </MemoryRouter>,
        );

        expect(screen.getByRole("link", { name: "Modelo e Jev" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_1/model-jev",
        );
    });

    it("item sem exigência continua link sem organização aberta", () => {
        render(
            <MemoryRouter>
                <ItenDeNavegacao item={item("Organizações")} contexto={CONTEXTO_VAZIO} />
            </MemoryRouter>,
        );

        expect(screen.getByRole("link", { name: /Organizações/ })).toHaveAttribute("href", "/orgs");
    });

    it("item em breve não navega mesmo com organização aberta", async () => {
        const usuario = userEvent.setup();
        render(
            <MemoryRouter initialEntries={["/orgs/org_1"]}>
                <ItenDeNavegacao
                    item={item("Plugins e extensões")}
                    contexto={{ orgId: "org_1", agentId: null, totalDeOrgs: null }}
                />
                <Sonda />
            </MemoryRouter>,
        );

        expect(screen.queryByRole("link", { name: /Plugins e extensões/ })).toBeNull();
        const botao = screen.getByRole("button", { name: /Plugins e extensões/ });
        expect(botao).toBeDisabled();
        expect(botao).toHaveTextContent("em breve");

        await usuario.click(botao);
        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_1");
    });

    it("contador de Organizações mostra o total da sessão", () => {
        render(
            <MemoryRouter>
                <ItenDeNavegacao
                    item={item("Organizações")}
                    contexto={{ orgId: null, agentId: null, totalDeOrgs: 7 }}
                />
            </MemoryRouter>,
        );

        expect(screen.getByRole("link", { name: /Organizações/ })).toHaveTextContent("7");
    });
});

describe("CN-02: agentes sob o item Agentes", () => {
    function agent(partial: Partial<Agente> & Pick<Agente, "id" | "name">): Agente {
        return {
            org_id: "org_1",
            slug: partial.id,
            description: "",
            status: "active",
            default_department_id: null,
            human_handoff_department_id: null,
            version: 1,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            ...partial,
        };
    }

    const AGENTS = [
        agent({ id: "agt_1", name: "Atendente", slug: "atendente", status: "active" }),
        agent({ id: "agt_2", name: "Patinhas", slug: "patinhas", status: "inactive" }),
        agent({ id: "agt_3", name: "Arquivado", slug: "arquivado", status: "archived" }),
    ];

    it("lista os agentes da organização logo abaixo de Agentes, com a bolinha do status", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure", { agents: AGENTS });

        expect(screen.getByRole("link", { name: /Atendente/ })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_1/instructions",
        );
        expect(screen.getByRole("link", { name: /Patinhas/ })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_2/instructions",
        );
        expect(screen.getByRole("link", { name: /Arquivado/ })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_3/instructions",
        );
    });

    it("a bolinha é verde no agente ativo e cinza nos demais, com o status no title", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure", { agents: AGENTS });

        expect(screen.getByTitle("Ativo")).toHaveClass("poph-rail__agent-dot--active");
        expect(screen.getByTitle("Inativo")).toHaveClass("poph-rail__agent-dot--idle");
        expect(screen.getByTitle("Arquivado")).toHaveClass("poph-rail__agent-dot--idle");
        expect(screen.getByLabelText("Ativo")).toBe(screen.getByTitle("Ativo"));
    });

    it("status desconhecido cai no cinza com o próprio texto", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure", {
            agents: [agent({ id: "agt_4", name: "Novo", status: "paused" })],
        });

        expect(screen.getByTitle("paused")).toHaveClass("poph-rail__agent-dot--idle");
    });

    it("clicar no agente leva direto para as Instruções dele", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure", { agents: AGENTS });

        await usuario.click(screen.getByRole("link", { name: /Patinhas/ }));

        expect(screen.getByTestId("caminho")).toHaveTextContent(
            "/orgs/org_1/agents/agt_2/instructions",
        );
    });

    it("no modo foco o menu não lista os agentes", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure", { agents: AGENTS, compacta: true });

        expect(screen.queryByRole("link", { name: /Atendente/ })).toBeNull();
        expect(screen.queryByTitle("Ativo")).toBeNull();
    });

    it("organização sem agentes mantém o item Agentes e não cria subitem", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure", { agents: [] });

        expect(screen.getByRole("link", { name: "Agentes" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents",
        );
        expect(screen.queryByTitle("Ativo")).toBeNull();
    });

    it("sem organização aberta, o menu não lista agentes", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs", { agents: AGENTS });

        expect(screen.queryByRole("link", { name: /Atendente/ })).toBeNull();
    });
});

describe("Saúde da instância (HR-48a)", () => {
    it("com organização na URL, é link para a tela de operação, sem 'em breve'", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        const link = screen.getByRole("link", { name: /Saúde da instância/ });
        expect(link).toHaveAttribute("href", "/orgs/org_1/operacao");
        expect(link).not.toHaveTextContent("em breve");
    });

    it("sem organização na URL, fica desabilitada com o motivo, sem 'em breve'", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs");

        const botao = screen.getByRole("button", { name: /Saúde da instância/ });
        expect(botao).toBeDisabled();
        expect(botao).not.toHaveTextContent("em breve");
    });
});

describe('"em breve" aparece só nos itens que ainda não têm tela', () => {
    it("com organização na URL, Estrutura, Produtos e Agentes são links sem 'em breve'", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        for (const rotulo of ["Estrutura", "Produtos e catálogo", "Agentes"]) {
            const link = screen.getByRole("link", { name: rotulo });
            expect(link).not.toHaveTextContent("em breve");
        }
    });

    // CF-09: nas rotas de agente o rail passou a ser o cockpit da prancha 03, então estes itens de
    // `ITENS_DE_NAVEGACAO` são conferidos no próprio item (eles continuam alimentando a busca).
    it("com agente na URL, Chat de teste é link para a aba do agente, sem 'em breve'", () => {
        render(
            <MemoryRouter>
                <ItenDeNavegacao
                    item={item("Chat de teste")}
                    contexto={{ orgId: "org_1", agentId: "agt_1", totalDeOrgs: null }}
                />
            </MemoryRouter>,
        );

        const link = screen.getByRole("link", { name: "Chat de teste" });
        expect(link).toHaveAttribute("href", "/orgs/org_1/agents/agt_1/test-chat");
        expect(link).not.toHaveTextContent("em breve");
    });

    it("com agente na URL, Modelo e Jev é link sem 'em breve'", () => {
        render(
            <MemoryRouter>
                <ItenDeNavegacao
                    item={item("Modelo e Jev")}
                    contexto={{ orgId: "org_1", agentId: "agt_1", totalDeOrgs: null }}
                />
            </MemoryRouter>,
        );

        const link = screen.getByRole("link", { name: "Modelo e Jev" });
        expect(link).toHaveAttribute("href", "/orgs/org_1/agents/agt_1/model-jev");
        expect(link).not.toHaveTextContent("em breve");
    });

    it("sem organização na URL, os itens que dependem dela ficam desabilitados sem 'em breve' e com o motivo", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs");

        for (const rotulo of [
            "Estrutura",
            "Produtos e catálogo",
            "Agentes",
            "Modelo e Jev",
            "Chat de teste",
            "Saúde da instância",
            "Matrix (Element)",
        ]) {
            const botao = screen.getByRole("button", { name: new RegExp(rotulo.replace(/[()]/g, "\\$&")) });
            expect(botao).toBeDisabled();
            expect(botao).not.toHaveTextContent("em breve");
        }
        expect(screen.getAllByTitle("Requer uma organização aberta")).toHaveLength(7);
    });

    it("com organização mas sem agente, Modelo e Jev fica desabilitado com o motivo do agente", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        const botao = screen.getByRole("button", { name: /Modelo e Jev/ });
        expect(botao).toBeDisabled();
        expect(botao).not.toHaveTextContent("em breve");
        const motivos = screen.getAllByTitle("Requer um agente aberto");
        expect(motivos.some((motivo) => motivo.contains(botao))).toBe(true);
    });

    // CF-09: conferido no item, e não no rail montado, porque nas rotas de agente o rail é o cockpit.
    it("itens de estado emBreve continuam desabilitados e com 'em breve', mesmo com organização aberta", () => {
        for (const rotulo of [
            "Inspeção de turnos",
            "WhatsApp e canais",
            "Plugins e extensões",
        ]) {
            const renderizado = render(
                <MemoryRouter>
                    <ItenDeNavegacao
                        item={item(rotulo)}
                        contexto={{ orgId: "org_1", agentId: "agt_1", totalDeOrgs: null }}
                    />
                </MemoryRouter>,
            );
            const botao = screen.getByRole("button", {
                name: new RegExp(rotulo.replace(/[()]/g, "\\$&")),
            });
            expect(botao).toBeDisabled();
            expect(botao).toHaveTextContent("em breve");
            renderizado.unmount();
        }
    });
});

describe("menu montado com a sessão", () => {
    it("sem org na URL, Estrutura, Produtos e catálogo, Agentes e Modelo e Jev ficam desabilitados", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs");

        for (const rotulo of ["Estrutura", "Produtos e catálogo", "Agentes", "Modelo e Jev", "Chat de teste"]) {
            expect(screen.getByRole("button", { name: new RegExp(rotulo.replace(/[()]/g, "\\$&")) })).toBeDisabled();
        }
    });

    it("com org na URL, os links de organização apontam para a rota certa", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        expect(screen.getByRole("link", { name: "Estrutura" })).toHaveAttribute(
            "href",
            "/orgs/org_1/structure",
        );
        expect(screen.getByRole("link", { name: "Produtos e catálogo" })).toHaveAttribute(
            "href",
            "/orgs/org_1/products",
        );
        expect(screen.getByRole("link", { name: "Agentes" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents",
        );
    });

    it("com agente na URL, o rail é o cockpit do agente, e não os grupos de organização", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/agents/agt_1/instructions");

        expect(screen.getByRole("link", { name: "Modelo & Parâmetros (Jev)" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_1/model-jev",
        );
        expect(screen.queryByText("TENANTS E RECURSOS")).toBeNull();
    });

    it("com organização aberta o menu não tem Organizações e o item ativo é o da tela", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        expect(screen.getByRole("link", { name: "Estrutura" })).toHaveAttribute(
            "aria-current",
            "page",
        );
        expect(screen.queryByRole("link", { name: /^Organizações/ })).toBeNull();
    });

    it("marca Organizações quando a tela é a lista", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs");

        expect(screen.getByRole("link", { name: /Organizações/ })).toHaveAttribute(
            "aria-current",
            "page",
        );
    });

    it("mostra o total de organizações da sessão no contador", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs");

        expect(screen.getByRole("link", { name: /Organizações/ })).toHaveTextContent("2");
    });

    it("o seletor mostra a organização aberta e lista as que o token vê, com Administrar Organizações", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        const seletor = screen.getByRole("combobox", { name: "Organização: Loja Antiga" });
        expect(seletor).toHaveTextContent("Loja Antiga");
        expect(screen.queryByRole("listbox")).toBeNull();

        await usuario.click(seletor);

        expect(screen.getByRole("listbox", { name: "Organizações disponíveis" })).toBeInTheDocument();
        expect(
            screen.getAllByRole("option")[0],
        ).toHaveAccessibleName(/Administrar Organizações/);
        expect(screen.getByRole("option", { name: /Loja Antiga/ })).toHaveAttribute(
            "aria-current",
            "true",
        );
        expect(screen.getByRole("option", { name: /Clínica Nova/ })).toBeInTheDocument();
    });

    it("na lista de organizações, o topo é o chip do cluster — e não o combobox", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs");

        const chip = screen.getByText("POP Harness Cluster");
        expect(chip).toBeInTheDocument();
        expect(chip.closest(".poph-rail__cluster")).toHaveAttribute(
            "title",
            "POP Harness Cluster · instância respondendo",
        );
        expect(screen.queryByRole("combobox")).toBeNull();
    });

    it("com organização aberta, o topo volta a ser o combobox do seletor", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        expect(screen.getByRole("combobox", { name: "Organização: Loja Antiga" })).toBeInTheDocument();
        expect(screen.queryByText("POP Harness Cluster")).toBeNull();
    });

    it("o ponto do chip acompanha a saúde: verde respondendo, vermelho degradada ou sem conexão", () => {
        prepararSessao("global", DUAS_ORGS);

        const ponto = () => {
            const { container, unmount } = montarRail("/orgs");
            const achado = container.querySelector(".poph-rail__cluster-ponto");
            const classe = achado?.className ?? "";
            unmount();
            return { classe, titulo: achado?.closest(".poph-rail__cluster")?.getAttribute("title") };
        };

        estadoDaSaude.atual = { saude: { version: "0.9.1" }, semConexao: false, degradada: false };
        expect(ponto()).toEqual({
            classe: "poph-rail__cluster-ponto",
            titulo: "POP Harness Cluster · instância respondendo",
        });

        estadoDaSaude.atual = { saude: null, semConexao: false, degradada: true };
        expect(ponto().titulo).toBe("POP Harness Cluster · instância degradada");
        expect(ponto().classe).toContain("poph-rail__cluster-ponto--alerta");

        estadoDaSaude.atual = { saude: null, semConexao: true, degradada: false };
        expect(ponto().titulo).toBe("POP Harness Cluster · sem conexão com a instância");
        expect(ponto().classe).toContain("poph-rail__cluster-ponto--alerta");

        estadoDaSaude.atual = { saude: null, semConexao: false, degradada: false };
        expect(ponto().titulo).toBe("POP Harness Cluster · verificando a instância");
        expect(ponto().classe).toContain("poph-rail__cluster-ponto--neutro");

        estadoDaSaude.atual = { saude: { version: "0.9.1" }, semConexao: false, degradada: false };
    });

    it("fora das telas de organização, sem organização aberta, diz Selecionar organização", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/");

        expect(screen.getByRole("combobox")).toHaveTextContent("Selecionar organização");
    });

    it("sem nenhuma organização, o gatilho diz Nenhuma organização e o popover explica", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", []);
        montarRail("/orgs/org_1/structure");

        const seletor = screen.getByRole("combobox");
        expect(seletor).toHaveTextContent("Nenhuma organização");

        await usuario.click(seletor);

        expect(screen.queryByRole("option")).toBeNull();
        expect(screen.getByRole("status")).toHaveTextContent("Nenhuma organização");
    });

    it("com token de org, o gatilho mostra a própria organização e não abre a lista", async () => {
        const usuario = userEvent.setup();
        prepararSessao("org", [{ id: "org_1", name: "Loja Antiga", slug: "loja-antiga" }]);
        montarRail("/orgs/org_1/structure");

        const seletor = screen.getByRole("combobox");
        expect(seletor).toBeDisabled();
        expect(seletor).toHaveTextContent("Loja Antiga");

        await usuario.click(seletor);
        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("escolher outra organização sem alterações pendentes navega para ela", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));
        await usuario.click(screen.getByRole("option", { name: /Clínica Nova/ }));

        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_2");
        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("o primeiro item do combobox é Administrar Organizações e leva para a lista", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));
        const administrar = screen.getByRole("option", { name: /Administrar Organizações/ });
        expect(screen.getAllByRole("option")[0]).toBe(administrar);

        await usuario.click(administrar);

        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs");
        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("Administrar Organizações com alterações pendentes pede confirmação antes de sair", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure", { sujo: true });

        await usuario.click(screen.getByRole("combobox"));
        await usuario.click(screen.getByRole("option", { name: /Administrar Organizações/ }));

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_1/structure");
    });

    it("escolher a organização que já está aberta não navega", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));
        await usuario.click(screen.getByRole("option", { name: /Loja Antiga/ }));

        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_1/structure");
        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("escolher outra organização com alterações pendentes pede confirmação antes de sair", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure", { sujo: true });

        await usuario.click(screen.getByRole("combobox"));
        await usuario.click(screen.getByRole("option", { name: /Clínica Nova/ }));

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_1/structure");
    });

    it("ao abrir, o foco vai para o campo de busca", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));

        expect(screen.getByRole("textbox", { name: /Filtrar organizações/ })).toHaveFocus();
    });

    it("Esc fecha sem navegar e devolve o foco ao gatilho", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));
        await usuario.keyboard("{Escape}");

        expect(screen.queryByRole("listbox")).toBeNull();
        expect(screen.getByRole("combobox")).toHaveFocus();
        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_1/structure");
    });

    it("clicar fora fecha o seletor sem devolver o foco", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));
        expect(screen.getByRole("listbox")).toBeInTheDocument();

        await usuario.click(screen.getByTestId("caminho"));

        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("digitar filtra por nome ou slug, sem diferenciar acento nem caixa", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));
        await usuario.type(screen.getByRole("textbox", { name: /Filtrar organizações/ }), "CLINICA");

        expect(screen.getByRole("option", { name: /Clínica Nova/ })).toBeInTheDocument();
        expect(screen.queryByRole("option", { name: /Loja Antiga/ })).toBeNull();
        expect(screen.queryByRole("option", { name: /Administrar Organizações/ })).toBeNull();

        await usuario.clear(screen.getByRole("textbox", { name: /Filtrar organizações/ }));
        await usuario.type(screen.getByRole("textbox", { name: /Filtrar organizações/ }), "loja-antiga");
        expect(screen.getByRole("option", { name: /Loja Antiga/ })).toBeInTheDocument();
        expect(screen.queryByRole("option", { name: /Clínica Nova/ })).toBeNull();
    });

    it("busca sem resultado mostra a mensagem com o termo", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));
        await usuario.type(screen.getByRole("textbox", { name: /Filtrar organizações/ }), "xyz");

        expect(screen.queryByRole("option")).toBeNull();
        expect(screen.getByRole("status")).toHaveTextContent("Nenhuma organização encontrada para “xyz”");
    });

    it("setas e Enter percorrem a lista e abrem a organização escolhida, devolvendo o foco", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));
        const campo = screen.getByRole("textbox", { name: /Filtrar organizações/ });
        const administrar = screen.getByRole("option", { name: /Administrar Organizações/ });
        const clinica = screen.getByRole("option", { name: /Clínica Nova/ });

        // A primeira opção começa ativa; duas setas para baixo chegam em Clínica Nova.
        expect(campo).toHaveAttribute("aria-activedescendant", administrar.id);
        await usuario.keyboard("{ArrowDown}{ArrowDown}");
        expect(campo).toHaveAttribute("aria-activedescendant", clinica.id);
        expect(clinica).toHaveAttribute("aria-selected", "true");

        await usuario.keyboard("{Enter}");

        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_2");
        expect(screen.getByRole("combobox")).toHaveFocus();
    });

    it("End e Home levam à última e à primeira opção, e a seta para baixo não passa do fim", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));
        const campo = screen.getByRole("textbox", { name: /Filtrar organizações/ });
        const administrar = screen.getByRole("option", { name: /Administrar Organizações/ });
        const clinica = screen.getByRole("option", { name: /Clínica Nova/ });

        await usuario.keyboard("{End}");
        expect(campo).toHaveAttribute("aria-activedescendant", clinica.id);
        await usuario.keyboard("{ArrowDown}");
        expect(campo).toHaveAttribute("aria-activedescendant", clinica.id);
        await usuario.keyboard("{Home}");
        expect(campo).toHaveAttribute("aria-activedescendant", administrar.id);
    });

    it("a situação de cada organização fica no nome acessível da opção", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.click(screen.getByRole("combobox"));

        expect(screen.getByRole("option", { name: /Loja Antiga/ })).toHaveAccessibleName(/Ativa/);
        expect(screen.getByRole("option", { name: /Clínica Nova/ })).toHaveAccessibleName(/Inativa/);
    });

    it("o rodapé mostra a versão da instância e o botão de atalhos abre a lista", async () => {
        const usuario = userEvent.setup();
        const aoAbrirAtalhos = vi.fn();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs", { aoAbrirAtalhos });

        expect(screen.getByText("POP Harness v0.9.1 · Rust")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Atalhos" })).toBeNull();

        await usuario.click(screen.getByRole("button", { name: "? atalhos" }));
        expect(aoAbrirAtalhos).toHaveBeenCalledTimes(1);
    });

    describe("CF-02: rail de 240 px e modo foco de 64 px", () => {
        function montarComEstado(
            compacta: boolean,
            aoAlternarCompacta = vi.fn(),
            caminho = "/orgs",
        ) {
            prepararSessao("global", DUAS_ORGS);
            return {
                aoAlternarCompacta,
                ...render(
                    <MemoryRouter initialEntries={[caminho]}>
                        <SessaoProvider>
                            <GuardaAlteracoesProvider>
                                <Rail
                                    aoAbrirAtalhos={() => {}}
                                    compacta={compacta}
                                    aoAlternarCompacta={aoAlternarCompacta}
                                />
                            </GuardaAlteracoesProvider>
                        </SessaoProvider>
                    </MemoryRouter>,
                ),
            };
        }

        it("expandido: mostra o chip do cluster na lista e o botão de recolher", async () => {
            const usuario = userEvent.setup();
            const { aoAlternarCompacta } = montarComEstado(false);

            expect(screen.getByText("POP Harness Cluster")).toBeInTheDocument();
            expect(screen.getByText("POP Harness v0.9.1 · Rust")).toBeInTheDocument();
            expect(screen.queryByRole("button", { name: "Expandir menu lateral" })).toBeNull();

            await usuario.click(screen.getByRole("button", { name: "Recolher menu lateral" }));
            expect(aoAlternarCompacta).toHaveBeenCalledTimes(1);
        });

        it("em foco: o chip do cluster vira um tile de 32×32 com o nome no title", () => {
            montarComEstado(true);

            const chip = screen.getByText("POP Harness Cluster").closest(".poph-rail__cluster");
            expect(chip).toHaveClass("poph-rail__cluster--compacta");
            expect(chip).toHaveAttribute("title", "POP Harness Cluster · instância respondendo");
            expect(screen.queryByText("POP Harness v0.9.1 · Rust")).toBeNull();
            expect(screen.getByRole("button", { name: "Expandir menu lateral" })).toBeInTheDocument();
            expect(screen.getByRole("link", { name: /Organizações/ })).toHaveAttribute(
                "title",
                "Organizações",
            );
        });

        it("em foco: com organização aberta, o seletor vira um tile e o popover abre ao lado", async () => {
            const usuario = userEvent.setup();
            montarComEstado(true, vi.fn(), "/orgs/org_1/structure");

            expect(screen.queryByText("POP Harness Cluster")).toBeNull();
            const seletor = screen.getByRole("combobox");
            expect(seletor).toHaveAttribute("title", "Loja Antiga");
            expect(seletor.closest(".poph-orgsel")).toHaveClass("poph-orgsel--compacta");

            // O popover abre ao lado do tile, com o mesmo fluxo do modo expandido.
            await usuario.click(seletor);
            expect(screen.getByRole("listbox")).toBeInTheDocument();
        });

        it("o botão que alterna continua alcançável por teclado nos dois estados", async () => {
            const usuario = userEvent.setup();
            const { aoAlternarCompacta } = montarComEstado(true);

            const expandir = screen.getByRole("button", { name: "Expandir menu lateral" });
            expandir.focus();
            expect(expandir).toHaveFocus();
            await usuario.keyboard("{Enter}");
            expect(aoAlternarCompacta).toHaveBeenCalledTimes(1);
        });
    });
});

/**
 * CF-07: na visão do agente (`/orgs/:id/agents/:agentId/...`, prancha 03) o topo do rail é o link
 * "← Agentes da Org" e o combobox do agente. Todas as leituras passam por um servidor falso, para
 * o teste conferir também quantas consultas a tela faz.
 */
describe("CF-07: seletor de agente no topo do rail", () => {
    function agente(parcial: Partial<Agente> & Pick<Agente, "id" | "name" | "slug">): Agente {
        return {
            org_id: "org_1",
            description: "",
            status: "active",
            default_department_id: null,
            human_handoff_department_id: null,
            version: 1,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
            ...parcial,
        };
    }

    const MATRIX = agente({
        id: "agt_1",
        name: "Atendente Matrix",
        slug: "atendente-matrix",
        model: "gemini-3.8-flash",
        status: "active",
    });
    // Sem `model` próprio: a API manda `null` nesse caso e a tela mostra o padrão do servidor.
    const PATINHAS = agente({
        id: "agt_2",
        name: "Patinhas",
        slug: "patinhas",
        status: "inactive",
    });
    const TRIAGEM = agente({
        id: "agt_3",
        name: "Triagem",
        slug: "triagem",
        model: "gpt-6-sol",
        status: "active",
    });
    const FORA_DA_LISTA = agente({
        id: "agt_9",
        name: "Agente Arquivado",
        slug: "agente-arquivado",
        status: "archived",
    });

    /** Servidor falso com a lista de agentes; a leitura de um agente só (`agt_9`) é declarada antes. */
    function prepararServidor(
        opcoes: {
            agentes?: Agente[];
            avulso?: { agente: Agente } | { falha: true };
            falhaNaLista?: boolean;
        } = {},
    ) {
        const servidor = new ServidorFalso();
        const avulso = opcoes.avulso;
        if (avulso && "agente" in avulso) {
            servidor.responder("GET", `/agents/${avulso.agente.id}`, {
                status: 200,
                corpo: avulso.agente,
            });
        } else if (avulso) {
            servidor.responder("GET", "/agents/", () =>
                erroApi(404, "not_found", "agente não encontrado"),
            );
        }
        servidor.responder("GET", "/agents", () =>
            opcoes.falhaNaLista
                ? erroApi(500, "interno", "falha ao ler os agentes")
                : {
                      status: 200,
                      corpo: { items: opcoes.agentes ?? [MATRIX, PATINHAS, TRIAGEM], next_cursor: null },
                  },
        );
        servidor.instalar();
        return servidor;
    }

    function montar(caminho: string, compacta = false) {
        prepararSessao("global", DUAS_ORGS);
        return render(
            <MemoryRouter initialEntries={[caminho]}>
                <SessaoProvider>
                    <GuardaAlteracoesProvider>
                        <Rail aoAbrirAtalhos={() => {}} compacta={compacta} />
                        <Sonda />
                    </GuardaAlteracoesProvider>
                </SessaoProvider>
            </MemoryRouter>,
        );
    }

    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("mostra o link de volta para os agentes da organização", () => {
        prepararServidor();
        montar("/orgs/org_1/agents/agt_1/instructions");

        expect(screen.getByRole("link", { name: "Agentes da Org" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents",
        );
    });

    it("no lugar do seletor de organização, mostra o agente da URL com modelo e situação", async () => {
        prepararServidor();
        montar("/orgs/org_1/agents/agt_1/instructions");

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" });
        expect(gatilho).toHaveTextContent("Atendente Matrix");
        expect(gatilho).toHaveTextContent("gemini-3.8-flash · Ativo");
        // O topo do rail na visão do agente não tem o chip nem o seletor de organização.
        expect(screen.queryByText("POP Harness Cluster")).toBeNull();
        expect(screen.queryByRole("combobox", { name: /^Organização:/ })).toBeNull();
    });

    it("agente sem modelo próprio mostra o padrão do servidor e a situação", async () => {
        prepararServidor();
        montar("/orgs/org_1/agents/agt_2/files");

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Patinhas" });
        expect(gatilho).toHaveTextContent("modelo padrão do servidor · Inativo");
    });

    it("abre a lista dos agentes da organização com busca por nome", async () => {
        const usuario = userEvent.setup();
        prepararServidor();
        montar("/orgs/org_1/agents/agt_1/instructions");

        await usuario.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));

        const campo = screen.getByRole("textbox", { name: /Filtrar agentes/ });
        expect(campo).toHaveFocus();
        expect(screen.getAllByRole("option")).toHaveLength(3);

        await usuario.type(campo, "TRIAGE");
        expect(screen.getByRole("option", { name: /Triagem/ })).toBeInTheDocument();
        expect(screen.queryByRole("option", { name: /Patinhas/ })).toBeNull();

        await usuario.clear(campo);
        await usuario.type(campo, "patinhas");
        expect(screen.getByRole("option", { name: /Patinhas/ })).toBeInTheDocument();
        expect(screen.queryByRole("option", { name: /Triagem/ })).toBeNull();
    });

    it("marca o agente aberto com o check e não navega ao escolhê-lo", async () => {
        const usuario = userEvent.setup();
        prepararServidor();
        montar("/orgs/org_1/agents/agt_1/instructions");

        await usuario.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));
        const atual = screen.getByRole("option", { name: /Atendente Matrix/ });
        expect(atual).toHaveAttribute("aria-current", "true");

        await usuario.click(atual);

        expect(screen.queryByRole("listbox")).toBeNull();
        expect(screen.getByTestId("caminho")).toHaveTextContent(
            "/orgs/org_1/agents/agt_1/instructions",
        );
    });

    it("trocar de agente com alterações pendentes pede confirmação antes de sair", async () => {
        const usuario = userEvent.setup();
        prepararServidor();
        prepararSessao("global", DUAS_ORGS);
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/agents/agt_1/instructions"]}>
                <SessaoProvider>
                    <GuardaAlteracoesProvider>
                        <Rail aoAbrirAtalhos={() => {}} />
                        <Sujo />
                        <Sonda />
                    </GuardaAlteracoesProvider>
                </SessaoProvider>
            </MemoryRouter>,
        );

        await usuario.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));
        await usuario.click(screen.getByRole("option", { name: /Triagem/ }));

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("caminho")).toHaveTextContent(
            "/orgs/org_1/agents/agt_1/instructions",
        );
    });

    it("setas e Enter trocam de agente, indo para as Instruções dele", async () => {
        const usuario = userEvent.setup();
        prepararServidor();
        montar("/orgs/org_1/agents/agt_1/instructions");

        await usuario.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));
        const campo = screen.getByRole("textbox", { name: /Filtrar agentes/ });
        const triagem = screen.getByRole("option", { name: /Triagem/ });

        // A primeira opção começa ativa; duas setas chegam em Triagem (a terceira da lista).
        expect(campo).toHaveAttribute("aria-activedescendant", screen.getByRole("option", { name: /Atendente Matrix/ }).id);
        await usuario.keyboard("{ArrowDown}{ArrowDown}");
        expect(campo).toHaveAttribute("aria-activedescendant", triagem.id);

        await usuario.keyboard("{Enter}");

        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_1/agents/agt_3/instructions");
        expect(screen.getByRole("combobox")).toHaveFocus();
    });

    it("Esc fecha sem navegar e devolve o foco ao gatilho", async () => {
        const usuario = userEvent.setup();
        prepararServidor();
        montar("/orgs/org_1/agents/agt_1/instructions");

        await usuario.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));
        await usuario.keyboard("{Escape}");

        expect(screen.queryByRole("listbox")).toBeNull();
        expect(screen.getByRole("combobox")).toHaveFocus();
        expect(screen.getByTestId("caminho")).toHaveTextContent(
            "/orgs/org_1/agents/agt_1/instructions",
        );
    });

    it("clicar fora fecha o seletor", async () => {
        const usuario = userEvent.setup();
        prepararServidor();
        montar("/orgs/org_1/agents/agt_1/instructions");

        await usuario.click(await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }));
        expect(screen.getByRole("listbox")).toBeInTheDocument();

        await usuario.click(screen.getByTestId("caminho"));

        expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("organização sem agentes mostra o estado vazio no popover", async () => {
        const usuario = userEvent.setup();
        prepararServidor({ agentes: [] });
        montar("/orgs/org_1/agents/agt_1/instructions");

        await usuario.click(await screen.findByRole("combobox"));
        expect(screen.getByRole("status")).toHaveTextContent("Nenhum agente nesta organização");
    });

    it("falha ao ler os agentes aparece no popover e não derruba a navegação", async () => {
        const usuario = userEvent.setup();
        prepararServidor({ falhaNaLista: true });
        montar("/orgs/org_1/agents/agt_1/instructions");

        const gatilho = await screen.findByRole("combobox");
        expect(gatilho).toHaveAccessibleName("Agente: Agentes indisponíveis");
        await usuario.click(gatilho);
        expect(await screen.findByRole("status")).toHaveTextContent(
            "Não foi possível ler os agentes",
        );
        // O resto do menu continua funcionando.
        expect(screen.getByRole("link", { name: "Chat de Teste" })).toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Agentes da Org" })).toBeInTheDocument();
    });

    it("agente da URL fora da lista é lido sozinho e mostra o nome dele", async () => {
        prepararServidor({ avulso: { agente: FORA_DA_LISTA } });
        montar("/orgs/org_1/agents/agt_9/instructions");

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Agente Arquivado" });
        expect(gatilho).toHaveTextContent("Agente Arquivado");
        expect(gatilho).toHaveTextContent("Arquivado");
    });

    it("agente da URL que não existe mais aparece como não encontrado, sem quebrar a tela", async () => {
        prepararServidor({ avulso: { falha: true } });
        montar("/orgs/org_1/agents/agt_9/instructions");

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Agente não encontrado" });
        expect(gatilho).toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Agentes da Org" })).toBeInTheDocument();
    });

    it("uma consulta por organização: abrir a lista duas vezes não relê os agentes", async () => {
        const usuario = userEvent.setup();
        const servidor = prepararServidor();
        montar("/orgs/org_1/agents/agt_1/instructions");

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" });
        await usuario.click(gatilho);
        await usuario.keyboard("{Escape}");
        await usuario.click(gatilho);

        expect(servidor.chamadasPara("GET", "/agents")).toHaveLength(1);
    });

    it("em modo foco, o link e o gatilho viram tiles e o popover abre ao lado", async () => {
        const usuario = userEvent.setup();
        prepararServidor();
        montar("/orgs/org_1/agents/agt_1/instructions", true);

        const gatilho = await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" });
        expect(gatilho).toHaveAttribute("title", "Atendente Matrix");
        expect(gatilho.closest(".poph-agsel")).toHaveClass("poph-agsel--compacta");

        const voltar = screen.getByRole("link", { name: "Agentes da Org" });
        expect(voltar).toHaveClass("poph-agsel__voltar--compacta");

        await usuario.click(gatilho);
        expect(screen.getByRole("listbox")).toBeInTheDocument();
    });

    it("com token de organização, o seletor do agente continua funcionando", async () => {
        prepararSessao("org", [{ id: "org_1", name: "Loja Antiga", slug: "loja-antiga" }]);
        prepararServidor();
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/agents/agt_1/instructions"]}>
                <SessaoProvider>
                    <GuardaAlteracoesProvider>
                        <Rail aoAbrirAtalhos={() => {}} />
                        <Sonda />
                    </GuardaAlteracoesProvider>
                </SessaoProvider>
            </MemoryRouter>,
        );

        expect(
            await screen.findByRole("combobox", { name: "Agente: Atendente Matrix" }),
        ).toBeEnabled();
    });

    it("fora das rotas de agente, o topo continua sendo o seletor de organização", () => {
        prepararServidor();
        montar("/orgs/org_1/structure");

        expect(screen.getByRole("combobox", { name: "Organização: Loja Antiga" })).toBeInTheDocument();
        expect(screen.queryByRole("link", { name: "Agentes da Org" })).toBeNull();
    });
});

/**
 * CF-09: cockpit do agente no rail (prancha 03). Nas rotas `/orgs/:orgId/agents/:agentId/...` o rail
 * passa a ser o cockpit vertical do desenho — primeiro bloco sem título, depois "Diagnóstico &
 * Teste" — e o rodapé troca a versão por "Agente: <nome>". Fora dessas rotas nada muda.
 */
describe("CF-09: cockpit do agente no rail", () => {
    const AGENTE: Agente = {
        id: "agt_1",
        org_id: "org_1",
        name: "Atendente Matrix",
        slug: "atendente-matrix",
        description: "",
        status: "active",
        model: "gemini-3.8-flash",
        default_department_id: null,
        human_handoff_department_id: null,
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    };

    /** Lista de agentes do servidor falso: o combobox do agente e o rodapé leem daqui. */
    function prepararServidor() {
        const servidor = new ServidorFalso();
        servidor.responder("GET", "/agents", {
            status: 200,
            corpo: { items: [AGENTE], next_cursor: null },
        });
        servidor.instalar();
        return servidor;
    }

    function montar(caminho: string, opcoes: { compacta?: boolean; sujo?: boolean } = {}) {
        prepararSessao("global", DUAS_ORGS);
        return render(
            <MemoryRouter initialEntries={[caminho]}>
                <SessaoProvider>
                    <GuardaAlteracoesProvider>
                        <Rail aoAbrirAtalhos={() => {}} compacta={opcoes.compacta ?? false} />
                        {opcoes.sujo && <Sujo />}
                        <Sonda />
                    </GuardaAlteracoesProvider>
                </SessaoProvider>
            </MemoryRouter>,
        );
    }

    const ROTA = "/orgs/org_1/agents/agt_1/instructions";

    it("nas rotas de agente, o rail tem os itens da prancha 03 na ordem do desenho", () => {
        prepararServidor();
        montar(ROTA);

        const rotulos = [
            "Instruções & Prompts",
            "Base de Conhecimento",
            "Arquivos do Agente",
            "Roteamento & Habilidades",
            "Modelo & Parâmetros (Jev)",
            "Chat de Teste",
            "Inspeção de Turnos",
        ];
        // Só os itens do cockpit (o topo do rail tem o link de volta e o combobox do agente).
        const lista = screen.getByRole("navigation").querySelector(".sofa-sidebar-rail__list");
        const ordem = Array.from(lista?.querySelectorAll(".sofa-rail-item__label") ?? []).map(
            (no) => no.textContent,
        );
        expect(ordem).toEqual(rotulos);
        expect(screen.getByText("Diagnóstico & Teste")).toBeInTheDocument();
        // Nenhum grupo do rail de organização aparece na visão do agente.
        expect(screen.queryByText("TENANTS E RECURSOS")).toBeNull();
        expect(screen.queryByText("SISTEMA")).toBeNull();
    });

    it("cada item ativo leva para a tela real do agente", () => {
        prepararServidor();
        montar(ROTA);

        const rotas: [string, string][] = [
            ["Instruções & Prompts", "instructions"],
            ["Base de Conhecimento", "knowledge-base"],
            ["Arquivos do Agente", "files"],
            ["Roteamento & Habilidades", "routing"],
            ["Modelo & Parâmetros (Jev)", "model-jev"],
            ["Chat de Teste", "test-chat"],
        ];
        for (const [rotulo, aba] of rotas) {
            expect(screen.getByRole("link", { name: rotulo })).toHaveAttribute(
                "href",
                `/orgs/org_1/agents/agt_1/${aba}`,
            );
        }
    });

    it("o item da rota atual fica destacado, e só ele", () => {
        prepararServidor();
        montar("/orgs/org_1/agents/agt_1/routing");

        expect(screen.getByRole("link", { name: "Roteamento & Habilidades" })).toHaveAttribute(
            "aria-current",
            "page",
        );
        expect(screen.getByRole("link", { name: "Instruções & Prompts" })).not.toHaveAttribute(
            "aria-current",
        );
    });

    it("na raiz do agente, Instruções & Prompts já aparece ativa", () => {
        prepararServidor();
        montar("/orgs/org_1/agents/agt_1");

        // A raiz redireciona para /instructions: o item já nasce destacado (o `aria-current` do
        // link é do NavLink; quem marca o estado visual do item é o SofaRailItem).
        const link = screen.getByRole("link", { name: "Instruções & Prompts" });
        expect(link.querySelector(".sofa-rail-item")).toHaveAttribute("aria-current", "true");
    });

    it("toda rota do cockpit existe no App.tsx", () => {
        const declaradas = Array.from(AppFonte.matchAll(/path="([^"]+)"/g), (achado) => achado[1] ?? "");
        for (const entrada of ITENS_DO_COCKPIT_DO_AGENTE) {
            const rota = entrada.rota({ orgId: "org_1", agentId: "agt_1", totalDeOrgs: null });
            if (rota === null) continue;
            const padrao = rota
                .replace("/orgs/org_1", "/orgs/:orgId")
                .replace("/agents/agt_1", "/agents/:agentId");
            expect(declaradas).toContain(padrao);
        }
    });

    it("Inspeção de Turnos continua 'em breve', desabilitada e sem navegar", async () => {
        const usuario = userEvent.setup();
        prepararServidor();
        montar(ROTA);

        expect(screen.queryByRole("link", { name: "Inspeção de Turnos" })).toBeNull();
        const botao = screen.getByRole("button", { name: /Inspeção de Turnos/ });
        expect(botao).toBeDisabled();
        expect(botao).toHaveTextContent("em breve");

        await usuario.click(botao);
        expect(screen.getByTestId("caminho")).toHaveTextContent(ROTA);
    });

    it("o rodapé mostra 'Agente: <nome>' no lugar da versão", async () => {
        prepararServidor();
        montar(ROTA);

        expect(await screen.findByText("Agente: Atendente Matrix")).toBeInTheDocument();
        expect(screen.queryByText("POP Harness v0.9.1 · Rust")).toBeNull();
    });

    it("no modo foco os itens viram tiles com title e o título de seção sai de cena", () => {
        prepararServidor();
        montar(ROTA, { compacta: true });

        expect(screen.queryByText("Diagnóstico & Teste")).toBeNull();
        expect(screen.queryByText(/^Agente: /)).toBeNull();
        expect(screen.getByRole("link", { name: "Instruções & Prompts" })).toHaveAttribute(
            "title",
            "Instruções & Prompts",
        );
        // O item "em breve" também ganha o nome no title, que é a única pista no modo foco.
        const emBreve = screen.getByRole("button", { name: /Inspeção de Turnos/ });
        expect(emBreve.closest(".poph-rail__bloqueado")).toHaveAttribute(
            "title",
            "Inspeção de Turnos",
        );
    });

    it("trocar de tela pelo cockpit com alterações pendentes pede confirmação", async () => {
        const usuario = userEvent.setup();
        prepararServidor();
        montar(ROTA, { sujo: true });

        await usuario.click(screen.getByRole("link", { name: "Arquivos do Agente" }));

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("caminho")).toHaveTextContent(ROTA);
    });

    it("fora das rotas de agente, o rail continua com os grupos e itens de sempre", () => {
        prepararServidor();
        montar("/orgs/org_1/structure");

        expect(screen.getByText("TENANTS E RECURSOS")).toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Agentes" })).toBeInTheDocument();
        expect(screen.queryByText("Instruções & Prompts")).toBeNull();
        expect(screen.queryByText("Diagnóstico & Teste")).toBeNull();
    });
});

describe("I18N-04: rail e seletores em en-US", () => {
    const AGENTE_EN: Agente = {
        id: "agt_1",
        org_id: "org_1",
        name: "Atendente Matrix",
        slug: "atendente-matrix",
        description: "",
        status: "active",
        model: "gemini-3.8-flash",
        default_department_id: null,
        human_handoff_department_id: null,
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    };

    function servirAgentes(agentes: Agente[] = [AGENTE_EN]) {
        return new ServidorFalso()
            .responder("GET", "/agents", {
                status: 200,
                corpo: { items: agentes, next_cursor: null },
            })
            .instalar();
    }

    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("a lista de navegação tem os mesmos itens em inglês", () => {
        setLocale("en-US");

        expect(ITENS_DE_NAVEGACAO.map((entrada) => entrada.rotulo)).toEqual([
            "Organizations",
            "Structure",
            "Products and catalog",
            "Agents",
            "Model and Jev",
            "Turn inspection",
            "Test chat",
            "WhatsApp and channels",
            "Matrix (Element)",
            "Plugins and extensions",
            "Instance health",
        ]);
        expect(ITENS_DO_COCKPIT_DO_AGENTE.map((entrada) => entrada.rotulo)).toContain(
            "Instructions & Prompts",
        );
    });

    it("os grupos e o item 'em breve' aparecem em inglês no rail", () => {
        prepararSessao("global", DUAS_ORGS);
        setLocale("en-US");
        montarRail("/orgs");

        expect(screen.getByText("TENANTS AND RESOURCES")).toBeInTheDocument();
        expect(screen.getByText("RUNTIME AND ENGINE")).toBeInTheDocument();
        expect(screen.getByText("SYSTEM")).toBeInTheDocument();
        expect(screen.getByRole("link", { name: /^Organizations/ })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /Turn inspection/ })).toHaveTextContent(
            "coming soon",
        );
    });

    it("o rail reage à troca de idioma sem recarregar", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs");

        expect(screen.getByRole("link", { name: /^Organizações/ })).toBeInTheDocument();

        act(() => setLocale("en-US"));

        expect(screen.getByRole("link", { name: /^Organizations/ })).toBeInTheDocument();
        expect(screen.queryByRole("link", { name: /^Organizações/ })).toBeNull();
    });

    it("o seletor de organização fala inglês no gatilho, na busca e nas opções", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        setLocale("en-US");
        montarRail("/orgs/org_1/structure");

        const gatilho = screen.getByRole("combobox", {
            name: "Organization: Loja Antiga",
        });
        await usuario.click(gatilho);

        expect(screen.getByPlaceholderText("Filter organizations…")).toHaveAccessibleName(
            "Filter organizations by name or slug",
        );
        expect(screen.getByRole("listbox", { name: "Available organizations" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: /Manage Organizations/ })).toBeInTheDocument();
    });

    it("o seletor de agente e o link de volta falam inglês", async () => {
        servirAgentes();
        prepararSessao("global", DUAS_ORGS);
        setLocale("en-US");
        montarRail("/orgs/org_1/agents/agt_1/instructions");

        expect(screen.getByRole("link", { name: "Org Agents" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents",
        );
        const gatilho = await screen.findByRole("combobox", { name: "Agent: Atendente Matrix" });
        expect(gatilho).toHaveTextContent("gemini-3.8-flash · Active");
    });

    it("agente sem modelo próprio mostra o padrão do servidor em inglês", async () => {
        servirAgentes([{ ...AGENTE_EN, model: undefined, status: "inactive" }]);
        prepararSessao("global", DUAS_ORGS);
        setLocale("en-US");
        montarRail("/orgs/org_1/agents/agt_1/files");

        const gatilho = await screen.findByRole("combobox", { name: "Agent: Atendente Matrix" });
        expect(gatilho).toHaveTextContent("server default model · Inactive");
    });

    it("organização sem agentes mostra o estado vazio em inglês", async () => {
        const usuario = userEvent.setup();
        servirAgentes([]);
        prepararSessao("global", DUAS_ORGS);
        setLocale("en-US");
        montarRail("/orgs/org_1/agents/agt_1/instructions");

        await usuario.click(await screen.findByRole("combobox"));
        expect(screen.getByRole("status")).toHaveTextContent("No agent in this organization");
    });

    it("o rodapé do cockpit, os atalhos e o botão de recolher falam inglês", async () => {
        servirAgentes();
        prepararSessao("global", DUAS_ORGS);
        setLocale("en-US");
        montarRail("/orgs/org_1/agents/agt_1/instructions");

        expect(await screen.findByText("Agent: Atendente Matrix")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "? shortcuts" })).toHaveAttribute(
            "title",
            "Keyboard shortcuts (?)",
        );
        expect(screen.getByRole("button", { name: "Collapse sidebar" })).toBeInTheDocument();
    });

    it("o cockpit do agente tem as abas e o grupo de diagnóstico em inglês", () => {
        servirAgentes();
        prepararSessao("global", DUAS_ORGS);
        setLocale("en-US");
        montarRail("/orgs/org_1/agents/agt_1/instructions");

        expect(screen.getByRole("link", { name: "Knowledge Base" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_1/knowledge-base",
        );
        expect(screen.getByText("Diagnostics & Testing")).toBeInTheDocument();
    });
});

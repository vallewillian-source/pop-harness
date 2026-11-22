/**
 * Testes do menu lateral (RL-01 e RL-03).
 *
 * Cobrem a lista de navegação como o rail a usa: grupos e itens, o que exige organização ou
 * agente na URL, o item ativo, o contador de Organizações, o seletor de organização (com token de
 * org e passando pela guarda de alterações) e o rodapé.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation } from "react-router-dom";
import { useEffect } from "react";
import { ItenDeNavegacao, Rail } from "./Rail";
import { ITENS_DE_NAVEGACAO, type ContextoDeNavegacao, type ItemDeNavegacao } from "./navegacao";
import { GuardaAlteracoesProvider, useGuardaDeAlteracoes } from "./GuardaAlteracoes";
import { SessaoProvider } from "../sessao/Sessao";

// O rail consulta a saúde da instância (rede). Nos testes ela devolve uma versão fixa.
vi.mock("./saude", () => ({
    useSaude: () => ({ saude: { version: "0.9.1" }, semConexao: false }),
}));

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

function prepararSessao(escopo: "global" | "org", orgs: { id: string; name: string }[]) {
    window.sessionStorage.setItem(
        CHAVE_SESSAO,
        JSON.stringify({
            token: escopo === "org" ? "poph_teste" : "pophg_teste",
            escopo,
            orgs,
        }),
    );
}

function montarRail(caminho: string, opcoes: { sujo?: boolean; aoAbrirAtalhos?: () => void } = {}) {
    return render(
        <MemoryRouter initialEntries={[caminho]}>
            <SessaoProvider>
                <GuardaAlteracoesProvider>
                    <Rail aoAbrirAtalhos={opcoes.aoAbrirAtalhos ?? (() => {})} />
                    {opcoes.sujo && <Sujo />}
                    <Sonda />
                </GuardaAlteracoesProvider>
            </SessaoProvider>
        </MemoryRouter>,
    );
}

const DUAS_ORGS = [
    { id: "org_1", name: "Loja Antiga" },
    { id: "org_2", name: "Clínica Nova" },
];

beforeEach(() => {
    window.sessionStorage.clear();
});

afterEach(() => {
    window.sessionStorage.clear();
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

    it("com agente na URL, Chat de teste é link para a aba do agente, sem 'em breve'", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/agents/agt_1/instructions");

        const link = screen.getByRole("link", { name: "Chat de teste" });
        expect(link).toHaveAttribute("href", "/orgs/org_1/agents/agt_1/test-chat");
        expect(link).not.toHaveTextContent("em breve");
    });

    it("com agente na URL, Modelo e Jev é link sem 'em breve'", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/agents/agt_1/instructions");

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

    it("itens de estado emBreve continuam desabilitados e com 'em breve', mesmo com organização aberta", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/agents/agt_1/instructions");

        for (const rotulo of [
            "Inspeção de turnos",
            "WhatsApp e canais",
            "Plugins e extensões",
        ]) {
            const botao = screen.getByRole("button", { name: new RegExp(rotulo.replace(/[()]/g, "\\$&")) });
            expect(botao).toBeDisabled();
            expect(botao).toHaveTextContent("em breve");
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

    it("com agente na URL, Modelo e Jev é link para a aba do agente", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/agents/agt_1/instructions");

        expect(screen.getByRole("link", { name: "Modelo e Jev" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_1/model-jev",
        );
    });

    it("marca o item ativo com aria-current, sem marcar Organizações dentro de uma org", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        expect(screen.getByRole("link", { name: "Estrutura" })).toHaveAttribute(
            "aria-current",
            "page",
        );
        expect(screen.getByRole("link", { name: /Organizações/ })).not.toHaveAttribute(
            "aria-current",
        );
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

    it("com token global, o seletor oferece Todas e as organizações", () => {
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs");

        const seletor = screen.getByRole("combobox");
        expect(seletor).toBeEnabled();
        expect(screen.getByRole("option", { name: "Todas" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: "Loja Antiga" })).toBeInTheDocument();
    });

    it("com token de org, o seletor fica bloqueado na própria organização", () => {
        prepararSessao("org", [{ id: "org_1", name: "Loja Antiga" }]);
        montarRail("/orgs/org_1/structure");

        const seletor = screen.getByRole("combobox");
        expect(seletor).toBeDisabled();
        expect(seletor).toHaveValue("org_1");
        expect(screen.queryByRole("option", { name: "Todas" })).toBeNull();
    });

    it("trocar a org no seletor sem alterações pendentes navega para a nova org", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure");

        await usuario.selectOptions(screen.getByRole("combobox"), "org_2");

        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_2");
    });

    it("trocar a org no seletor com alterações pendentes pede confirmação antes de sair", async () => {
        const usuario = userEvent.setup();
        prepararSessao("global", DUAS_ORGS);
        montarRail("/orgs/org_1/structure", { sujo: true });

        await usuario.selectOptions(screen.getByRole("combobox"), "org_2");

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_1/structure");
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
});

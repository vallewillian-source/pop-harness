/**
 * Testes do header (RL-02 e RL-04).
 *
 * RL-02: escopo do token em texto conforme a sessão, estado da instância a partir do host da página,
 * "Sem conexão" quando a saúde falha, botão de sair que limpa a sessão e vai para `/login`, rótulo
 * "Modelo e Jev" na trilha e o selo "DEV LOCAL" fora do DOM.
 *
 * RL-04: busca rápida de organizações — atalho Ctrl/⌘+K, não roubar o atalho de outro campo, filtro
 * por nome e slug sem acento e sem caixa, setas + Enter pela guarda de alterações, Esc, mensagem
 * de sem resultado e nome acessível do campo.
 */

import { afterEach, describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { useEffect } from "react";
import { SessaoProvider } from "../sessao/Sessao";
import { ServidorFalso } from "../testes/servidor";
import { GuardaAlteracoesProvider, useGuardaDeAlteracoes } from "./GuardaAlteracoes";
import { Topbar } from "./Topbar";

const CHAVE_DA_SESSAO = "poph-console.sessao";

const SAUDE = {
    status: "ok",
    version: "0.4.2",
    uptime_seconds: 10,
    databases: { harness: 1, rag: 1 },
};

const TRES_ORGS = [
    { id: "org_1", name: "Loja Antiga", slug: "loja-antiga", status: "active" },
    { id: "org_2", name: "Clínica Nova", slug: "clinica-nova", status: "active" },
    { id: "org_3", name: "Padaria Sol", slug: "padaria-sol", status: "active" },
];

function prepararSessao(token: string, escopo: "global" | "org", orgs = TRES_ORGS.slice(0, 1)) {
    window.sessionStorage.setItem(
        CHAVE_DA_SESSAO,
        JSON.stringify({
            token,
            escopo,
            orgs,
        }),
    );
}

/** Mostra o caminho atual para provar se um Enter ou clique navegou. */
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

function montar(
    caminho = "/orgs",
    opcoes: { sujo?: boolean; comCampoExterno?: boolean } = {},
) {
    return render(
        <MemoryRouter initialEntries={[caminho]}>
            <SessaoProvider>
                <GuardaAlteracoesProvider>
                    {opcoes.comCampoExterno && <input aria-label="Campo de outra tela" />}
                    <Routes>
                        <Route
                            path="*"
                            element={
                                <Topbar orgs={[{ id: "org_1", name: "Loja Antiga" }]} aoAbrirAtalhos={() => {}} />
                            }
                        />
                        <Route path="/login" element={<p>Tela de login</p>} />
                    </Routes>
                    {opcoes.sujo && <Sujo />}
                    <Sonda />
                </GuardaAlteracoesProvider>
            </SessaoProvider>
        </MemoryRouter>,
    );
}

function campoDeBusca() {
    return screen.getByRole("combobox", { name: "Buscar organização" });
}

afterEach(() => {
    window.sessionStorage.removeItem(CHAVE_DA_SESSAO);
});

describe("Topbar", () => {
    it("mostra 'Token global' e o endereço local na sessão global", () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();

        expect(screen.getByText("Token global")).toBeInTheDocument();
        expect(screen.getByText(/^Local :\d+$/)).toBeInTheDocument();
        expect(screen.getByText("POP HARNESS")).toBeInTheDocument();
    });

    it("mostra 'Token de org' com token de organização", () => {
        prepararSessao("poph_org_teste", "org");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();

        expect(screen.getByText("Token de org")).toBeInTheDocument();
        expect(screen.queryByText("Token global")).not.toBeInTheDocument();
    });

    it("mostra 'Sem conexão' quando a consulta de saúde falha", async () => {
        prepararSessao("pophg_teste", "global");
        // Sem rota para /healthz: o fetch falso recusa, que é o caso de instância fora do ar.
        new ServidorFalso().instalar();

        montar();

        expect(await screen.findByText("Sem conexão")).toBeInTheDocument();
        expect(screen.queryByText(/^Local :\d+$/)).not.toBeInTheDocument();
    });

    it("o botão sair limpa a sessão e vai para /login", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();

        await usuario.click(screen.getByRole("button", { name: "Sair do Console" }));

        expect(await screen.findByText("Tela de login")).toBeInTheDocument();
        expect(window.sessionStorage.getItem(CHAVE_DA_SESSAO)).toBeNull();
    });

    it("a trilha mostra 'Modelo e Jev' na aba model-jev", () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar("/orgs/org_1/agents/agt_1/model-jev");

        expect(screen.getByText("Modelo e Jev")).toBeInTheDocument();
    });

    it("não renderiza mais o selo 'DEV LOCAL'", () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();

        expect(screen.queryByText("DEV LOCAL")).not.toBeInTheDocument();
    });
});

describe("busca rápida de organizações (RL-04)", () => {
    it("o campo tem nome acessível de verdade", () => {
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();

        expect(campoDeBusca()).toBeInTheDocument();
    });

    it("Ctrl+K foca o campo de busca", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();
        await usuario.keyboard("{Control>}k{/Control}");

        expect(campoDeBusca()).toHaveFocus();
    });

    it("⌘K (Meta+K) também foca o campo de busca", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();
        await usuario.keyboard("{Meta>}k{/Meta}");

        expect(campoDeBusca()).toHaveFocus();
    });

    it("o atalho não sequestra a combinação quando o foco está em outro campo de texto", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar("/orgs", { comCampoExterno: true });
        const outro = screen.getByRole("textbox", { name: "Campo de outra tela" });
        await usuario.click(outro);
        await usuario.keyboard("{Control>}k{/Control}");

        expect(outro).toHaveFocus();
        expect(campoDeBusca()).not.toHaveFocus();
    });

    it("filtra por nome e por slug, sem acento e sem caixa", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();
        await usuario.click(campoDeBusca());

        // Nome, com acento e caixa diferentes do cadastro.
        await usuario.type(campoDeBusca(), "CLINICA");
        let lista = screen.getByRole("listbox", { name: "Organizações encontradas" });
        expect(within(lista).getAllByRole("option")).toHaveLength(1);
        expect(within(lista).getByText("Clínica Nova")).toBeInTheDocument();

        // Slug, com caixa diferente.
        await usuario.clear(campoDeBusca());
        await usuario.type(campoDeBusca(), "LOJA-ANTI");
        lista = screen.getByRole("listbox", { name: "Organizações encontradas" });
        expect(within(lista).getAllByRole("option")).toHaveLength(1);
        expect(within(lista).getByText("Loja Antiga")).toBeInTheDocument();
    });

    it("setas escolhem a organização e Enter abre /orgs/:id", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();
        await usuario.type(campoDeBusca(), "a");
        // "a" acha as três; a primeira é destacada, uma seta abaixo põe a segunda.
        const opcoes = screen.getAllByRole("option");
        expect(opcoes).toHaveLength(3);
        expect(opcoes[0]).toHaveAttribute("aria-selected", "true");

        await usuario.keyboard("{ArrowDown}");
        expect(screen.getAllByRole("option")[1]).toHaveAttribute("aria-selected", "true");

        await usuario.keyboard("{Enter}");
        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_2");
    });

    it("Enter com alterações não salvas abre o aviso e não sai da tela", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar("/orgs", { sujo: true });
        await usuario.type(campoDeBusca(), "padaria");
        await usuario.keyboard("{Enter}");

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs");
        // Com o aviso aberto o resto da tela fica oculto para leitor de tela; o texto digitado segue.
        expect(screen.getByDisplayValue("padaria")).toBeInTheDocument();
    });

    it("Esc limpa o campo e fecha a lista", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();
        await usuario.type(campoDeBusca(), "loja");
        expect(screen.getByRole("listbox")).toBeInTheDocument();

        await usuario.keyboard("{Escape}");

        expect(campoDeBusca()).toHaveValue("");
        expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
        expect(screen.queryByRole("status")).not.toBeInTheDocument();
    });

    it("sem resultado mostra 'Nenhuma organização encontrada'", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();
        await usuario.type(campoDeBusca(), "zzzz");

        expect(screen.getByText("Nenhuma organização encontrada")).toBeInTheDocument();
        expect(screen.queryByRole("option")).not.toBeInTheDocument();
    });
});

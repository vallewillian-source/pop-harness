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
 *
 * CF-04: busca global (organizações, telas e comandos do poph em grupos), chip de versão, estado
 * com a RAM de /metrics e o caso de 503 (degradada).
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { useEffect } from "react";
import { SessaoProvider } from "../sessao/Sessao";
import { ServidorFalso } from "../testes/servidor";
import { GuardaAlteracoesProvider, useGuardaDeAlteracoes } from "./GuardaAlteracoes";
import { publishAgentModel } from "./agentModel";
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
    return screen.getByRole("combobox", { name: "Buscar organizações, telas e comandos" });
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
        // O endereço saiu do texto visível e foi para a dica do indicador de estado (CF-04).
        expect(screen.getByTitle(/^Local :\d+ · Token global/)).toBeInTheDocument();
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
        expect(screen.queryByText(/Engine:/)).not.toBeInTheDocument();
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
        let lista = screen.getByRole("listbox", { name: "Resultados da busca" });
        expect(within(lista).getAllByRole("option")).toHaveLength(1);
        expect(within(lista).getByText("Clínica Nova")).toBeInTheDocument();

        // Slug, com caixa diferente.
        await usuario.clear(campoDeBusca());
        await usuario.type(campoDeBusca(), "LOJA-ANTI");
        lista = screen.getByRole("listbox", { name: "Resultados da busca" });
        expect(within(lista).getAllByRole("option")).toHaveLength(1);
        expect(within(lista).getByText("Loja Antiga")).toBeInTheDocument();
    });

    it("setas escolhem a organização e Enter abre /orgs/:id", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();
        await usuario.type(campoDeBusca(), "a");
        // "a" acha as três organizações (no grupo Organizações); a primeira é destacada, uma seta
        // abaixo põe a segunda.
        const grupo = screen.getByRole("group", { name: "Organizações" });
        const opcoes = within(grupo).getAllByRole("option");
        expect(opcoes).toHaveLength(3);
        expect(opcoes[0]).toHaveAttribute("aria-selected", "true");

        await usuario.keyboard("{ArrowDown}");
        expect(within(grupo).getAllByRole("option")[1]).toHaveAttribute("aria-selected", "true");

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
        expect(screen.queryByText(/Nada encontrado/)).not.toBeInTheDocument();
    });

    it("sem resultado mostra 'Nenhuma organização encontrada'", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();

        montar();
        await usuario.type(campoDeBusca(), "zzzz");

        expect(screen.getByText("Nada encontrado para “zzzz”")).toBeInTheDocument();
        expect(screen.queryByRole("option")).not.toBeInTheDocument();
    });
});

describe("busca global, versão e estado da instância (CF-04)", () => {
    const SAUDE_COM_VERSAO = { ...SAUDE, version: "0.1.0" };

    afterEach(() => {
        vi.unstubAllGlobals();
        Reflect.deleteProperty(navigator, "clipboard");
    });

    it("mostra o chip de versão da instância ao lado da marca", async () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_COM_VERSAO }).instalar();

        montar();

        expect(await screen.findByText("v0.1.0")).toBeInTheDocument();
    });

    it("mostra 'Engine: 200 OK · 125 MB' com a memória lida de /metrics", async () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE_COM_VERSAO })
            .responder("GET", "/metrics", {
                status: 200,
                texto: "# TYPE poph_memoria_processo_bytes gauge\npoph_memoria_processo_bytes 131072000\n",
            })
            .instalar();

        montar();

        expect(await screen.findByText("Engine: 200 OK · 125 MB")).toBeInTheDocument();
    });

    it("sem a memória, mostra só 'Engine: 200 OK' (sem MB inventado)", async () => {
        prepararSessao("pophg_teste", "global");
        // Sem rota para /metrics: a consulta falha e a tela não mostra o número.
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_COM_VERSAO }).instalar();

        montar();

        expect(await screen.findByText("Engine: 200 OK")).toBeInTheDocument();
        expect(screen.queryByText(/MB/)).not.toBeInTheDocument();
    });

    it("saúde 503 aparece como 'degradada', e não como 'Sem conexão'", async () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso()
            .responder("GET", "/healthz", { status: 503, corpo: { ...SAUDE_COM_VERSAO, status: "degraded" } })
            .instalar();

        montar();

        expect(await screen.findByText("Engine: 503 · degradada")).toBeInTheDocument();
        expect(screen.queryByText("Sem conexão")).not.toBeInTheDocument();
    });

    it("agrupa o resultado em Organizações, Telas e Comandos do poph", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_COM_VERSAO }).instalar();

        montar();
        await usuario.type(campoDeBusca(), "organiz");

        // Nenhuma organização tem "organiz" no nome; a tela "Organizações" e os comandos sim.
        expect(screen.queryByRole("group", { name: "Organizações" })).not.toBeInTheDocument();
        expect(screen.getByRole("group", { name: "Telas" })).toBeInTheDocument();
        expect(screen.getByRole("group", { name: "Comandos do poph" })).toBeInTheDocument();
    });

    it("tela do menu só aparece na busca quando a organização está aberta na URL", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_COM_VERSAO }).instalar();

        montar("/orgs/org_2/structure");
        await usuario.type(campoDeBusca(), "produtos");

        // Com org aberta, "Produtos e catálogo" existe como tela; Enter abre a tela pela guarda.
        const tela = screen.getByRole("option", { name: /Produtos e catálogo/ });
        expect(tela).toBeInTheDocument();
        await usuario.click(tela);
        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_2/products");
    });

    it("sem organização aberta, as telas que exigem organização não aparecem", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_COM_VERSAO }).instalar();

        montar("/orgs");
        await usuario.type(campoDeBusca(), "produtos");

        expect(screen.queryByRole("option", { name: /Produtos e catálogo/ })).not.toBeInTheDocument();
        expect(screen.getByText("Nada encontrado para “produtos”")).toBeInTheDocument();
    });

    it("Enter num comando copia o texto e avisa que foi copiado", async () => {
        const usuario = userEvent.setup();
        const copiar = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(navigator, "clipboard", {
            value: { writeText: copiar },
            configurable: true,
        });
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_COM_VERSAO }).instalar();

        montar();
        await usuario.type(campoDeBusca(), "doctor");
        expect(screen.getByRole("group", { name: "Comandos do poph" })).toBeInTheDocument();
        await usuario.keyboard("{Enter}");

        expect(copiar).toHaveBeenCalledWith("poph doctor");
        expect(await screen.findByText("Comando copiado: poph doctor")).toBeInTheDocument();
        // Copiar comando não navega para outra tela.
        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs");
    });

    it("sem área de transferência, o comando aparece no aviso para copiar à mão", async () => {
        // Força o navegador sem clipboard (o userEvent instala um próprio, que aqui não serve).
        const usuario = userEvent.setup();
        Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_COM_VERSAO }).instalar();

        montar();
        await usuario.type(campoDeBusca(), "doctor");
        await usuario.keyboard("{Enter}");

        expect(await screen.findByText("Não consegui copiar. Comando: poph doctor")).toBeInTheDocument();
    });

    it("Enter numa organização continua abrindo /orgs/:id pela busca global", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_COM_VERSAO }).instalar();

        montar();
        await usuario.type(campoDeBusca(), "padaria");
        await usuario.keyboard("{Enter}");

        expect(screen.getByTestId("caminho")).toHaveTextContent("/orgs/org_3");
    });
});

describe("cabeçalho da visão do agente (CF-10)", () => {
    const SAUDE_OK = { ...SAUDE, version: "0.1.0" };

    it("a busca da visão do agente usa o placeholder contextual", () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/agents/agt_ctx/instructions");

        expect(campoDeBusca()).toHaveAttribute(
            "placeholder",
            "Buscar instruções, parâmetros ou turnos deste agente…",
        );
    });

    it("a busca da visão do agente abre a aba do agente pela rota real", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/agents/agt_busca/instructions");
        await usuario.type(campoDeBusca(), "roteamento");

        const aba = screen.getByRole("option", { name: "Roteamento & Habilidades" });
        await usuario.click(aba);

        expect(screen.getByTestId("caminho")).toHaveTextContent(
            "/orgs/org_1/agents/agt_busca/routing",
        );
    });

    it("a busca da visão do agente lista as seis abas com rota", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/agents/agt_abas/instructions");
        await usuario.type(campoDeBusca(), "e");

        const grupo = screen.getByRole("group", { name: "Telas" });
        const rotulos = within(grupo)
            .getAllByRole("option")
            .map((opcao) => opcao.textContent);

        expect(rotulos).toEqual(
            expect.arrayContaining([
                "Instruções & Prompts",
                "Base de Conhecimento",
                "Arquivos do Agente",
                "Roteamento & Habilidades",
                "Modelo & Parâmetros (Jev)",
                "Chat de Teste",
            ]),
        );
        expect(rotulos).not.toContain("Inspeção de Turnos");
    });

    it("fora da visão do agente a busca continua global", () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/structure");

        expect(campoDeBusca()).toHaveAttribute(
            "placeholder",
            "Buscar organizações, rotas ou comandos globais…",
        );
    });

    it("a busca da organização não oferece as abas do agente", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global", TRES_ORGS);
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/structure");
        await usuario.type(campoDeBusca(), "conhecimento");

        expect(screen.queryByRole("option", { name: /Base de Conhecimento/ })).not.toBeInTheDocument();
        expect(screen.getByText("Nada encontrado para “conhecimento”")).toBeInTheDocument();
    });

    it("o chip mostra o modelo do agente aberto e a saúde da instância", async () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();
        publishAgentModel("agt_chip", "gemini-3.8-flash");

        montar("/orgs/org_1/agents/agt_chip/instructions");

        expect(await screen.findByText("Modelo: gemini-3.8-flash")).toBeInTheDocument();
        expect(screen.getByText("200 OK")).toBeInTheDocument();
    });

    it("sem modelo publicado o chip mostra 'Modelo: —'", async () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/agents/agt_sem_modelo/instructions");

        expect(await screen.findByText("Modelo: —")).toBeInTheDocument();
    });

    it("o chip avisa quando a instância responde 503", async () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso()
            .responder("GET", "/healthz", { status: 503, corpo: { ...SAUDE_OK, status: "degraded" } })
            .instalar();
        publishAgentModel("agt_degradado", "gemini-3.8-flash");

        montar("/orgs/org_1/agents/agt_degradado/instructions");

        expect(await screen.findByText("503 · degradada")).toBeInTheDocument();
        expect(screen.getByText("Modelo: gemini-3.8-flash")).toHaveClass("poph-topbar__estado-texto");
    });

    it("o chip avisa quando a instância não responde", async () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().instalar();
        publishAgentModel("agt_sem_conexao", "gemini-3.8-flash");

        montar("/orgs/org_1/agents/agt_sem_conexao/instructions");

        expect(await screen.findByText("sem conexão")).toBeInTheDocument();
    });

    it("na visão do agente o botão de sair aparece com o texto 'Sair'", () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/agents/agt_sair/instructions");

        const botao = screen.getByRole("button", { name: "Sair do Console" });
        expect(within(botao).getByText("Sair")).toBeInTheDocument();
    });

    it("fora da visão do agente o botão de sair continua só com o ícone", () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/structure");

        expect(screen.getByRole("button", { name: "Sair do Console" })).toBeInTheDocument();
        expect(screen.queryByText("Sair")).not.toBeInTheDocument();
    });
});

describe("cabeçalho da visão do agente sem selo de token e com trilha curta (CF-11)", () => {
    const SAUDE_OK = { ...SAUDE, version: "0.1.0" };

    it("na visão do agente o selo de token sai e o escopo fica só na dica do chip", async () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/agents/agt_selo/instructions");

        expect(await screen.findByText(/Modelo:/)).toBeInTheDocument();
        expect(screen.queryByText("Token global")).not.toBeInTheDocument();
        expect(screen.queryByText("Token de org")).not.toBeInTheDocument();
        expect(screen.getByTitle(/^Local :\d+ · Token global/)).toBeInTheDocument();
    });

    it("na visão do agente com token de organização o selo também sai", async () => {
        prepararSessao("poph_org_teste", "org");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/agents/agt_selo_org/instructions");

        expect(await screen.findByText(/Modelo:/)).toBeInTheDocument();
        expect(screen.queryByText("Token de org")).not.toBeInTheDocument();
    });

    it("fora da visão do agente o selo de token continua", () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/structure");

        expect(screen.getByText("Token global")).toBeInTheDocument();
    });

    it("a trilha da visão do agente tem três passos, sem Organizações e sem Agentes", () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/agents/agt_trilha/instructions");

        const passos = [...document.querySelectorAll(".poph-topbar__passo")];
        expect(passos.map((passo) => passo.textContent?.replace("/", "").trim())).toEqual([
            "Loja Antiga",
            "Agente",
            "Instruções",
        ]);
        expect(screen.queryByText("Organizações")).not.toBeInTheDocument();
        expect(screen.queryByText("Agentes")).not.toBeInTheDocument();
    });

    it("na lista de agentes da organização a trilha continua como era", () => {
        prepararSessao("pophg_teste", "global");
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE_OK }).instalar();

        montar("/orgs/org_1/agents");

        const passos = [...document.querySelectorAll(".poph-topbar__passo")];
        expect(passos.map((passo) => passo.textContent?.replace("/", "").trim())).toEqual([
            "Organizações",
            "Loja Antiga",
            "Agentes",
        ]);
    });
});

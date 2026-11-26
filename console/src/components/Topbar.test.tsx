
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { useEffect } from "react";
import { setLocale } from "../i18n";
import { SessionProvider } from "../session/Session";
import { FakeServer } from "../testing/server";
import { GuardChangesProvider, useGuardChanges } from "../components/GuardChanges";
import { publishAgentModel } from "../components/agentModel";
import { Topbar } from "../components/Topbar";

const KEY_SESSION = "poph-console.sessao";

const HEALTH = {
    status: "ok",
    version: "0.4.2",
    uptime_seconds: 10,
    databases: { harness: 1, rag: 1 },
};

const THREE_ORGS = [
    { id: "org_1", name: "Loja Antiga", slug: "loja-antiga", status: "active" },
    { id: "org_2", name: "Clínica Nova", slug: "clinica-nova", status: "active" },
    { id: "org_3", name: "Padaria Sol", slug: "padaria-sol", status: "active" },
];

function prepareSession(token: string, scope: "global" | "org", orgs = THREE_ORGS.slice(0, 1)) {
    window.sessionStorage.setItem(
        KEY_SESSION,
        JSON.stringify({
            token,
            escopo: scope,
            orgs,
        }),
    );
}

function Probe() {
    const { pathname } = useLocation();
    return <span data-testid="path">{pathname}</span>;
}

function Dirty() {
    const { register } = useGuardChanges();
    useEffect(() => {
        register({ dirty: true, save: async () => true });
        return () => register(null);
    }, [register]);
    return null;
}

function build(
    path = "/orgs",
    options: { dirty?: boolean; withFieldExternal?: boolean } = {},
) {
    return render(
        <MemoryRouter initialEntries={[path]}>
            <SessionProvider>
                <GuardChangesProvider>
                    {options.withFieldExternal && <input aria-label="Campo de outra tela" />}
                    <Routes>
                        <Route
                            path="*"
                            element={
                                <Topbar orgs={[{ id: "org_1", name: "Loja Antiga" }]} onOpenShortcuts={() => {}} />
                            }
                        />
                        <Route path="/login" element={<p>Tela de login</p>} />
                    </Routes>
                    {options.dirty && <Dirty />}
                    <Probe />
                </GuardChangesProvider>
            </SessionProvider>
        </MemoryRouter>,
    );
}

function fieldSearch() {
    return screen.getByRole("combobox", { name: "Buscar organizações, telas e comandos" });
}

afterEach(() => {
    window.sessionStorage.removeItem(KEY_SESSION);
    window.localStorage.removeItem("poph-console.locale");
    setLocale("pt-BR");
});

describe("Topbar", () => {
    it("mostra 'Token global' e o endereço local na sessão global", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();

        expect(screen.getByText("Token global")).toBeInTheDocument();
        expect(screen.getByTitle(/^Local :\d+ · Token global/)).toBeInTheDocument();
        expect(screen.getByText("POP HARNESS")).toBeInTheDocument();
        expect(document.querySelector(".poph-topbar__monograma")).toBeNull();
    });

    it("mostra 'Token de org' com token de organização", () => {
        prepareSession("poph_org_teste", "org");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();

        expect(screen.getByText("Token de org")).toBeInTheDocument();
        expect(screen.queryByText("Token global")).not.toBeInTheDocument();
    });

    it("mostra 'Sem conexão' quando a consulta de saúde falha", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().install();

        build();

        expect(await screen.findByText("Sem conexão")).toBeInTheDocument();
        expect(screen.queryByText(/Engine:/)).not.toBeInTheDocument();
    });

    it("o botão sair limpa a sessão e vai para /login", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();

        await user.click(screen.getByRole("button", { name: "Sair do Console" }));

        expect(await screen.findByText("Tela de login")).toBeInTheDocument();
        expect(window.sessionStorage.getItem(KEY_SESSION)).toBeNull();
    });

    it("a trilha mostra 'Modelo e Jev' na aba model-jev", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build("/orgs/org_1/agents/agt_1/model-jev");

        expect(screen.getByText("Modelo e Jev")).toBeInTheDocument();
    });

    it("não renderiza mais o selo 'DEV LOCAL'", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();

        expect(screen.queryByText("DEV LOCAL")).not.toBeInTheDocument();
    });
});

describe("busca rápida de organizações (RL-04)", () => {
    it("o campo tem nome acessível de verdade", () => {
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();

        expect(fieldSearch()).toBeInTheDocument();
    });

    it("Ctrl+K foca o campo de busca", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();
        await user.keyboard("{Control>}k{/Control}");

        expect(fieldSearch()).toHaveFocus();
    });

    it("⌘K (Meta+K) também foca o campo de busca", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();
        await user.keyboard("{Meta>}k{/Meta}");

        expect(fieldSearch()).toHaveFocus();
    });

    it("o atalho não sequestra a combinação quando o foco está em outro campo de texto", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build("/orgs", { withFieldExternal: true });
        const other = screen.getByRole("textbox", { name: "Campo de outra tela" });
        await user.click(other);
        await user.keyboard("{Control>}k{/Control}");

        expect(other).toHaveFocus();
        expect(fieldSearch()).not.toHaveFocus();
    });

    it("filtra por nome e por slug, sem acento e sem caixa", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();
        await user.click(fieldSearch());

        await user.type(fieldSearch(), "CLINICA");
        let list = screen.getByRole("listbox", { name: "Resultados da busca" });
        expect(within(list).getAllByRole("option")).toHaveLength(1);
        expect(within(list).getByText("Clínica Nova")).toBeInTheDocument();

        await user.clear(fieldSearch());
        await user.type(fieldSearch(), "LOJA-ANTI");
        list = screen.getByRole("listbox", { name: "Resultados da busca" });
        expect(within(list).getAllByRole("option")).toHaveLength(1);
        expect(within(list).getByText("Loja Antiga")).toBeInTheDocument();
    });

    it("setas escolhem a organização e Enter abre /orgs/:id", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();
        await user.type(fieldSearch(), "a");
        const group = screen.getByRole("group", { name: "Organizações" });
        const options = within(group).getAllByRole("option");
        expect(options).toHaveLength(3);
        expect(options[0]).toHaveAttribute("aria-selected", "true");

        await user.keyboard("{ArrowDown}");
        expect(within(group).getAllByRole("option")[1]).toHaveAttribute("aria-selected", "true");

        await user.keyboard("{Enter}");
        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_2");
    });

    it("Enter com alterações não salvas abre o aviso e não sai da tela", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build("/orgs", { dirty: true });
        await user.type(fieldSearch(), "padaria");
        await user.keyboard("{Enter}");

        expect(await screen.findByText("Alterações não salvas")).toBeInTheDocument();
        expect(screen.getByTestId("path")).toHaveTextContent("/orgs");
        expect(screen.getByDisplayValue("padaria")).toBeInTheDocument();
    });

    it("Esc limpa o campo e fecha a lista", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();
        await user.type(fieldSearch(), "loja");
        expect(screen.getByRole("listbox")).toBeInTheDocument();

        await user.keyboard("{Escape}");

        expect(fieldSearch()).toHaveValue("");
        expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
        expect(screen.queryByText(/Nada encontrado/)).not.toBeInTheDocument();
    });

    it("sem resultado mostra 'Nenhuma organização encontrada'", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build();
        await user.type(fieldSearch(), "zzzz");

        expect(screen.getByText("Nada encontrado para “zzzz”")).toBeInTheDocument();
        expect(screen.queryByRole("option")).not.toBeInTheDocument();
    });
});

describe("busca global, versão e estado da instância (CF-04)", () => {
    const HEALTH_VERSION = { ...HEALTH, version: "0.1.0" };

    afterEach(() => {
        vi.unstubAllGlobals();
        Reflect.deleteProperty(navigator, "clipboard");
    });

    it("mostra o chip de versão da instância ao lado da marca", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_VERSION }).install();

        build();

        expect(await screen.findByText("v0.1.0")).toBeInTheDocument();
    });

    it("mostra 'Engine: 200 OK · 125 MB' com a memória lida de /metrics", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer()
            .respond("GET", "/healthz", { status: 200, body: HEALTH_VERSION })
            .respond("GET", "/metrics", {
                status: 200,
                texto: "# TYPE poph_memoria_processo_bytes gauge\npoph_memoria_processo_bytes 131072000\n",
            })
            .install();

        build();

        expect(await screen.findByText("Engine: 200 OK · 125 MB")).toBeInTheDocument();
    });

    it("sem a memória, mostra só 'Engine: 200 OK' (sem MB inventado)", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_VERSION }).install();

        build();

        expect(await screen.findByText("Engine: 200 OK")).toBeInTheDocument();
        expect(screen.queryByText(/MB/)).not.toBeInTheDocument();
    });

    it("saúde 503 aparece como 'degradada', e não como 'Sem conexão'", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer()
            .respond("GET", "/healthz", { status: 503, body: { ...HEALTH_VERSION, status: "degraded" } })
            .install();

        build();

        expect(await screen.findByText("Engine: 503 · degradada")).toBeInTheDocument();
        expect(screen.queryByText("Sem conexão")).not.toBeInTheDocument();
    });

    it("agrupa o resultado em Organizações, Telas e Comandos do poph", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_VERSION }).install();

        build();
        await user.type(fieldSearch(), "organiz");

        expect(screen.queryByRole("group", { name: "Organizações" })).not.toBeInTheDocument();
        expect(screen.getByRole("group", { name: "Telas" })).toBeInTheDocument();
        expect(screen.getByRole("group", { name: "Comandos do poph" })).toBeInTheDocument();
    });

    it("tela do menu só aparece na busca quando a organização está aberta na URL", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_VERSION }).install();

        build("/orgs/org_2/structure");
        await user.type(fieldSearch(), "produtos");

        const view = screen.getByRole("option", { name: /Produtos e catálogo/ });
        expect(view).toBeInTheDocument();
        await user.click(view);
        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_2/products");
    });

    it("sem organização aberta, as telas que exigem organização não aparecem", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_VERSION }).install();

        build("/orgs");
        await user.type(fieldSearch(), "produtos");

        expect(screen.queryByRole("option", { name: /Produtos e catálogo/ })).not.toBeInTheDocument();
        expect(screen.getByText("Nada encontrado para “produtos”")).toBeInTheDocument();
    });

    it("Enter num comando copia o texto e avisa que foi copiado", async () => {
        const user = userEvent.setup();
        const copy = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(navigator, "clipboard", {
            value: { writeText: copy },
            configurable: true,
        });
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_VERSION }).install();

        build();
        await user.type(fieldSearch(), "doctor");
        expect(screen.getByRole("group", { name: "Comandos do poph" })).toBeInTheDocument();
        await user.keyboard("{Enter}");

        expect(copy).toHaveBeenCalledWith("poph doctor");
        expect(await screen.findByText("Comando copiado: poph doctor")).toBeInTheDocument();
        expect(screen.getByTestId("path")).toHaveTextContent("/orgs");
    });

    it("sem área de transferência, o comando aparece no aviso para copiar à mão", async () => {
        const user = userEvent.setup();
        Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_VERSION }).install();

        build();
        await user.type(fieldSearch(), "doctor");
        await user.keyboard("{Enter}");

        expect(await screen.findByText("Não consegui copiar. Comando: poph doctor")).toBeInTheDocument();
    });

    it("Enter numa organização continua abrindo /orgs/:id pela busca global", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_VERSION }).install();

        build();
        await user.type(fieldSearch(), "padaria");
        await user.keyboard("{Enter}");

        expect(screen.getByTestId("path")).toHaveTextContent("/orgs/org_3");
    });
});

describe("cabeçalho da visão do agente (CF-10)", () => {
    const HEALTH_OK = { ...HEALTH, version: "0.1.0" };

    it("a busca da visão do agente usa o placeholder contextual", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/agents/agt_ctx/instructions");

        expect(fieldSearch()).toHaveAttribute(
            "placeholder",
            "Buscar instruções, parâmetros ou turnos deste agente…",
        );
    });

    it("a busca da visão do agente abre a aba do agente pela rota real", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/agents/agt_busca/instructions");
        await user.type(fieldSearch(), "roteamento");

        const tab = screen.getByRole("option", { name: "Roteamento & Habilidades" });
        await user.click(tab);

        expect(screen.getByTestId("path")).toHaveTextContent(
            "/orgs/org_1/agents/agt_busca/routing",
        );
    });

    it("a busca da visão do agente lista as seis abas com rota", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/agents/agt_abas/instructions");
        await user.type(fieldSearch(), "e");

        const group = screen.getByRole("group", { name: "Telas" });
        const labels = within(group)
            .getAllByRole("option")
            .map((option) => option.textContent);

        expect(labels).toEqual(
            expect.arrayContaining([
                "Instruções & Prompts",
                "Base de Conhecimento",
                "Arquivos do Agente",
                "Roteamento & Habilidades",
                "Modelo & Parâmetros (Jev)",
                "Chat de Teste",
            ]),
        );
        expect(labels).not.toContain("Inspeção de Turnos");
    });

    it("fora da visão do agente a busca continua global", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/structure");

        expect(fieldSearch()).toHaveAttribute(
            "placeholder",
            "Buscar organizações, rotas ou comandos globais…",
        );
    });

    it("a busca da organização não oferece as abas do agente", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/structure");
        await user.type(fieldSearch(), "conhecimento");

        expect(screen.queryByRole("option", { name: /Base de Conhecimento/ })).not.toBeInTheDocument();
        expect(screen.getByText("Nada encontrado para “conhecimento”")).toBeInTheDocument();
    });

    it("o chip mostra o modelo do agente aberto e a saúde da instância", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();
        publishAgentModel("agt_chip", "gemini-3.8-flash");

        build("/orgs/org_1/agents/agt_chip/instructions");

        expect(await screen.findByText("Modelo: gemini-3.8-flash")).toBeInTheDocument();
        expect(screen.getByText("200 OK")).toBeInTheDocument();
    });

    it("sem modelo publicado o chip mostra 'Modelo: —'", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/agents/agt_sem_modelo/instructions");

        expect(await screen.findByText("Modelo: —")).toBeInTheDocument();
    });

    it("o chip avisa quando a instância responde 503", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer()
            .respond("GET", "/healthz", { status: 503, body: { ...HEALTH_OK, status: "degraded" } })
            .install();
        publishAgentModel("agt_degradado", "gemini-3.8-flash");

        build("/orgs/org_1/agents/agt_degradado/instructions");

        expect(await screen.findByText("503 · degradada")).toBeInTheDocument();
        expect(screen.getByText("Modelo: gemini-3.8-flash")).toHaveClass("poph-topbar__state-text");
    });

    it("o chip avisa quando a instância não responde", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().install();
        publishAgentModel("agt_sem_conexao", "gemini-3.8-flash");

        build("/orgs/org_1/agents/agt_sem_conexao/instructions");

        expect(await screen.findByText("sem conexão")).toBeInTheDocument();
    });

    it("na visão do agente o botão de sair aparece com o texto 'Sair'", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/agents/agt_sair/instructions");

        const button = screen.getByRole("button", { name: "Sair do Console" });
        expect(within(button).getByText("Sair")).toBeInTheDocument();
    });

    it("fora da visão do agente o botão de sair continua só com o ícone", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/structure");

        expect(screen.getByRole("button", { name: "Sair do Console" })).toBeInTheDocument();
        expect(screen.queryByText("Sair")).not.toBeInTheDocument();
    });
});

describe("cabeçalho da visão do agente sem selo de token e com trilha curta (CF-11)", () => {
    const HEALTH_OK = { ...HEALTH, version: "0.1.0" };

    it("na visão do agente o selo de token sai e o escopo fica só na dica do chip", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/agents/agt_selo/instructions");

        expect(await screen.findByText(/Modelo:/)).toBeInTheDocument();
        expect(screen.queryByText("Token global")).not.toBeInTheDocument();
        expect(screen.queryByText("Token de org")).not.toBeInTheDocument();
        expect(screen.getByTitle(/^Local :\d+ · Token global/)).toBeInTheDocument();
    });

    it("na visão do agente com token de organização o selo também sai", async () => {
        prepareSession("poph_org_teste", "org");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/agents/agt_selo_org/instructions");

        expect(await screen.findByText(/Modelo:/)).toBeInTheDocument();
        expect(screen.queryByText("Token de org")).not.toBeInTheDocument();
    });

    it("fora da visão do agente o selo de token continua", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/structure");

        expect(screen.getByText("Token global")).toBeInTheDocument();
    });

    it("a trilha da visão do agente tem três passos, sem Organizações e sem Agentes", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/agents/agt_trilha/instructions");

        const steps = [...document.querySelectorAll(".poph-topbar__step")];
        expect(steps.map((step) => step.textContent?.replace("/", "").trim())).toEqual([
            "Loja Antiga",
            "Agente",
            "Instruções",
        ]);
        expect(screen.queryByText("Organizações")).not.toBeInTheDocument();
        expect(screen.queryByText("Agentes")).not.toBeInTheDocument();
    });

    it("na lista de agentes da organização a trilha continua como era", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        build("/orgs/org_1/agents");

        const steps = [...document.querySelectorAll(".poph-topbar__step")];
        expect(steps.map((step) => step.textContent?.replace("/", "").trim())).toEqual([
            "Organizações",
            "Loja Antiga",
            "Agentes",
        ]);
    });
});

describe("trilha do agente de três passos em 1280 (CF-11 rodada 1)", () => {
    const HEALTH_OK = { ...HEALTH, version: "0.1.0" };

    it("na visão do agente o cabeçalho e a trilha levam a marca --agent", async () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        const { container } = build("/orgs/org_1/agents/agt_largura/instructions");

        expect(await screen.findByText(/Modelo:/)).toBeInTheDocument();
        expect(container.querySelector("header.poph-topbar")).toHaveClass("poph-topbar--agent");
        expect(container.querySelector("nav.poph-topbar__trail")).toHaveClass(
            "poph-topbar__trail--agent",
        );
    });

    it("na rota de organização nem o cabeçalho nem a trilha levam a marca", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        const { container } = build("/orgs/org_1/structure");

        expect(container.querySelector("header.poph-topbar")).not.toHaveClass("poph-topbar--agent");
        expect(container.querySelector("nav.poph-topbar__trail")).not.toHaveClass(
            "poph-topbar__trail--agent",
        );
    });

    it("na lista de agentes da organização a trilha também fica sem a marca", () => {
        prepareSession("pophg_teste", "global");
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH_OK }).install();

        const { container } = build("/orgs/org_1/agents");

        expect(container.querySelector("nav.poph-topbar__trail")).not.toHaveClass(
            "poph-topbar__trail--agent",
        );
    });
});

describe("Topbar i18n", () => {
    it("switches Topbar, global search, and unsaved guard to en-US", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global", THREE_ORGS);
        new FakeServer().respond("GET", "/healthz", { status: 200, body: HEALTH }).install();

        build("/orgs", { dirty: true });

        await user.click(screen.getByRole("button", { name: "Inglês (Estados Unidos)" }));

        expect(screen.getByText("Global token")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Sign out of Console" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Keyboard shortcuts" })).toBeInTheDocument();
        expect(window.localStorage.getItem("poph-console.locale")).toBe("en-US");

        const searchInput = screen.getByRole("combobox", {
            name: "Search organizations, screens, and commands",
        });
        await user.type(searchInput, "Starts");
        expect(screen.getByRole("group", { name: "poph commands" })).toBeInTheDocument();
        expect(screen.getByText("Starts the instance HTTP server")).toBeInTheDocument();

        await user.clear(searchInput);
        await user.type(searchInput, "padaria");
        await user.keyboard("{Enter}");

        expect(await screen.findByText("Unsaved changes")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Discard changes" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Save and leave" })).toBeInTheDocument();
    });
});

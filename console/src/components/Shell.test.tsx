
import { afterEach, describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { setLocale } from "../i18n";
import { SessionProvider } from "../session/Session";
import { FakeServer, apiError } from "../testing/server";
import { GuardChangesProvider } from "../components/GuardChanges";
import { Shell } from "../components/Shell";

const KEY_SESSION = "poph-console.sessao";

const HEALTH = {
    status: "ok",
    version: "0.4.2",
    uptime_seconds: 10,
    databases: { harness: 1, rag: 1 },
};

const TWO_ORGS = [
    { id: "org_1", name: "Loja Antiga", slug: "loja-antiga", status: "active" },
    { id: "org_2", name: "Clínica Nova", slug: "clinica-nova", status: "active" },
];

const THREE_AGENTS = {
    items: [
        { id: "agt_1", name: "Atendente", org_id: "org_1" },
        { id: "agt_2", name: "Triagem", org_id: "org_1" },
        { id: "agt_3", name: "Vendas", org_id: "org_1" },
    ],
    next_cursor: null,
};

const AGENTS_WITH_STATUS = {
    items: [
        { id: "agt_1", name: "Atendente", org_id: "org_1", status: "active" },
        { id: "agt_2", name: "Triagem", org_id: "org_1", status: "inactive" },
        { id: "agt_3", name: "Vendas", org_id: "org_1", status: "archived" },
    ],
    next_cursor: null,
};

function prepareSession() {
    window.sessionStorage.setItem(
        KEY_SESSION,
        JSON.stringify({ token: "pophg_teste", escopo: "global", orgs: TWO_ORGS }),
    );
}

function build(path: string) {
    return render(
        <MemoryRouter initialEntries={[path]}>
            <SessionProvider>
                <GuardChangesProvider>
                    <Shell>
                        <p>Conteúdo da tela</p>
                    </Shell>
                </GuardChangesProvider>
            </SessionProvider>
        </MemoryRouter>,
    );
}

afterEach(() => {
    window.sessionStorage.removeItem(KEY_SESSION);
    window.localStorage.removeItem("poph-console.locale");
    setLocale("pt-BR");
});

describe("Casca do Console (HR-61)", () => {
    it("botão de ajuda no header abre o modal de atalhos", async () => {
        const user = userEvent.setup();
        prepareSession();
        new FakeServer().respond("GET", "/healthz", { status: 200, corpo: HEALTH }).install();
        build("/orgs");

        await user.click(screen.getByRole("button", { name: "Atalhos do teclado" }));

        expect(await screen.findByRole("dialog", { name: "Atalhos do Console" })).toBeInTheDocument();
    });

    it("mostra o total de agentes da organização aberta, com uma consulta só ao navegar", async () => {
        const user = userEvent.setup();
        prepareSession();
        const servidor = new FakeServer()
            .respond("GET", "/healthz", { status: 200, corpo: HEALTH })
            .respond("GET", "/admin/v1/orgs/org_1/agents", { status: 200, corpo: THREE_AGENTS })
            .install();
        build("/orgs/org_1/structure");

        const agents = screen.getByRole("link", { name: /Agentes/ });
        await waitFor(() => expect(agents).toHaveTextContent("3"));

        await user.click(screen.getByRole("link", { name: /Produtos e catálogo/ }));
        await user.click(screen.getByRole("link", { name: /Agentes/ }));
        expect(screen.getByRole("link", { name: /Agentes/ })).toHaveTextContent("3");
        expect(servidor.callsFor("GET", "/admin/v1/orgs/org_1/agents")).toHaveLength(1);
    });

    it("lista os agentes da organização no menu, sem segunda consulta", async () => {
        const user = userEvent.setup();
        prepareSession();
        const servidor = new FakeServer()
            .respond("GET", "/healthz", { status: 200, corpo: HEALTH })
            .respond("GET", "/admin/v1/orgs/org_1/agents", {
                status: 200,
                corpo: AGENTS_WITH_STATUS,
            })
            .install();
        build("/orgs/org_1/structure");

        expect(await screen.findByRole("link", { name: /Atendente/ })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents/agt_1/instructions",
        );
        expect(screen.getByTitle("Ativo")).toHaveClass("poph-rail__agent-dot--active");
        expect(screen.getByTitle("Arquivado")).toHaveClass("poph-rail__agent-dot--idle");

        await user.click(screen.getByRole("link", { name: /Produtos e catálogo/ }));
        await user.click(screen.getByRole("link", { name: /Agentes/ }));
        expect(screen.getByRole("link", { name: /Vendas/ })).toBeInTheDocument();
        expect(servidor.callsFor("GET", "/admin/v1/orgs/org_1/agents")).toHaveLength(1);
    });

    it("sem organização aberta, o item Agentes não tem contador nem faz consulta", () => {
        prepareSession();
        const servidor = new FakeServer()
            .respond("GET", "/healthz", { status: 200, corpo: HEALTH })
            .install();
        build("/orgs");

        const agents = screen.getByRole("button", { name: /Agentes/ });
        expect(agents).not.toHaveTextContent(/\d/);
        expect(servidor.callsFor("GET", "/agents")).toHaveLength(0);
    });

    it("se a consulta de agentes falha, o item fica sem contador e a tela segue", async () => {
        prepareSession();
        const servidor = new FakeServer()
            .respond("GET", "/healthz", { status: 200, corpo: HEALTH })
            .respond("GET", "/admin/v1/orgs/org_1/agents", apiError(500, "interno", "falha de teste"))
            .install();
        build("/orgs/org_1/structure");

        await waitFor(() =>
            expect(servidor.callsFor("GET", "/admin/v1/orgs/org_1/agents")).toHaveLength(1),
        );
        await new Promise((resolver) => setTimeout(resolver, 20));

        expect(screen.getByRole("link", { name: /Agentes/ })).not.toHaveTextContent(/\d/);
        expect(screen.getByText("Conteúdo da tela")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Atalhos do teclado" })).toBeInTheDocument();
    });

    it("switches the shortcuts modal to en-US when clicking EN", async () => {
        const user = userEvent.setup();
        prepareSession();
        new FakeServer().respond("GET", "/healthz", { status: 200, corpo: HEALTH }).install();
        build("/orgs");

        await user.click(screen.getByRole("button", { name: "Inglês (Estados Unidos)" }));
        await user.click(screen.getByRole("button", { name: "Keyboard shortcuts" }));

        expect(
            await screen.findByRole("dialog", { name: "Console shortcuts" }),
        ).toBeInTheDocument();
        expect(screen.getByText("Navigate between controls")).toBeInTheDocument();
        expect(screen.getByText("Space")).toBeInTheDocument();
    });
});

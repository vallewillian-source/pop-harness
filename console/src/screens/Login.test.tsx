
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useParams } from "react-router-dom";
import { setLocale } from "../i18n";
import { LAST_ORG_STORAGE_KEY, SessionProvider } from "../session/Session";
import { FakeServer } from "../testing/server";
import { Login } from "./Login";

const ORGS = [
    { id: "org_1", name: "Loja Antiga", slug: "loja-antiga", status: "active" },
    { id: "org_2", name: "Clínica Nova", slug: "clinica-nova", status: "active" },
];

function build() {
    return render(
        <MemoryRouter initialEntries={["/login"]}>
            <SessionProvider>
                <Routes>
                    <Route path="/login" element={<Login />} />
                    <Route path="/orgs" element={<p>lista de organizações</p>} />
                    <Route path="/orgs/:orgId" element={<PanelOrg />} />
                </Routes>
            </SessionProvider>
        </MemoryRouter>,
    );
}

function PanelOrg() {
    const { orgId = "" } = useParams();
    return <p>painel da organização {orgId}</p>;
}

function fieldToken() {
    return screen.getByLabelText(/Token administrativo/) as HTMLInputElement;
}

function serveOrgs(items: { id: string; name: string; slug: string; status: string }[]) {
    return new FakeServer()
        .respond("GET", "/admin/v1/orgs", {
            status: 200,
            body: { items, next_cursor: null },
        })
        .install();
}

async function signInWith(token: string) {
    await userEvent.type(fieldToken(), token);
    await userEvent.click(screen.getByRole("button", { name: "Entrar no Console" }));
}

afterEach(() => {
    vi.unstubAllGlobals();
    window.sessionStorage.clear();
    window.localStorage.clear();
    setLocale("pt-BR");
});

describe("Login", () => {
    it("mostra a marca POP HARNESS e não usa cartão", () => {
        build();

        expect(screen.getByText("POP HARNESS")).toBeInTheDocument();
        expect(screen.getByRole("heading", { level: 1, name: "Console do POP Harness" })).toBeInTheDocument();
        expect(document.querySelector(".sofa-card")).toBeNull();
    });

    it("o campo do token tem rótulo visível e é o foco inicial", () => {
        build();

        expect(fieldToken()).toHaveFocus();
        expect(fieldToken().type).toBe("password");
    });

    it("mostra o aviso de uso interno", () => {
        build();

        expect(screen.getByText("Acesso administrativo por token. Uso interno.")).toBeInTheDocument();
    });

    it("o botão de mostrar token alterna a visibilidade do campo", async () => {
        build();

        await userEvent.click(screen.getByRole("button", { name: "Mostrar token" }));

        expect(fieldToken().type).toBe("text");
        expect(screen.getByRole("button", { name: "Ocultar token" })).toBeInTheDocument();
    });

    it("sem organização na sessão, token válido entra e leva para /orgs, guardando a sessão na aba", async () => {
        serveOrgs([]);
        build();

        await signInWith("pophg_teste");

        expect(await screen.findByText("lista de organizações")).toBeInTheDocument();
        const recorded = JSON.parse(window.sessionStorage.getItem("poph-console.sessao") ?? "{}");
        expect(recorded.token).toBe("pophg_teste");
        expect(recorded.escopo).toBe("global");
    });

    it("com organizações e nenhuma lembrada, token válido abre a primeira da lista", async () => {
        serveOrgs(ORGS);
        build();

        await signInWith("pophg_teste");

        expect(await screen.findByText("painel da organização org_1")).toBeInTheDocument();
        expect(window.localStorage.getItem(LAST_ORG_STORAGE_KEY)).toBeNull();
    });

    it("token válido abre a última organização lembrada em localStorage", async () => {
        window.localStorage.setItem(LAST_ORG_STORAGE_KEY, "org_2");
        serveOrgs(ORGS);
        build();

        await signInWith("pophg_teste");

        expect(await screen.findByText("painel da organização org_2")).toBeInTheDocument();
        expect(window.localStorage.getItem(LAST_ORG_STORAGE_KEY)).toBe("org_2");
    });

    it("registro lembrado que não está mais na sessão cai na primeira organização", async () => {
        window.localStorage.setItem(LAST_ORG_STORAGE_KEY, "org_que_sumiu");
        serveOrgs(ORGS);
        build();

        await signInWith("pophg_teste");

        expect(await screen.findByText("painel da organização org_1")).toBeInTheDocument();
    });

    it("token recusado mostra o erro no campo e não navega", async () => {
        const servidor = new FakeServer();
        servidor
            .respond("GET", "/admin/v1/orgs", { status: 401, body: { error: "unauthorized" } })
            .install();
        build();

        await signInWith("pophg_errado");

        expect(
            await screen.findByText(/Token administrativo inválido ou revogado/),
        ).toBeInTheDocument();
        expect(screen.queryByText("lista de organizações")).toBeNull();
        expect(fieldToken()).toBeInTheDocument();
        expect(window.sessionStorage.getItem("poph-console.sessao")).toBeNull();
    });

    it("falha de conexão mostra o aviso 'Sem conexão' e fica na tela", async () => {
        vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
        build();

        await signInWith("pophg_teste");

        expect(await screen.findByText("Sem conexão")).toBeInTheDocument();
        expect(screen.getByText(/Não foi possível alcançar a instância/)).toBeInTheDocument();
        await waitFor(() => expect(fieldToken()).toBeInTheDocument());
        expect(screen.queryByText("lista de organizações")).toBeNull();
    });

    it("switches to en-US when clicking EN and persists in localStorage", async () => {
        build();

        const enButton = screen.getByRole("button", { name: "Inglês (Estados Unidos)" });
        await userEvent.click(enButton);

        expect(
            screen.getByRole("heading", { level: 1, name: "POP Harness Console" }),
        ).toBeInTheDocument();
        expect(
            screen.getByText("Token-based administrative access. Internal use."),
        ).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Sign in to Console" })).toBeInTheDocument();
        expect(screen.getByLabelText(/Administrative token/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "English (United States)" })).toHaveAttribute(
            "aria-pressed",
            "true",
        );
        expect(document.documentElement.lang).toBe("en-US");
        expect(window.localStorage.getItem("poph-console.locale")).toBe("en-US");

        await userEvent.click(screen.getByRole("button", { name: "Portuguese (Brazil)" }));
        expect(
            screen.getByRole("heading", { level: 1, name: "Console do POP Harness" }),
        ).toBeInTheDocument();
    });
});

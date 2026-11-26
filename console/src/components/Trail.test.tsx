
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { setLocale } from "../i18n";
import { Trail, trail } from "../components/Trail";
import { publishNameAgent } from "../components/nameAgent";

const ORGS = [{ id: "org_1", name: "Clínica Aurora" }];

afterEach(() => {
    cleanup();
    window.localStorage.removeItem("poph-console.locale");
    setLocale("pt-BR");
});

describe("trilha (função)", () => {
    it("em rota de agente tem três passos: organização, agente e aba", () => {
        const crumbs = trail("/orgs/org_1/agents/agt_1/instructions", ORGS, "Bia");

        expect(crumbs.map((crumb) => crumb.rotulo)).toEqual([
            "Clínica Aurora",
            "Bia",
            "Instruções",
        ]);
        expect(crumbs[0]?.para).toBe("/orgs/org_1");
        expect(crumbs[1]?.para).toBe("/orgs/org_1/agents/agt_1");
        expect(crumbs[2]?.para).toBeUndefined();
    });

    it("na raiz do agente o agente é a migalha atual, sem link", () => {
        const crumbs = trail("/orgs/org_1/agents/agt_1", ORGS, "Bia");

        expect(crumbs.map((crumb) => crumb.rotulo)).toEqual(["Clínica Aurora", "Bia"]);
        expect(crumbs[0]?.para).toBe("/orgs/org_1");
        expect(crumbs[1]?.para).toBeUndefined();
    });

    it("sem nome publicado, mostra Agente no lugar do nome", () => {
        const crumbs = trail("/orgs/org_1/agents/agt_1/model-jev", ORGS);

        expect(crumbs.map((crumb) => crumb.rotulo)).toEqual([
            "Clínica Aurora",
            "Agente",
            "Modelo e Jev",
        ]);
    });

    it("a aba Chat de teste e as telas fora do agente mantêm os rótulos", () => {
        expect(trail("/orgs/org_1/agents/agt_1/test-chat", ORGS, "Bia").map((m) => m.rotulo)).toContain("Chat de teste");
        expect(trail("/orgs/org_1/structure", ORGS).map((m) => m.rotulo)).toEqual([
            "Organizações",
            "Clínica Aurora",
            "Estrutura",
        ]);
        expect(trail("/orgs/org_1/operacao", ORGS).map((m) => m.rotulo)).toEqual([
            "Organizações",
            "Clínica Aurora",
            "Saúde da instância",
        ]);
    });

    it("a lista de agentes da organização continua com Organizações e Agentes", () => {
        const crumbs = trail("/orgs/org_1/agents", ORGS);

        expect(crumbs.map((crumb) => crumb.rotulo)).toEqual([
            "Organizações",
            "Clínica Aurora",
            "Agentes",
        ]);
        expect(crumbs[2]?.para).toBe("/orgs/org_1/agents");
    });
});

describe("trilha (tela) com o nome publicado pela aba do agente", () => {
    it("mostra o nome do agente da URL depois que a aba publica", () => {
        publishNameAgent("agt_1", "Bia");
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/agents/agt_1/routing"]}>
                <Trail orgs={ORGS} />
            </MemoryRouter>,
        );

        expect(screen.getByText("Bia")).toBeInTheDocument();
        expect(screen.getByText("Roteamento")).toBeInTheDocument();
        expect(screen.queryByText("Agente")).not.toBeInTheDocument();
    });

    it("não mostra o nome de outro agente: com a URL de outro id, cai no rótulo Agente", () => {
        publishNameAgent("agt_9", "Outro agente");
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/agents/agt_1/routing"]}>
                <Trail orgs={ORGS} />
            </MemoryRouter>,
        );

        expect(screen.getByText("Agente")).toBeInTheDocument();
        expect(screen.queryByText("Outro agente")).not.toBeInTheDocument();
    });
});

describe("trilha curta (rota de agente) e prioridade de corte", () => {
    const ROUTE = "/orgs/org_1/agents/agt_1/instructions";

    function build() {
        publishNameAgent("agt_1", "Bia");
        return render(
            <MemoryRouter initialEntries={[ROUTE]}>
                <Trail orgs={ORGS} />
            </MemoryRouter>,
        );
    }

    it("os três passos aparecem, na ordem do caminho", () => {
        build();
        const steps = document.querySelectorAll(".poph-topbar__step");
        expect([...steps].map((step) => step.textContent?.replace("/", "").trim())).toEqual([
            "Clínica Aurora",
            "Bia",
            "Instruções",
        ]);
    });

    it("organização e agente são links; a aba é a tela atual", () => {
        build();
        expect(screen.getByRole("link", { name: "Clínica Aurora" })).toHaveAttribute("href", "/orgs/org_1");
        expect(screen.getByRole("link", { name: "Bia" })).toHaveAttribute("href", "/orgs/org_1/agents/agt_1");
        expect(screen.queryByRole("link", { name: "Instruções" })).not.toBeInTheDocument();
    });

    it("a tela atual fica inteira: marcada como atual, sem title e sem encolher", () => {
        build();
        const current = screen.getByText("Instruções");
        expect(current).toHaveClass("poph-topbar__current");
        expect(current).toHaveAttribute("aria-current", "page");
        expect(current).not.toHaveAttribute("title");
        expect(current.parentElement).toHaveClass("poph-topbar__step--current");
        expect((current.parentElement as HTMLElement).style.flexShrink).toBe("0");
    });

    it("as migalhas anteriores levam o texto completo no title e encolhem por prioridade", () => {
        build();
        const steps = [...document.querySelectorAll<HTMLElement>(".poph-topbar__step")];
        expect(steps.map((step) => step.style.flexShrink)).toEqual(["2", "1", "0"]);
        expect(screen.getByText("Clínica Aurora")).toHaveAttribute("title", "Clínica Aurora");
        expect(screen.getByText("Bia")).toHaveAttribute("title", "Bia");
    });

    it("cada passo carrega o próprio separador, então esconder um passo não deixa '/' solto", () => {
        build();
        const steps = [...document.querySelectorAll(".poph-topbar__step")];
        expect(steps[0]?.querySelector(".poph-topbar__separator")).toBeNull();
        for (const step of steps.slice(1)) {
            expect(step.querySelector(".poph-topbar__separator")?.textContent).toBe("/");
        }
    });

    it("em caminho curto a última migalha também é a atual e não tem title", () => {
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/structure"]}>
                <Trail orgs={ORGS} />
            </MemoryRouter>,
        );
        const current = screen.getByText("Estrutura");
        expect(current).toHaveAttribute("aria-current", "page");
        expect(current).not.toHaveAttribute("title");
    });
});

describe("marca da trilha do agente", () => {
    it("em rota de agente a trilha leva a marca --agent", () => {
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/agents/agt_1/instructions"]}>
                <Trail orgs={ORGS} />
            </MemoryRouter>,
        );
        expect(document.querySelector("nav.poph-topbar__trail")).toHaveClass(
            "poph-topbar__trail--agent",
        );
    });

    it("na tela da organização e na lista de agentes a trilha fica sem a marca", () => {
        for (const path of ["/orgs/org_1/structure", "/orgs/org_1/agents"]) {
            render(
                <MemoryRouter initialEntries={[path]}>
                    <Trail orgs={ORGS} />
                </MemoryRouter>,
            );
            expect(document.querySelector("nav.poph-topbar__trail")).not.toHaveClass(
                "poph-topbar__trail--agent",
            );
            cleanup();
        }
    });
});

describe("Trilha i18n", () => {
    it("translates breadcrumbs and agent tabs to en-US", () => {
        setLocale("en-US");

        expect(trail("/orgs/org_1/agents/agt_1/model-jev", ORGS).map((m) => m.rotulo)).toEqual([
            "Clínica Aurora",
            "Agent",
            "Model and Jev",
        ]);
        expect(trail("/orgs/org_1/operacao", ORGS).map((m) => m.rotulo)).toEqual([
            "Organizations",
            "Clínica Aurora",
            "Instance health",
        ]);

        render(
            <MemoryRouter initialEntries={["/orgs/org_1/structure"]}>
                <Trail orgs={ORGS} />
            </MemoryRouter>,
        );

        expect(screen.getByRole("navigation", { name: "Breadcrumb navigation" })).toBeInTheDocument();
        expect(screen.getByText("Structure")).toBeInTheDocument();
    });
});

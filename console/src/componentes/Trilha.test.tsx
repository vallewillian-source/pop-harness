/**
 * Trilha da barra superior: em rota de agente ela tem três passos (CF-11) — organização / agente /
 * aba —, com o nome do agente publicado pela aba (HR-49b); sem nome publicado, mostra "Agente".
 */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { setLocale } from "../i18n";
import { Trilha, trilha } from "./Trilha";
import { publicarNomeDoAgente } from "./nomeDoAgente";

const ORGS = [{ id: "org_1", name: "Clínica Aurora" }];

afterEach(() => {
    cleanup();
    window.localStorage.removeItem("poph-console.locale");
    setLocale("pt-BR");
});

describe("trilha (função)", () => {
    it("em rota de agente tem três passos: organização, agente e aba", () => {
        const migalhas = trilha("/orgs/org_1/agents/agt_1/instructions", ORGS, "Bia");

        expect(migalhas.map((migalha) => migalha.rotulo)).toEqual([
            "Clínica Aurora",
            "Bia",
            "Instruções",
        ]);
        expect(migalhas[0]?.para).toBe("/orgs/org_1");
        expect(migalhas[1]?.para).toBe("/orgs/org_1/agents/agt_1");
        expect(migalhas[2]?.para).toBeUndefined();
    });

    it("na raiz do agente o agente é a migalha atual, sem link", () => {
        const migalhas = trilha("/orgs/org_1/agents/agt_1", ORGS, "Bia");

        expect(migalhas.map((migalha) => migalha.rotulo)).toEqual(["Clínica Aurora", "Bia"]);
        expect(migalhas[0]?.para).toBe("/orgs/org_1");
        expect(migalhas[1]?.para).toBeUndefined();
    });

    it("sem nome publicado, mostra Agente no lugar do nome", () => {
        const migalhas = trilha("/orgs/org_1/agents/agt_1/model-jev", ORGS);

        expect(migalhas.map((migalha) => migalha.rotulo)).toEqual([
            "Clínica Aurora",
            "Agente",
            "Modelo e Jev",
        ]);
    });

    it("a aba Chat de teste e as telas fora do agente mantêm os rótulos", () => {
        expect(trilha("/orgs/org_1/agents/agt_1/test-chat", ORGS, "Bia").map((m) => m.rotulo)).toContain("Chat de teste");
        expect(trilha("/orgs/org_1/structure", ORGS).map((m) => m.rotulo)).toEqual([
            "Organizações",
            "Clínica Aurora",
            "Estrutura",
        ]);
        expect(trilha("/orgs/org_1/operacao", ORGS).map((m) => m.rotulo)).toEqual([
            "Organizações",
            "Clínica Aurora",
            "Saúde da instância",
        ]);
    });

    it("a lista de agentes da organização continua com Organizações e Agentes", () => {
        const migalhas = trilha("/orgs/org_1/agents", ORGS);

        expect(migalhas.map((migalha) => migalha.rotulo)).toEqual([
            "Organizações",
            "Clínica Aurora",
            "Agentes",
        ]);
        expect(migalhas[2]?.para).toBe("/orgs/org_1/agents");
    });
});

describe("trilha (tela) com o nome publicado pela aba do agente", () => {
    it("mostra o nome do agente da URL depois que a aba publica", () => {
        publicarNomeDoAgente("agt_1", "Bia");
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/agents/agt_1/routing"]}>
                <Trilha orgs={ORGS} />
            </MemoryRouter>,
        );

        expect(screen.getByText("Bia")).toBeInTheDocument();
        expect(screen.getByText("Roteamento")).toBeInTheDocument();
        expect(screen.queryByText("Agente")).not.toBeInTheDocument();
    });

    it("não mostra o nome de outro agente: com a URL de outro id, cai no rótulo Agente", () => {
        publicarNomeDoAgente("agt_9", "Outro agente");
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/agents/agt_1/routing"]}>
                <Trilha orgs={ORGS} />
            </MemoryRouter>,
        );

        expect(screen.getByText("Agente")).toBeInTheDocument();
        expect(screen.queryByText("Outro agente")).not.toBeInTheDocument();
    });
});

/**
 * CF-11: em rota de agente a trilha tem três passos e deixa de cortar os nomes. A tela atual fica
 * inteira, as anteriores encolhem por prioridade e cada uma leva o texto completo no `title`.
 */
describe("trilha curta (rota de agente) e prioridade de corte", () => {
    const ROTA = "/orgs/org_1/agents/agt_1/instructions";

    function montar() {
        publicarNomeDoAgente("agt_1", "Bia");
        return render(
            <MemoryRouter initialEntries={[ROTA]}>
                <Trilha orgs={ORGS} />
            </MemoryRouter>,
        );
    }

    it("os três passos aparecem, na ordem do caminho", () => {
        montar();
        const passos = document.querySelectorAll(".poph-topbar__passo");
        expect([...passos].map((passo) => passo.textContent?.replace("/", "").trim())).toEqual([
            "Clínica Aurora",
            "Bia",
            "Instruções",
        ]);
    });

    it("organização e agente são links; a aba é a tela atual", () => {
        montar();
        expect(screen.getByRole("link", { name: "Clínica Aurora" })).toHaveAttribute("href", "/orgs/org_1");
        expect(screen.getByRole("link", { name: "Bia" })).toHaveAttribute("href", "/orgs/org_1/agents/agt_1");
        expect(screen.queryByRole("link", { name: "Instruções" })).not.toBeInTheDocument();
    });

    it("a tela atual fica inteira: marcada como atual, sem title e sem encolher", () => {
        montar();
        const atual = screen.getByText("Instruções");
        expect(atual).toHaveClass("poph-topbar__atual");
        expect(atual).toHaveAttribute("aria-current", "page");
        expect(atual).not.toHaveAttribute("title");
        expect(atual.parentElement).toHaveClass("poph-topbar__passo--atual");
        expect((atual.parentElement as HTMLElement).style.flexShrink).toBe("0");
    });

    it("as migalhas anteriores levam o texto completo no title e encolhem por prioridade", () => {
        montar();
        const passos = [...document.querySelectorAll<HTMLElement>(".poph-topbar__passo")];
        // A distância até a tela atual é o `flex-shrink`: a mais longe encolhe primeiro.
        expect(passos.map((passo) => passo.style.flexShrink)).toEqual(["2", "1", "0"]);
        // Texto completo no title, para o nome cortado continuar acessível.
        expect(screen.getByText("Clínica Aurora")).toHaveAttribute("title", "Clínica Aurora");
        expect(screen.getByText("Bia")).toHaveAttribute("title", "Bia");
    });

    it("cada passo carrega o próprio separador, então esconder um passo não deixa '/' solto", () => {
        montar();
        const passos = [...document.querySelectorAll(".poph-topbar__passo")];
        expect(passos[0]?.querySelector(".poph-topbar__separador")).toBeNull();
        for (const passo of passos.slice(1)) {
            expect(passo.querySelector(".poph-topbar__separador")?.textContent).toBe("/");
        }
    });

    it("em caminho curto a última migalha também é a atual e não tem title", () => {
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/structure"]}>
                <Trilha orgs={ORGS} />
            </MemoryRouter>,
        );
        const atual = screen.getByText("Estrutura");
        expect(atual).toHaveAttribute("aria-current", "page");
        expect(atual).not.toHaveAttribute("title");
    });
});

describe("marca da trilha do agente", () => {
    it("em rota de agente a trilha leva a marca --agent", () => {
        render(
            <MemoryRouter initialEntries={["/orgs/org_1/agents/agt_1/instructions"]}>
                <Trilha orgs={ORGS} />
            </MemoryRouter>,
        );
        expect(document.querySelector("nav.poph-topbar__trilha")).toHaveClass(
            "poph-topbar__trilha--agent",
        );
    });

    it("na tela da organização e na lista de agentes a trilha fica sem a marca", () => {
        for (const caminho of ["/orgs/org_1/structure", "/orgs/org_1/agents"]) {
            render(
                <MemoryRouter initialEntries={[caminho]}>
                    <Trilha orgs={ORGS} />
                </MemoryRouter>,
            );
            expect(document.querySelector("nav.poph-topbar__trilha")).not.toHaveClass(
                "poph-topbar__trilha--agent",
            );
            cleanup();
        }
    });
});

describe("Trilha i18n", () => {
    it("translates breadcrumbs and agent tabs to en-US", () => {
        setLocale("en-US");

        expect(trilha("/orgs/org_1/agents/agt_1/model-jev", ORGS).map((m) => m.rotulo)).toEqual([
            "Clínica Aurora",
            "Agent",
            "Model and Jev",
        ]);
        expect(trilha("/orgs/org_1/operacao", ORGS).map((m) => m.rotulo)).toEqual([
            "Organizations",
            "Clínica Aurora",
            "Instance health",
        ]);

        render(
            <MemoryRouter initialEntries={["/orgs/org_1/structure"]}>
                <Trilha orgs={ORGS} />
            </MemoryRouter>,
        );

        expect(screen.getByRole("navigation", { name: "Breadcrumb navigation" })).toBeInTheDocument();
        expect(screen.getByText("Structure")).toBeInTheDocument();
    });
});

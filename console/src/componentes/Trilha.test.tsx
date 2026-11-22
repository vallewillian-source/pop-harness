/**
 * Trilha da barra superior com o nome do agente (HR-49b): o nome entra entre "Agentes" e a aba, só
 * quando a aba do agente da URL já publicou o nome dele; sem isso, mostra "Agente".
 */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { Trilha, trilha } from "./Trilha";
import { publicarNomeDoAgente } from "./nomeDoAgente";

const ORGS = [{ id: "org_1", name: "Clínica Aurora" }];

afterEach(() => {
    cleanup();
});

describe("trilha (função)", () => {
    it("põe o nome do agente entre Agentes e a aba, com o nome sem link", () => {
        const migalhas = trilha("/orgs/org_1/agents/agt_1/instructions", ORGS, "Bia");

        expect(migalhas.map((migalha) => migalha.rotulo)).toEqual([
            "Organizações",
            "Clínica Aurora",
            "Agentes",
            "Bia",
            "Instruções",
        ]);
        expect(migalhas[3]?.para).toBeUndefined();
    });

    it("sem nome publicado, mostra Agente no lugar do nome", () => {
        const migalhas = trilha("/orgs/org_1/agents/agt_1/model-jev", ORGS);

        expect(migalhas.map((migalha) => migalha.rotulo)).toEqual([
            "Organizações",
            "Clínica Aurora",
            "Agentes",
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

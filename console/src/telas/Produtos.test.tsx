/**
 * Testes da tela de produtos (HR-42): a listagem pede todas as situações e pagina sem `limit`, a
 * contagem de agentes segue a regra do servidor (sem categoria vinculada = vê tudo), os filtros
 * por situação, categoria e nome, e os estados vazio, sem resultado e de erro.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { setLocale } from "../i18n";
import { ServidorFalso, erroApi } from "../testes/servidor";
import { Produtos } from "./Produtos";

const ORG = "org_1";
const BASE = `/admin/v1/orgs/${ORG}`;

function produto(id: string, nome: string, categoria: string, sobrescrito: Record<string, unknown> = {}) {
    return {
        id,
        org_id: ORG,
        category_id: categoria,
        name: nome,
        slug: nome.toLowerCase().replace(/\s+/g, "-"),
        aliases: [],
        description: "",
        price_text: "R$ 450,00",
        price_cents: 45000,
        unit: "",
        includes: "",
        excludes: "",
        conditions: "",
        tags: [],
        status: "active",
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...sobrescrito,
    };
}

function categoria(id: string, nome: string) {
    return { id, org_id: ORG, name: nome, description: "", status: "active", version: 1, created_at: 0, updated_at: 0 };
}

const PRODUTOS = [
    produto("prod_1", "Clareamento dental", "cat_1", { aliases: ["clareamento"], unit: "por sessão" }),
    produto("prod_2", "Implante unitário", "cat_2", {
        status: "inactive",
        price_cents: 320000,
        price_text: "R$ 3.200,00",
    }),
];

const CATEGORIAS = [categoria("cat_1", "Ortodontia"), categoria("cat_2", "Implantes")];

/** ag_1 sem categoria (vê tudo); ag_2 só com cat_1 (vê o clareamento). */
const AGENTES = [
    { id: "ag_1", org_id: ORG, name: "Atendente geral", product_category_ids: [] },
    { id: "ag_2", org_id: ORG, name: "Ortodontia", product_category_ids: ["cat_1"] },
];

function servidorPadrao(produtos = PRODUTOS, categorias = CATEGORIAS) {
    return new ServidorFalso()
        .responder("GET", `${BASE}/agents/ag_1`, { status: 200, corpo: AGENTES[0] })
        .responder("GET", `${BASE}/agents/ag_2`, { status: 200, corpo: AGENTES[1] })
        .responder("GET", `${BASE}/agents`, {
            status: 200,
            corpo: { items: AGENTES.map(({ id, name }) => ({ id, name })), next_cursor: null },
        })
        .responder("GET", `${BASE}/products`, {
            status: 200,
            corpo: { items: produtos, next_cursor: null },
        })
        .responder("GET", `${BASE}/categories`, {
            status: 200,
            corpo: { items: categorias, next_cursor: null },
        })
        .responder("GET", BASE, { status: 200, corpo: { id: ORG, name: "Clínica Teste" } })
        .instalar();
}

function montar() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/products`]}>
            <Routes>
                <Route path="/orgs/:orgId/products" element={<Produtos />} />
                <Route path="/orgs/:orgId/structure" element={<p>tela de estrutura</p>} />
            </Routes>
        </MemoryRouter>,
    );
}

function linhaDe(nome: string): HTMLElement {
    return screen.getByText(nome).closest("tr") as HTMLElement;
}

describe("Produtos", () => {
    it("lista todas as situações sem mandar limit e conta os agentes que oferecem cada produto", async () => {
        const servidor = servidorPadrao();
        montar();

        expect(await screen.findByText("Clareamento dental")).toBeInTheDocument();
        expect(screen.getByText("Implante unitário")).toBeInTheDocument();
        expect(screen.getByText("também: clareamento")).toBeInTheDocument();

        const [listagem] = servidor.chamadasPara("GET", `${BASE}/products`);
        expect(listagem?.url).toContain("all=true");
        expect(listagem?.url).not.toContain("limit");

        // Clareamento: ag_1 (sem categoria) e ag_2 (cat_1). Implante: só ag_1.
        expect(within(linhaDe("Clareamento dental")).getByText("2 agentes")).toBeInTheDocument();
        expect(within(linhaDe("Implante unitário")).getByText("1 agente")).toBeInTheDocument();
    });

    it("filtra por situação, por categoria e pelo nome (inclusive outro nome)", async () => {
        const usuario = userEvent.setup();
        servidorPadrao();
        montar();
        await screen.findByText("Clareamento dental");

        await usuario.selectOptions(screen.getByRole("combobox", { name: "Situação" }), "inactive");
        expect(screen.queryByText("Clareamento dental")).not.toBeInTheDocument();
        expect(screen.getByText("Implante unitário")).toBeInTheDocument();

        await usuario.selectOptions(screen.getByRole("combobox", { name: "Situação" }), "todas");
        await usuario.selectOptions(screen.getByRole("combobox", { name: "Categoria" }), "cat_1");
        expect(screen.getByText("Clareamento dental")).toBeInTheDocument();
        expect(screen.queryByText("Implante unitário")).not.toBeInTheDocument();

        await usuario.selectOptions(screen.getByRole("combobox", { name: "Categoria" }), "todas");
        await usuario.type(screen.getByRole("textbox", { name: "Buscar por nome" }), "CLAREA");
        expect(screen.getByText("Clareamento dental")).toBeInTheDocument();
        expect(screen.queryByText("Implante unitário")).not.toBeInTheDocument();
    });

    it("sem produto que case com o filtro, mostra o estado vazio com limpar filtros", async () => {
        const usuario = userEvent.setup();
        servidorPadrao();
        montar();
        await screen.findByText("Clareamento dental");

        await usuario.type(screen.getByRole("textbox", { name: "Buscar por nome" }), "zzz");
        expect(await screen.findByText("Nenhum produto com esses filtros")).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Limpar filtros" }));
        expect(await screen.findByText("Clareamento dental")).toBeInTheDocument();
    });

    it("sem produto cadastrado e com categoria, oferece criar o primeiro", async () => {
        servidorPadrao([], CATEGORIAS);
        montar();
        expect(await screen.findByText("Nenhum produto cadastrado")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Criar primeiro produto" })).toBeInTheDocument();
    });

    it("sem categoria, o vazio manda para a Estrutura antes de criar produto", async () => {
        const usuario = userEvent.setup();
        servidorPadrao([], []);
        montar();
        expect(await screen.findByText("Nenhum produto cadastrado")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Novo produto" })).toBeDisabled();

        await usuario.click(screen.getByRole("button", { name: "Ir para Estrutura" }));
        expect(await screen.findByText("tela de estrutura")).toBeInTheDocument();
    });

    it("falha ao carregar os produtos mostra o erro com tentar de novo, sem derrubar a tela", async () => {
        new ServidorFalso()
            .responder("GET", `${BASE}/products`, erroApi(500, "internal", "falha interna ao listar"))
            .responder("GET", `${BASE}/categories`, { status: 200, corpo: { items: CATEGORIAS, next_cursor: null } })
            .responder("GET", `${BASE}/agents`, { status: 200, corpo: { items: [], next_cursor: null } })
            .responder("GET", BASE, { status: 200, corpo: { id: ORG, name: "Clínica Teste" } })
            .instalar();
        montar();

        expect(await screen.findByText("Não consegui carregar os produtos")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
        expect(screen.getByRole("heading", { name: "Produtos" })).toBeInTheDocument();
    });

    it("trocar o preço em centavos atualiza o preço que a lista mostra, sem reenviar o texto antigo", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorPadrao();
        servidor.responder("PATCH", `${BASE}/products/prod_1`, {
            status: 200,
            corpo: produto("prod_1", "Clareamento dental", "cat_1", {
                price_cents: 50000,
                price_text: "R$ 500,00",
                version: 2,
            }),
        });
        montar();
        await screen.findByText("Clareamento dental");
        expect(within(linhaDe("Clareamento dental")).getByText("R$ 450,00")).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Editar produto Clareamento dental" }));
        const preco = await screen.findByRole("textbox", { name: "Preço (R$)" });
        await usuario.clear(preco);
        await usuario.type(preco, "500");
        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(await within(linhaDe("Clareamento dental")).findByText("R$ 500,00")).toBeInTheDocument();
        expect(within(linhaDe("Clareamento dental")).queryByText("R$ 450,00")).not.toBeInTheDocument();
        const [patch] = servidor.chamadasPara("PATCH", `${BASE}/products/prod_1`);
        expect(patch?.corpo).toMatchObject({ price_cents: 50000, price_text: "" });
    });

    it("o catálogo é uma seção plana com tabela contínua, sem cartão", async () => {
        servidorPadrao();
        const { container } = montar();
        await screen.findByText("Clareamento dental");

        expect(screen.getByRole("heading", { level: 2, name: "Catálogo" })).toBeInTheDocument();
        expect(container.querySelector(".sofa-card")).toBeNull();
        expect(container.querySelector(".poph-produtos__tabela table")).not.toBeNull();
        expect(within(linhaDe("Clareamento dental")).getByText("Ativo")).toBeInTheDocument();
    });

    it("renders catalog in en-US and updates live when locale changes", async () => {
        setLocale("en-US");
        servidorPadrao();
        montar();

        await screen.findByText("Clareamento dental");
        expect(screen.getByRole("heading", { level: 1, name: "Products" })).toBeInTheDocument();
        expect(screen.getByRole("heading", { level: 2, name: "Catalog" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "New product" })).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "Offering agents" })).toBeInTheDocument();
        expect(within(linhaDe("Clareamento dental")).getByText("Active")).toBeInTheDocument();

        act(() => {
            setLocale("pt-BR");
        });
        expect(await screen.findByRole("heading", { level: 1, name: "Produtos" })).toBeInTheDocument();
        expect(within(linhaDe("Clareamento dental")).getByText("Ativo")).toBeInTheDocument();
    });
});

afterEach(() => {
    setLocale("pt-BR");
});

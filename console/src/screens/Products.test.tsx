
import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { setLocale } from "../i18n";
import { FakeServer, apiError } from "../testing/server";
import { Products } from "./Products";

const ORG = "org_1";
const BASE = `/admin/v1/orgs/${ORG}`;

function product(id: string, nome: string, category: string, sobrescrito: Record<string, unknown> = {}) {
    return {
        id,
        org_id: ORG,
        category_id: category,
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

function category(id: string, nome: string) {
    return { id, org_id: ORG, name: nome, description: "", status: "active", version: 1, created_at: 0, updated_at: 0 };
}

const PRODUCTS = [
    product("prod_1", "Clareamento dental", "cat_1", { aliases: ["clareamento"], unit: "por sessão" }),
    product("prod_2", "Implante unitário", "cat_2", {
        status: "inactive",
        price_cents: 320000,
        price_text: "R$ 3.200,00",
    }),
];

const CATEGORIES = [category("cat_1", "Ortodontia"), category("cat_2", "Implantes")];

const AGENTS = [
    { id: "ag_1", org_id: ORG, name: "Atendente geral", product_category_ids: [] },
    { id: "ag_2", org_id: ORG, name: "Ortodontia", product_category_ids: ["cat_1"] },
];

function serverDefault(products = PRODUCTS, categories = CATEGORIES) {
    return new FakeServer()
        .respond("GET", `${BASE}/agents/ag_1`, { status: 200, body: AGENTS[0] })
        .respond("GET", `${BASE}/agents/ag_2`, { status: 200, body: AGENTS[1] })
        .respond("GET", `${BASE}/agents`, {
            status: 200,
            body: { items: AGENTS.map(({ id, name }) => ({ id, name })), next_cursor: null },
        })
        .respond("GET", `${BASE}/products`, {
            status: 200,
            body: { items: products, next_cursor: null },
        })
        .respond("GET", `${BASE}/categories`, {
            status: 200,
            body: { items: categories, next_cursor: null },
        })
        .respond("GET", BASE, { status: 200, body: { id: ORG, name: "Clínica Teste" } })
        .install();
}

function build() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/products`]}>
            <Routes>
                <Route path="/orgs/:orgId/products" element={<Products />} />
                <Route path="/orgs/:orgId/structure" element={<p>tela de estrutura</p>} />
            </Routes>
        </MemoryRouter>,
    );
}

function rowOf(nome: string): HTMLElement {
    return screen.getByText(nome).closest("tr") as HTMLElement;
}

describe("Produtos", () => {
    it("lista todas as situações sem mandar limit e conta os agentes que oferecem cada produto", async () => {
        const servidor = serverDefault();
        build();

        expect(await screen.findByText("Clareamento dental")).toBeInTheDocument();
        expect(screen.getByText("Implante unitário")).toBeInTheDocument();
        expect(screen.getByText("também: clareamento")).toBeInTheDocument();

        const [listing] = servidor.callsFor("GET", `${BASE}/products`);
        expect(listing?.url).toContain("all=true");
        expect(listing?.url).not.toContain("limit");

        expect(within(rowOf("Clareamento dental")).getByText("2 agentes")).toBeInTheDocument();
        expect(within(rowOf("Implante unitário")).getByText("1 agente")).toBeInTheDocument();
    });

    it("filtra por situação, por categoria e pelo nome (inclusive outro nome)", async () => {
        const user = userEvent.setup();
        serverDefault();
        build();
        await screen.findByText("Clareamento dental");

        await user.selectOptions(screen.getByRole("combobox", { name: "Situação" }), "inactive");
        expect(screen.queryByText("Clareamento dental")).not.toBeInTheDocument();
        expect(screen.getByText("Implante unitário")).toBeInTheDocument();

        await user.selectOptions(screen.getByRole("combobox", { name: "Situação" }), "todas");
        await user.selectOptions(screen.getByRole("combobox", { name: "Categoria" }), "cat_1");
        expect(screen.getByText("Clareamento dental")).toBeInTheDocument();
        expect(screen.queryByText("Implante unitário")).not.toBeInTheDocument();

        await user.selectOptions(screen.getByRole("combobox", { name: "Categoria" }), "todas");
        await user.type(screen.getByRole("textbox", { name: "Buscar por nome" }), "CLAREA");
        expect(screen.getByText("Clareamento dental")).toBeInTheDocument();
        expect(screen.queryByText("Implante unitário")).not.toBeInTheDocument();
    });

    it("sem produto que case com o filtro, mostra o estado vazio com limpar filtros", async () => {
        const user = userEvent.setup();
        serverDefault();
        build();
        await screen.findByText("Clareamento dental");

        await user.type(screen.getByRole("textbox", { name: "Buscar por nome" }), "zzz");
        expect(await screen.findByText("Nenhum produto com esses filtros")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Limpar filtros" }));
        expect(await screen.findByText("Clareamento dental")).toBeInTheDocument();
    });

    it("sem produto cadastrado e com categoria, oferece criar o primeiro", async () => {
        serverDefault([], CATEGORIES);
        build();
        expect(await screen.findByText("Nenhum produto cadastrado")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Criar primeiro produto" })).toBeInTheDocument();
    });

    it("sem categoria, o vazio manda para a Estrutura antes de criar produto", async () => {
        const user = userEvent.setup();
        serverDefault([], []);
        build();
        expect(await screen.findByText("Nenhum produto cadastrado")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Novo produto" })).toBeDisabled();

        await user.click(screen.getByRole("button", { name: "Ir para Estrutura" }));
        expect(await screen.findByText("tela de estrutura")).toBeInTheDocument();
    });

    it("falha ao carregar os produtos mostra o erro com tentar de novo, sem derrubar a tela", async () => {
        new FakeServer()
            .respond("GET", `${BASE}/products`, apiError(500, "internal", "falha interna ao listar"))
            .respond("GET", `${BASE}/categories`, { status: 200, body: { items: CATEGORIES, next_cursor: null } })
            .respond("GET", `${BASE}/agents`, { status: 200, body: { items: [], next_cursor: null } })
            .respond("GET", BASE, { status: 200, body: { id: ORG, name: "Clínica Teste" } })
            .install();
        build();

        expect(await screen.findByText("Não consegui carregar os produtos")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
        expect(screen.getByRole("heading", { name: "Produtos" })).toBeInTheDocument();
    });

    it("trocar o preço em centavos atualiza o preço que a lista mostra, sem reenviar o texto antigo", async () => {
        const user = userEvent.setup();
        const servidor = serverDefault();
        servidor.respond("PATCH", `${BASE}/products/prod_1`, {
            status: 200,
            body: product("prod_1", "Clareamento dental", "cat_1", {
                price_cents: 50000,
                price_text: "R$ 500,00",
                version: 2,
            }),
        });
        build();
        await screen.findByText("Clareamento dental");
        expect(within(rowOf("Clareamento dental")).getByText("R$ 450,00")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Editar produto Clareamento dental" }));
        const preco = await screen.findByRole("textbox", { name: "Preço (R$)" });
        await user.clear(preco);
        await user.type(preco, "500");
        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(await within(rowOf("Clareamento dental")).findByText("R$ 500,00")).toBeInTheDocument();
        expect(within(rowOf("Clareamento dental")).queryByText("R$ 450,00")).not.toBeInTheDocument();
        const [patch] = servidor.callsFor("PATCH", `${BASE}/products/prod_1`);
        expect(patch?.body).toMatchObject({ price_cents: 50000, price_text: "" });
    });

    it("o catálogo é uma seção plana com tabela contínua, sem cartão", async () => {
        serverDefault();
        const { container } = build();
        await screen.findByText("Clareamento dental");

        expect(screen.getByRole("heading", { level: 2, name: "Catálogo" })).toBeInTheDocument();
        expect(container.querySelector(".sofa-card")).toBeNull();
        expect(container.querySelector(".poph-products__table table")).not.toBeNull();
        expect(within(rowOf("Clareamento dental")).getByText("Ativo")).toBeInTheDocument();
    });

    it("renders catalog in en-US and updates live when locale changes", async () => {
        setLocale("en-US");
        serverDefault();
        build();

        await screen.findByText("Clareamento dental");
        expect(screen.getByRole("heading", { level: 1, name: "Products" })).toBeInTheDocument();
        expect(screen.getByRole("heading", { level: 2, name: "Catalog" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "New product" })).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "Offering agents" })).toBeInTheDocument();
        expect(within(rowOf("Clareamento dental")).getByText("Active")).toBeInTheDocument();

        act(() => {
            setLocale("pt-BR");
        });
        expect(await screen.findByRole("heading", { level: 1, name: "Produtos" })).toBeInTheDocument();
        expect(within(rowOf("Clareamento dental")).getByText("Ativo")).toBeInTheDocument();
    });
});

afterEach(() => {
    setLocale("pt-BR");
});

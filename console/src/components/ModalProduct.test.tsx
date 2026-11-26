
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Product } from "../api/products";
import { setLocale } from "../i18n";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { ModalProduct, centsOf, formatBRL, listOf, brlOf } from "../components/ModalProduct";

const ORG = "org_1";
const CATEGORIES = [
    { id: "cat_1", org_id: ORG, name: "Ortodontia", description: "", status: "active", version: 1, created_at: 0, updated_at: 0 },
    { id: "cat_2", org_id: ORG, name: "Implantes", description: "", status: "active", version: 1, created_at: 0, updated_at: 0 },
];

function productBase(sobrescrito: Partial<Product> = {}): Product {
    return {
        id: "prod_1",
        org_id: ORG,
        category_id: "cat_1",
        name: "Clareamento dental",
        slug: "clareamento-dental",
        aliases: ["clareamento"],
        description: "",
        price_text: "R$ 450,00",
        price_cents: 45000,
        unit: "por sessão",
        includes: "",
        excludes: "",
        conditions: "",
        tags: ["estetica"],
        status: "active",
        version: 2,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...sobrescrito,
    };
}

function campo(nome: string): HTMLElement {
    return screen.getByRole("textbox", { name: nome });
}

function build(props: Partial<React.ComponentProps<typeof ModalProduct>> = {}) {
    const onSave = vi.fn();
    const onClose = vi.fn();
    render(
        <ModalProduct
            org={ORG}
            aberto
            produto={null}
            categories={CATEGORIES}
            onClose={onClose}
            onSave={onSave}
            {...props}
        />,
    );
    return { aoSalvar: onSave, aoFechar: onClose };
}

describe("conversão de preço e listas", () => {
    it("aceita 450, 450,00, 1.234,50 e R$ 450 e devolve centavos", () => {
        expect(centsOf("450")).toBe(45000);
        expect(centsOf("450,00")).toBe(45000);
        expect(centsOf("1.234,50")).toBe(123450);
        expect(centsOf("R$ 450")).toBe(45000);
        expect(centsOf("")).toBe(0);
        expect(centsOf("abc")).toBeNull();
        expect(centsOf("1,234")).toBeNull();
    });

    it("mostra centavos em reais com vírgula e separa listas por vírgula", () => {
        expect(brlOf(45000)).toBe("450,00");
        expect(listOf(" a, ,b ,, c ")).toEqual(["a", "b", "c"]);
    });

    it("formata o texto calculado no padrão do servidor (com separador de milhar)", () => {
        expect(formatBRL(890000)).toBe("R$ 8.900,00");
        expect(formatBRL(245000)).toBe("R$ 2.450,00");
        expect(formatBRL(18000)).toBe("R$ 180,00");
        expect(formatBRL(0)).toBe("R$ 0,00");
        expect(formatBRL(99)).toBe("R$ 0,99");
        expect(formatBRL(100000000)).toBe("R$ 1.000.000,00");
    });
});

describe("ModalProduto", () => {
    it("cria produto com o corpo da API (centavos, listas, sem status e sem slug vazio)", async () => {
        const user = userEvent.setup();
        const created = productBase({ id: "prod_9", name: "Implante unitário", price_cents: 320000 });
        const servidor = new FakeServer()
            .responder("POST", `/admin/v1/orgs/${ORG}/products`, { status: 201, corpo: created })
            .instalar();
        const { aoSalvar: onSave, aoFechar: onClose } = build();
        await waitForModalFocus();

        await user.selectOptions(screen.getByRole("combobox", { name: "Categoria" }), "cat_2");
        await user.type(campo("Nome"), "Implante unitário");
        await user.type(campo("Outros nomes"), "implante, pino, implante simples");
        await user.type(campo("Preço (R$)"), "3.200,00");
        await user.type(campo("Etiquetas de busca"), "cirurgia");
        await user.click(screen.getByRole("button", { name: "Criar produto" }));

        await waitFor(() => expect(onSave).toHaveBeenCalledWith(created));
        expect(onClose).toHaveBeenCalled();
        const [request] = servidor.chamadasPara("POST", "/products");
        expect(request?.corpo).toEqual({
            category_id: "cat_2",
            name: "Implante unitário",
            aliases: ["implante", "pino", "implante simples"],
            description: "",
            price_text: "",
            price_cents: 320000,
            unit: "",
            includes: "",
            excludes: "",
            conditions: "",
            tags: ["cirurgia"],
        });
        expect(request?.corpo).not.toHaveProperty("status");
        expect(request?.corpo).not.toHaveProperty("slug");
    });

    it("na criação não existe o campo Situação, que a API recusa", async () => {
        new FakeServer().instalar();
        build();
        expect(await screen.findByRole("dialog", { name: "Novo produto" })).toBeInTheDocument();
        expect(screen.queryByRole("combobox", { name: "Situação" })).not.toBeInTheDocument();
    });

    it("sem categoria, nome vazio e preço inválido não chamam a API e marcam os campos", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer().instalar();
        build();
        await waitForModalFocus();

        await user.type(campo("Preço (R$)"), "abc");
        await user.click(screen.getByRole("button", { name: "Criar produto" }));

        expect(await screen.findByText("escolha a categoria do produto")).toBeInTheDocument();
        expect(screen.getByText("informe o nome do produto")).toBeInTheDocument();
        expect(screen.getByText("use o formato 450 ou 450,00 (valor em reais)")).toBeInTheDocument();
        expect(servidor.chamadas).toHaveLength(0);
    });

    it("erro de campo vindo da API (slug repetido) aparece no campo Identificador", async () => {
        const user = userEvent.setup();
        new FakeServer()
            .responder(
                "POST",
                `/admin/v1/orgs/${ORG}/products`,
                apiError(400, "validation_error", "campo `slug` inválido", {
                    field: "slug",
                    reason: "já existe um produto com esse identificador nesta org",
                }),
            )
            .instalar();
        build();
        await waitForModalFocus();

        await user.selectOptions(screen.getByRole("combobox", { name: "Categoria" }), "cat_1");
        await user.type(campo("Nome"), "Clareamento");
        await user.type(campo("Identificador (slug)"), "clareamento-dental");
        await user.click(screen.getByRole("button", { name: "Criar produto" }));

        expect(
            await screen.findByText("já existe um produto com esse identificador nesta org"),
        ).toBeInTheDocument();
    });

    it("editar envia If-Match e a situação; no 412 relê, mantém o que foi digitado e salva com a versão nova", async () => {
        const user = userEvent.setup();
        const onSave = vi.fn();
        let attempt = 0;
        const servidor = new FakeServer()
            .responder("PATCH", `/admin/v1/orgs/${ORG}/products/prod_1`, () => {
                attempt += 1;
                if (attempt === 1) {
                    return apiError(412, "version_conflict", "produto mudou desde a leitura", {
                        current_version: 3,
                        expected_version: 2,
                    });
                }
                return { status: 200, corpo: productBase({ version: 4, name: "Clareamento caseiro" }) };
            })
            .responder("GET", `/admin/v1/orgs/${ORG}/products/prod_1`, {
                status: 200,
                corpo: productBase({ version: 3, price_cents: 50000, price_text: "R$ 500,00" }),
            })
            .instalar();
        build({ produto: productBase(), onSave });
        await waitForModalFocus();

        expect(screen.queryByRole("combobox", { name: "Situação" })).toBeInTheDocument();
        await user.clear(campo("Nome"));
        await user.type(campo("Nome"), "Clareamento caseiro");
        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(await screen.findByText(/Outra pessoa salvou este produto/)).toBeInTheDocument();
        expect(screen.getByText(/agora na versão 3/)).toBeInTheDocument();
        expect(campo("Nome")).toHaveValue("Clareamento caseiro");

        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(onSave).toHaveBeenCalled());

        const patches = servidor.chamadasPara("PATCH", "/products/prod_1");
        expect(patches).toHaveLength(2);
        expect(patches[0]?.cabecalhos["if-match"]).toBe("2");
        expect(patches[1]?.cabecalhos["if-match"]).toBe("3");
        expect(patches[1]?.corpo).toMatchObject({
            name: "Clareamento caseiro",
            price_cents: 45000,
            status: "active",
        });
        expect(patches[1]?.corpo).not.toHaveProperty("slug");
        expect(patches[1]?.corpo).toMatchObject({ price_text: "" });
    });
});

describe("preço como o cliente vê", () => {
    const URL_PRODUCT = `/admin/v1/orgs/${ORG}/products/prod_1`;

    it("editar sem mexer no preço não envia o texto e mostra o que o cliente vê", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .responder("PATCH", URL_PRODUCT, { status: 200, corpo: productBase({ version: 3 }) })
            .instalar();
        build({ produto: productBase() });
        await waitForModalFocus();

        expect(campo("Preço como o cliente vê")).toHaveValue("");
        expect(screen.getByText(/Como o cliente vê: R\$ 450,00\./)).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(servidor.chamadasPara("PATCH", "/products/prod_1")).toHaveLength(1));

        const [patch] = servidor.chamadasPara("PATCH", "/products/prod_1");
        expect(patch?.corpo).not.toHaveProperty("price_text");
        expect(patch?.corpo).toMatchObject({ price_cents: 45000 });
    });

    it("mudar só o valor em centavos envia o texto vazio, a prévia acompanha e a lista recebe o preço novo", async () => {
        const user = userEvent.setup();
        const onSave = vi.fn();
        const servidor = new FakeServer()
            .responder("PATCH", URL_PRODUCT, {
                status: 200,
                corpo: productBase({ version: 3, price_cents: 50000, price_text: "R$ 500,00" }),
            })
            .instalar();
        build({ produto: productBase(), onSave });
        await waitForModalFocus();

        await user.clear(campo("Preço (R$)"));
        await user.type(campo("Preço (R$)"), "500");
        expect(screen.getByText(/Como o cliente vê: R\$ 500,00\./)).toBeInTheDocument();
        expect(screen.getByText(/O preço mudou/)).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(onSave).toHaveBeenCalled());

        const [patch] = servidor.chamadasPara("PATCH", "/products/prod_1");
        expect(patch?.corpo).toMatchObject({ price_cents: 50000, price_text: "" });
        expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ price_text: "R$ 500,00" }));
    });

    it("texto digitado pelo operador vai no corpo como foi digitado", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .responder("PATCH", URL_PRODUCT, { status: 200, corpo: productBase({ version: 3 }) })
            .instalar();
        build({ produto: productBase({ price_cents: 32000, price_text: "R$ 320,00" }) });
        await waitForModalFocus();

        await user.type(campo("Preço como o cliente vê"), "R$ 320,00 por visita");
        expect(screen.getByText(/Como o cliente vê: R\$ 320,00 por visita\./)).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(servidor.chamadasPara("PATCH", "/products/prod_1")).toHaveLength(1));

        const [patch] = servidor.chamadasPara("PATCH", "/products/prod_1");
        expect(patch?.corpo).toMatchObject({ price_cents: 32000, price_text: "R$ 320,00 por visita" });
    });

    it("apagar o texto digitado envia vazio e a prévia volta ao valor calculado", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .responder("PATCH", URL_PRODUCT, { status: 200, corpo: productBase({ version: 3 }) })
            .instalar();
        build({ produto: productBase({ price_cents: 32000, price_text: "R$ 320,00" }) });
        await waitForModalFocus();

        await user.type(campo("Preço como o cliente vê"), "x");
        await user.clear(campo("Preço como o cliente vê"));
        expect(screen.getByText(/Como o cliente vê: R\$ 320,00\./)).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(servidor.chamadasPara("PATCH", "/products/prod_1")).toHaveLength(1));

        const [patch] = servidor.chamadasPara("PATCH", "/products/prod_1");
        expect(patch?.corpo).toMatchObject({ price_text: "" });
    });

    it("com texto próprio gravado, o botão volta o texto a ser calculado pelo valor", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .responder("PATCH", URL_PRODUCT, { status: 200, corpo: productBase({ version: 3 }) })
            .instalar();
        build({ produto: productBase({ price_cents: 32000, price_text: "R$ 320,00 por visita" }) });
        await waitForModalFocus();

        expect(screen.getByText(/Como o cliente vê: R\$ 320,00 por visita\./)).toBeInTheDocument();
        await user.click(screen.getByRole("button", { name: "Voltar a calcular o texto pelo valor" }));
        expect(screen.getByText(/Como o cliente vê: R\$ 320,00\./)).toBeInTheDocument();
        expect(
            screen.queryByRole("button", { name: "Voltar a calcular o texto pelo valor" }),
        ).not.toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(servidor.chamadasPara("PATCH", "/products/prod_1")).toHaveLength(1));
        expect(servidor.chamadasPara("PATCH", "/products/prod_1")[0]?.corpo).toMatchObject({
            price_text: "",
        });
    });

    it("renders fields, statuses, and validation in en-US", async () => {
        const user = userEvent.setup();
        setLocale("en-US");
        build();
        await waitForModalFocus();

        expect(screen.getByRole("heading", { name: "New product" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: "Choose a category" })).toBeInTheDocument();
        await user.click(screen.getByRole("button", { name: "Create product" }));
        expect(await screen.findByText("choose the product category")).toBeInTheDocument();
        expect(screen.getByText("enter the product name")).toBeInTheDocument();

        act(() => {
            setLocale("pt-BR");
        });
        expect(await screen.findByRole("heading", { name: "Novo produto" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Criar produto" })).toBeInTheDocument();
    });
});

afterEach(() => {
    setLocale("pt-BR");
});

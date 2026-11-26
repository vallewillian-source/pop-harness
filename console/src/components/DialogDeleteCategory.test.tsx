
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setLocale } from "../i18n";
import { DialogDeleteCategory } from "../components/DialogDeleteCategory";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";

const ORG = "org_1";

const category = {
    id: "cat_1",
    org_id: ORG,
    name: "Clareamento",
    description: "",
    status: "active",
    version: 1,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

function build(onDelete = vi.fn(), onClose = vi.fn()) {
    return render(
        <DialogDeleteCategory
            opened
            onClose={onClose}
            org={ORG}
            category={category}
            onDelete={onDelete}
        />,
    );
}

function listProducts(products: Array<{ id: string; name: string; category_id: string }>) {
    return (call: { url: string }) => {
        if (call.url.includes("limit=")) {
            return apiError(400, "invalid_request", "parâmetros de paginação inválidos");
        }
        return { status: 200, body: { items: products, next_cursor: null } };
    };
}

function serverLinks(usedBy: Array<"ag_1" | "ag_2">, withProducts: boolean) {
    const all = [
        { id: "p1", name: "Clareamento a laser", category_id: "cat_1" },
        { id: "p2", name: "Consulta", category_id: "cat_2" },
        { id: "p3", name: "Clareamento caseiro", category_id: "cat_1" },
    ];
    const products = withProducts ? all : all.filter((product) => product.category_id !== "cat_1");
    const agents = [
        { id: "ag_1", name: "Atendente" },
        { id: "ag_2", name: "Triagem" },
    ];
    return new FakeServer()
        .respond("GET", "/products?all=true", listProducts(products))
        .respond("GET", "/agents?limit", {
            status: 200,
            body: { items: agents, next_cursor: null },
        })
        .respond("GET", "/agents/ag_1", {
            status: 200,
            body: { ...agents[0], product_category_ids: usedBy.includes("ag_1") ? ["cat_1"] : [] },
        })
        .respond("GET", "/agents/ag_2", {
            status: 200,
            body: { ...agents[1], product_category_ids: usedBy.includes("ag_2") ? ["cat_1"] : [] },
        });
}

describe("DialogoExcluirCategoria", () => {
    it("categoria sem vínculo não pede force e sai direto", async () => {
        const user = userEvent.setup();
        const onDelete = vi.fn();
        const onClose = vi.fn();
        const servidor = serverLinks([], false)
            .respond("DELETE", `/categories/cat_1`, { status: 204 })
            .install();

        build(onDelete, onClose);
        const dialog = await screen.findByRole("dialog", { name: "Excluir categoria?" });
        await waitForModalFocus();

        expect(await within(dialog).findByText(/Nenhum produto nem agente usa esta categoria/)).toBeInTheDocument();
        expect(within(dialog).queryByRole("checkbox")).toBeNull();

        await user.type(within(dialog).getByRole("textbox"), "Clareamento");
        await user.click(within(dialog).getByRole("button", { name: "Excluir categoria" }));

        await waitFor(() => expect(onDelete).toHaveBeenCalledTimes(1));
        expect(onClose).toHaveBeenCalled();
        const [request] = servidor.callsFor("DELETE", "/categories/cat_1");
        expect(request?.url).not.toContain("force");
    });

    it("com produtos e agentes, mostra a contagem, recusa sem force e libera a confirmação extra", async () => {
        const user = userEvent.setup();
        const onDelete = vi.fn();
        const servidor = serverLinks(["ag_1"], true)
            .respond("DELETE", `/categories/cat_1`, (call) => {
                if (call.url.includes("force=true")) return { status: 204 };
                return apiError(
                    409,
                    "conflict",
                    "a categoria tem 2 produto(s) e 1 agente(s) vinculados; excluir apaga os produtos dela e tira a categoria dos agentes. Para confirmar, repita com force=true",
                    { produtos: 2, agentes: 1 },
                );
            })
            .install();

        build(onDelete);
        const dialog = await screen.findByRole("dialog", { name: "Excluir categoria?" });
        await waitForModalFocus();

        expect(await within(dialog).findByText(/Clareamento a laser, Clareamento caseiro/)).toBeInTheDocument();
        expect(within(dialog).getByText("Atendente")).toBeInTheDocument();
        expect(within(dialog).queryByText("Consulta")).toBeNull();
        expect(within(dialog).queryByText("Triagem")).toBeNull();

        const confirm = within(dialog).getByRole("button", { name: "Excluir categoria" });
        await user.type(within(dialog).getByRole("textbox"), "Clareamento");
        await user.click(confirm);

        expect(
            await within(dialog).findByText(/a categoria tem 2 produto\(s\) e 1 agente\(s\) vinculados/),
        ).toBeInTheDocument();
        expect(within(dialog).getByText(/código: conflict/)).toBeInTheDocument();
        expect(onDelete).not.toHaveBeenCalled();
        expect(servidor.callsFor("DELETE", "/categories/cat_1")[0]?.url).not.toContain("force");
        expect(dialog).toBeInTheDocument();

        const extra = within(dialog).getByRole("checkbox", {
            name: /Apagar também os 2 produtos desta categoria/,
        });
        await user.click(extra);
        await user.click(confirm);

        await waitFor(() => expect(onDelete).toHaveBeenCalledTimes(1));
        const requests = servidor.callsFor("DELETE", "/categories/cat_1");
        expect(requests.at(-1)?.url).toContain("force=true");
        expect(requests).toHaveLength(2);
    });
});

describe("DialogoExcluirCategoria · contagem dos produtos", () => {
    it("lê todas as páginas de produtos só com cursor, sem mandar limit, e conta as duas", async () => {
        const servidor = new FakeServer()
            .respond("GET", "/products?all=true", (call) => {
                if (call.url.includes("limit=")) {
                    return apiError(400, "invalid_request", "parâmetros de paginação inválidos");
                }
                if (call.url.includes("cursor=cur_1")) {
                    return {
                        status: 200,
                        body: {
                            items: [{ id: "p3", name: "Clareamento caseiro", category_id: "cat_1" }],
                            next_cursor: null,
                        },
                    };
                }
                return {
                    status: 200,
                    body: {
                        items: [{ id: "p1", name: "Clareamento a laser", category_id: "cat_1" }],
                        next_cursor: "cur_1",
                    },
                };
            })
            .respond("GET", "/agents?limit", {
                status: 200,
                body: { items: [], next_cursor: null },
            })
            .respond("DELETE", "/categories/cat_1", { status: 204 })
            .install();

        render(
            <DialogDeleteCategory
                opened
                onClose={vi.fn()}
                org={ORG}
                category={category}
                onDelete={vi.fn()}
            />,
        );
        const dialog = await screen.findByRole("dialog", { name: "Excluir categoria?" });

        expect(
            await within(dialog).findByText(/Esta categoria tem 2 produtos/),
        ).toBeInTheDocument();
        expect(within(dialog).getByText(/Clareamento a laser, Clareamento caseiro/)).toBeInTheDocument();
        expect(within(dialog).queryByText(/Não consegui contar/)).toBeNull();

        const listings = servidor.callsFor("GET", "/products?all=true");
        expect(listings).toHaveLength(2);
        expect(listings.every((call) => !call.url.includes("limit="))).toBe(true);
        expect(listings[1]?.url).toContain("cursor=cur_1");
    });

    it("renders consequences and force label in en-US", async () => {
        setLocale("en-US");
        new FakeServer()
            .respond("GET", "/products?all=true", {
                status: 200,
                body: {
                    items: [
                        { id: "p1", name: "Laser whitening", category_id: "cat_1" },
                        { id: "p2", name: "Home whitening", category_id: "cat_1" },
                    ],
                    next_cursor: null,
                },
            })
            .respond("GET", "/agents?limit", {
                status: 200,
                body: { items: [], next_cursor: null },
            })
            .install();

        render(
            <DialogDeleteCategory
                opened
                onClose={vi.fn()}
                org={ORG}
                category={category}
                onDelete={vi.fn()}
            />,
        );
        const dialog = await screen.findByRole("dialog", { name: "Delete category?" });

        expect(
            await within(dialog).findByText(/This category has 2 products/),
        ).toBeInTheDocument();
        expect(
            within(dialog).getByRole("checkbox", {
                name: /Also delete the 2 products in this category/,
            }),
        ).toBeInTheDocument();
        expect(within(dialog).getByRole("button", { name: "Delete category" })).toBeInTheDocument();
    });
});

afterEach(() => {
    setLocale("pt-BR");
});

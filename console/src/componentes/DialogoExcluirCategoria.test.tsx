/**
 * Testes da exclusão de categoria (HR-39b): a consequência contada antes, a primeira tentativa
 * sem `force`, a recusa 409 mostrada como veio, a confirmação extra que manda `force=true`, e a
 * categoria sem vínculo que sai direto.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setLocale } from "../i18n";
import { DialogoExcluirCategoria } from "./DialogoExcluirCategoria";
import { esperarFocoDoModal } from "../testes/modal";
import { ServidorFalso, erroApi } from "../testes/servidor";

const ORG = "org_1";

const categoria = {
    id: "cat_1",
    org_id: ORG,
    name: "Clareamento",
    description: "",
    status: "active",
    version: 1,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

function montar(aoExcluir = vi.fn(), aoFechar = vi.fn()) {
    return render(
        <DialogoExcluirCategoria
            aberto
            aoFechar={aoFechar}
            org={ORG}
            categoria={categoria}
            aoExcluir={aoExcluir}
        />,
    );
}

/**
 * Servidor falso com os produtos (de qualquer categoria, para a filtragem ser exercitada) e dois
 * agentes. `usadoPor` diz quais agentes usam a categoria `cat_1`.
 */
/**
 * Listagem de produtos como o servidor real a responde: `limit` na URL é recusado com 400
 * (`invalid_request`), e a paginação é só por `cursor`. Sem isso, o teste aceitaria a URL errada
 * que a tela mandava antes da correção.
 */
function listaDeProdutos(produtos: Array<{ id: string; name: string; category_id: string }>) {
    return (chamada: { url: string }) => {
        if (chamada.url.includes("limit=")) {
            return erroApi(400, "invalid_request", "parâmetros de paginação inválidos");
        }
        return { status: 200, corpo: { items: produtos, next_cursor: null } };
    };
}

function servidorComVinculos(usadoPor: Array<"ag_1" | "ag_2">, comProdutos: boolean) {
    const todos = [
        { id: "p1", name: "Clareamento a laser", category_id: "cat_1" },
        { id: "p2", name: "Consulta", category_id: "cat_2" },
        { id: "p3", name: "Clareamento caseiro", category_id: "cat_1" },
    ];
    const produtos = comProdutos ? todos : todos.filter((produto) => produto.category_id !== "cat_1");
    const agentes = [
        { id: "ag_1", name: "Atendente" },
        { id: "ag_2", name: "Triagem" },
    ];
    return new ServidorFalso()
        .responder("GET", "/products?all=true", listaDeProdutos(produtos))
        .responder("GET", "/agents?limit", {
            status: 200,
            corpo: { items: agentes, next_cursor: null },
        })
        .responder("GET", "/agents/ag_1", {
            status: 200,
            corpo: { ...agentes[0], product_category_ids: usadoPor.includes("ag_1") ? ["cat_1"] : [] },
        })
        .responder("GET", "/agents/ag_2", {
            status: 200,
            corpo: { ...agentes[1], product_category_ids: usadoPor.includes("ag_2") ? ["cat_1"] : [] },
        });
}

describe("DialogoExcluirCategoria", () => {
    it("categoria sem vínculo não pede force e sai direto", async () => {
        const usuario = userEvent.setup();
        const aoExcluir = vi.fn();
        const aoFechar = vi.fn();
        const servidor = servidorComVinculos([], false)
            .responder("DELETE", `/categories/cat_1`, { status: 204 })
            .instalar();

        montar(aoExcluir, aoFechar);
        const dialogo = await screen.findByRole("dialog", { name: "Excluir categoria?" });
        await esperarFocoDoModal();

        expect(await within(dialogo).findByText(/Nenhum produto nem agente usa esta categoria/)).toBeInTheDocument();
        expect(within(dialogo).queryByRole("checkbox")).toBeNull();

        await usuario.type(within(dialogo).getByRole("textbox"), "Clareamento");
        await usuario.click(within(dialogo).getByRole("button", { name: "Excluir categoria" }));

        await waitFor(() => expect(aoExcluir).toHaveBeenCalledTimes(1));
        expect(aoFechar).toHaveBeenCalled();
        const [pedido] = servidor.chamadasPara("DELETE", "/categories/cat_1");
        expect(pedido?.url).not.toContain("force");
    });

    it("com produtos e agentes, mostra a contagem, recusa sem force e libera a confirmação extra", async () => {
        const usuario = userEvent.setup();
        const aoExcluir = vi.fn();
        const servidor = servidorComVinculos(["ag_1"], true)
            .responder("DELETE", `/categories/cat_1`, (chamada) => {
                if (chamada.url.includes("force=true")) return { status: 204 };
                return erroApi(
                    409,
                    "conflict",
                    "a categoria tem 2 produto(s) e 1 agente(s) vinculados; excluir apaga os produtos dela e tira a categoria dos agentes. Para confirmar, repita com force=true",
                    { produtos: 2, agentes: 1 },
                );
            })
            .instalar();

        montar(aoExcluir);
        const dialogo = await screen.findByRole("dialog", { name: "Excluir categoria?" });
        await esperarFocoDoModal();

        // Antes de confirmar, a consequência está escrita: produtos pelo nome e o agente que fica sem ela.
        expect(await within(dialogo).findByText(/Clareamento a laser, Clareamento caseiro/)).toBeInTheDocument();
        expect(within(dialogo).getByText("Atendente")).toBeInTheDocument();
        expect(within(dialogo).queryByText("Consulta")).toBeNull();
        expect(within(dialogo).queryByText("Triagem")).toBeNull();

        const confirmar = within(dialogo).getByRole("button", { name: "Excluir categoria" });
        await usuario.type(within(dialogo).getByRole("textbox"), "Clareamento");
        await usuario.click(confirmar);

        // Primeira tentativa sem force: a API recusa e o motivo real aparece.
        expect(
            await within(dialogo).findByText(/a categoria tem 2 produto\(s\) e 1 agente\(s\) vinculados/),
        ).toBeInTheDocument();
        expect(within(dialogo).getByText(/código: conflict/)).toBeInTheDocument();
        expect(aoExcluir).not.toHaveBeenCalled();
        expect(servidor.chamadasPara("DELETE", "/categories/cat_1")[0]?.url).not.toContain("force");
        expect(dialogo).toBeInTheDocument();

        // A confirmação extra oferece apagar os produtos; marcada, a exclusão vai com force=true.
        const extra = within(dialogo).getByRole("checkbox", {
            name: /Apagar também os 2 produtos desta categoria/,
        });
        await usuario.click(extra);
        await usuario.click(confirmar);

        await waitFor(() => expect(aoExcluir).toHaveBeenCalledTimes(1));
        const pedidos = servidor.chamadasPara("DELETE", "/categories/cat_1");
        expect(pedidos.at(-1)?.url).toContain("force=true");
        expect(pedidos).toHaveLength(2);
    });
});

describe("DialogoExcluirCategoria · contagem dos produtos", () => {
    it("lê todas as páginas de produtos só com cursor, sem mandar limit, e conta as duas", async () => {
        const servidor = new ServidorFalso()
            .responder("GET", "/products?all=true", (chamada) => {
                if (chamada.url.includes("limit=")) {
                    return erroApi(400, "invalid_request", "parâmetros de paginação inválidos");
                }
                if (chamada.url.includes("cursor=cur_1")) {
                    return {
                        status: 200,
                        corpo: {
                            items: [{ id: "p3", name: "Clareamento caseiro", category_id: "cat_1" }],
                            next_cursor: null,
                        },
                    };
                }
                return {
                    status: 200,
                    corpo: {
                        items: [{ id: "p1", name: "Clareamento a laser", category_id: "cat_1" }],
                        next_cursor: "cur_1",
                    },
                };
            })
            .responder("GET", "/agents?limit", {
                status: 200,
                corpo: { items: [], next_cursor: null },
            })
            .responder("DELETE", "/categories/cat_1", { status: 204 })
            .instalar();

        render(
            <DialogoExcluirCategoria
                aberto
                aoFechar={vi.fn()}
                org={ORG}
                categoria={categoria}
                aoExcluir={vi.fn()}
            />,
        );
        const dialogo = await screen.findByRole("dialog", { name: "Excluir categoria?" });

        expect(
            await within(dialogo).findByText(/Esta categoria tem 2 produtos/),
        ).toBeInTheDocument();
        expect(within(dialogo).getByText(/Clareamento a laser, Clareamento caseiro/)).toBeInTheDocument();
        expect(within(dialogo).queryByText(/Não consegui contar/)).toBeNull();

        const listagens = servidor.chamadasPara("GET", "/products?all=true");
        expect(listagens).toHaveLength(2);
        expect(listagens.every((chamada) => !chamada.url.includes("limit="))).toBe(true);
        expect(listagens[1]?.url).toContain("cursor=cur_1");
    });

    it("renders consequences and force label in en-US", async () => {
        setLocale("en-US");
        new ServidorFalso()
            .responder("GET", "/products?all=true", {
                status: 200,
                corpo: {
                    items: [
                        { id: "p1", name: "Laser whitening", category_id: "cat_1" },
                        { id: "p2", name: "Home whitening", category_id: "cat_1" },
                    ],
                    next_cursor: null,
                },
            })
            .responder("GET", "/agents?limit", {
                status: 200,
                corpo: { items: [], next_cursor: null },
            })
            .instalar();

        render(
            <DialogoExcluirCategoria
                aberto
                aoFechar={vi.fn()}
                org={ORG}
                categoria={categoria}
                aoExcluir={vi.fn()}
            />,
        );
        const dialogo = await screen.findByRole("dialog", { name: "Delete category?" });

        expect(
            await within(dialogo).findByText(/This category has 2 products/),
        ).toBeInTheDocument();
        expect(
            within(dialogo).getByRole("checkbox", {
                name: /Also delete the 2 products in this category/,
            }),
        ).toBeInTheDocument();
        expect(within(dialogo).getByRole("button", { name: "Delete category" })).toBeInTheDocument();
    });
});

afterEach(() => {
    setLocale("pt-BR");
});

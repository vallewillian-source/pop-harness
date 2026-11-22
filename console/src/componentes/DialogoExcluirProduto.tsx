/**
 * Exclusão de produto do catálogo (HR-42), com a consequência escrita antes do botão.
 *
 * Dois passos (HR-42g, mesmo desenho da categoria):
 * - a primeira tentativa vai sem `force`. Se o produto estiver em itens de carrinho, a API recusa
 *   com `409`, nada é apagado, e a tela mostra a contagem real (itens e carrinhos) em linguagem
 *   comum;
 * - a recusa libera a confirmação extra ("Excluir mesmo assim…"), e a tentativa seguinte vai com
 *   `?force=true`. Sem carrinho, a exclusão sai direto, como antes.
 *
 * O que sai junto, pelo esquema do banco: o produto some do catálogo e da busca dos agentes; itens
 * de carrinho que citam o produto perdem a referência a ele (o preço já gravado continua).
 *
 * A contagem de agentes vem da tela: agente sem categoria vinculada enxerga o catálogo inteiro, então
 * entra na conta. A confirmação é digitar o nome exato (`DialogoExclusao`).
 */

import { useEffect, useRef, useState } from "react";
import { ErroApi } from "../api/cliente";
import {
    contagemDaRecusa,
    excluirProduto,
    type ContagemCarrinhos,
    type Produto,
} from "../api/produtos";
import { DialogoExclusao } from "./DialogoExclusao";

export type DialogoExcluirProdutoProps = {
    aberto: boolean;
    aoFechar: () => void;
    org: string;
    produto: Produto | null;
    /** Agentes que oferecem o produto hoje; `null` = a leitura dos agentes falhou. */
    agentesQueVeem: number | null;
    /** Chamado depois da exclusão, com o aviso do índice quando ele não atualizou. */
    aoExcluir: (avisoDoIndice: string | null) => void;
};

function plural(quantidade: number, singular: string, plurala: string): string {
    return `${quantidade} ${quantidade === 1 ? singular : plurala}`;
}

export function DialogoExcluirProduto({
    aberto,
    aoFechar,
    org,
    produto,
    agentesQueVeem,
    aoExcluir,
}: DialogoExcluirProdutoProps) {
    // Aviso do índice da exclusão em curso: vai de `aoConfirmar` até `aoExcluir`.
    const avisoRef = useRef<string | null>(null);
    // Contagem que a recusa 409 trouxe nesta abertura; some quando o diálogo fecha.
    const [contagem, setContagem] = useState<ContagemCarrinhos | null>(null);

    useEffect(() => {
        if (aberto) {
            setContagem(null);
        }
    }, [aberto]);

    if (!produto) return null;

    function consequencia() {
        const texto =
            agentesQueVeem === null ? (
                "Não consegui contar os agentes que oferecem este produto. Eles deixam de oferecê-lo depois da exclusão."
            ) : agentesQueVeem === 0 ? (
                "Nenhum agente oferece este produto hoje."
            ) : (
                <>
                    {agentesQueVeem === 1
                        ? "1 agente oferece este produto"
                        : `${agentesQueVeem} agentes oferecem este produto`}{" "}
                    (direto ou porque enxerga a categoria, inclusive agente sem categoria vinculada) e
                    deixam de oferecê-lo.
                </>
            );
        return (
            <div className="poph-pilha" style={{ gap: "var(--pop-spacing-sm, 6px)" }}>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {texto}
                </p>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    Itens de carrinho de conversas que citam este produto ficam sem a referência a
                    ele; o preço já gravado neles continua.
                </p>
                {contagem && (
                    <p style={{ margin: 0, color: "var(--pop-text-primary)", lineHeight: 1.6 }}>
                        Este produto está em{" "}
                        {plural(contagem.itens_de_carrinho, "item", "itens")} de{" "}
                        {plural(contagem.carrinhos, "carrinho", "carrinhos")}. Excluir deixa esses itens
                        como "produto removido do catálogo", com o preço que já estava gravado.
                    </p>
                )}
            </div>
        );
    }

    const rotuloForcar = contagem
        ? `Excluir mesmo assim: ${plural(contagem.itens_de_carrinho, "item de carrinho", "itens de carrinho")} ficam como "produto removido do catálogo"`
        : "Excluir mesmo assim, mesmo com itens de carrinho que citam este produto";

    return (
        <DialogoExclusao
            aberto={aberto}
            aoFechar={aoFechar}
            titulo="Excluir produto?"
            descricao={`O produto ${produto.name} sai do catálogo. Não há como desfazer.`}
            recurso={produto.name}
            rotuloRecurso="nome do produto"
            rotuloConfirmar="Excluir produto"
            consequencia={consequencia()}
            permiteForcar
            forcarSoAposRecusa
            rotuloForcar={rotuloForcar}
            descricaoForcar="Sem esta confirmação, a API não apaga um produto que está em carrinhos. Nada é apagado antes disso."
            aoConfirmar={async (forcar) => {
                try {
                    const resultado = await excluirProduto(org, produto.id, forcar);
                    avisoRef.current = resultado.index_warning ?? null;
                } catch (falha) {
                    // 409: a recusa traz a contagem; a tela mostra e a confirmação extra libera o envio.
                    if (falha instanceof ErroApi && falha.status === 409) {
                        setContagem(contagemDaRecusa(falha.detalhes));
                    }
                    throw falha;
                }
            }}
            aoExcluir={() => aoExcluir(avisoRef.current)}
        />
    );
}

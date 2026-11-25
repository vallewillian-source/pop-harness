/**
 * Exclusão de categoria (HR-39b), com a consequência escrita antes da confirmação.
 *
 * Ao abrir, conta os produtos (de qualquer situação) e os agentes que usam a categoria. Sem
 * vínculo, a exclusão sai direto. Com vínculo, a primeira tentativa vai sem `force`: a API recusa
 * com 409 e a tela mostra o motivo real; a confirmação extra ("apagar também os N produtos")
 * então vira a saída, e a tentativa seguinte vai com `force=true`.
 */

import { useEffect, useState } from "react";
import { SofaSkeleton } from "@sofacoding/sofa-ui";
import {
    agentesQueUsamCategoria,
    excluirCategoria,
    produtosDaCategoria,
    type AgenteResumo,
    type Categoria,
    type ProdutoDaCategoria,
} from "../api/estrutura";
import { useI18n } from "../i18n";
import { DialogoExclusao } from "./DialogoExclusao";

export type DialogoExcluirCategoriaProps = {
    aberto: boolean;
    aoFechar: () => void;
    org: string;
    categoria: Categoria | null;
    /** Chamado depois da exclusão, para a lista se atualizar. */
    aoExcluir: () => void;
};

type Vinculos = {
    produtos: ProdutoDaCategoria[];
    agentes: AgenteResumo[];
};

export function DialogoExcluirCategoria({
    aberto,
    aoFechar,
    org,
    categoria,
    aoExcluir,
}: DialogoExcluirCategoriaProps) {
    const { translate } = useI18n();
    const [carregando, setCarregando] = useState(false);
    const [vinculos, setVinculos] = useState<Vinculos | null>(null);
    const [falhaAoVerificar, setFalhaAoVerificar] = useState(false);

    useEffect(() => {
        if (!aberto || !categoria) return;
        let cancelado = false;
        setCarregando(true);
        setVinculos(null);
        setFalhaAoVerificar(false);
        Promise.all([
            produtosDaCategoria(org, categoria.id),
            agentesQueUsamCategoria(org, categoria.id),
        ])
            .then(([produtos, agentes]) => {
                if (!cancelado) setVinculos({ produtos, agentes });
            })
            .catch(() => {
                if (!cancelado) setFalhaAoVerificar(true);
            })
            .finally(() => {
                if (!cancelado) setCarregando(false);
            });
        return () => {
            cancelado = true;
        };
    }, [aberto, categoria, org]);

    if (!categoria) return null;

    const semVinculo = vinculos !== null && vinculos.produtos.length === 0 && vinculos.agentes.length === 0;
    const quantidadeProdutos = vinculos?.produtos.length ?? 0;

    function consequencia() {
        if (carregando) {
            return <SofaSkeleton lines={2} />;
        }
        if (falhaAoVerificar || vinculos === null) {
            return (
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {translate("structure.deleteCatCheckFailed")}
                </p>
            );
        }
        if (semVinculo) {
            return (
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {translate("structure.deleteCatNoLinks")}
                </p>
            );
        }
        const nomes = vinculos.agentes.map((agente) => agente.name).join(", ");
        const productsText =
            vinculos.produtos.length === 1
                ? translate("structure.deleteCatProductOne")
                : translate("structure.deleteCatProductMany", { count: vinculos.produtos.length });
        const productsPronoun =
            vinculos.produtos.length === 1
                ? translate("structure.deleteCatProductsPronounOne")
                : translate("structure.deleteCatProductsPronounMany");
        const agentsText =
            vinculos.agentes.length === 1
                ? translate("structure.deleteCatAgentOne")
                : translate("structure.deleteCatAgentMany", { count: vinculos.agentes.length });
        return (
            <div className="poph-pilha" style={{ gap: "var(--pop-spacing-sm, 8px)" }}>
                {vinculos.produtos.length > 0 && (
                    <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                        {translate("structure.deleteCatProductsLine", {
                            products: productsText,
                            pronoun: productsPronoun,
                        })}{" "}
                        <strong>
                            {vinculos.produtos.map((produto) => produto.name).join(", ")}
                        </strong>
                        .
                    </p>
                )}
                {vinculos.agentes.length > 0 && (
                    <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                        {translate("structure.deleteCatAgentsLine", { agents: agentsText })}{" "}
                        <strong>{nomes}</strong>.
                    </p>
                )}
            </div>
        );
    }

    const rotuloForcar =
        quantidadeProdutos > 0
            ? quantidadeProdutos === 1
                ? translate("structure.deleteCatForceWithProductsOne")
                : translate("structure.deleteCatForceWithProductsMany", { count: quantidadeProdutos })
            : translate("structure.deleteCatForceAgentsOnly");

    return (
        <DialogoExclusao
            aberto={aberto}
            aoFechar={aoFechar}
            titulo={translate("structure.deleteCatTitle")}
            descricao={translate("structure.deleteCatDescription", { name: categoria.name })}
            recurso={categoria.name}
            rotuloRecurso={translate("structure.deleteCatResourceLabel")}
            rotuloConfirmar={translate("structure.deleteCatConfirm")}
            consequencia={consequencia()}
            permiteForcar={!semVinculo}
            rotuloForcar={rotuloForcar}
            descricaoForcar={translate("structure.deleteCatForceDescription")}
            aoConfirmar={async (forcar) => {
                await excluirCategoria(org, categoria.id, forcar);
            }}
            aoExcluir={aoExcluir}
        />
    );
}

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

function plural(quantidade: number, singular: string, plurala: string): string {
    return `${quantidade} ${quantidade === 1 ? singular : plurala}`;
}

export function DialogoExcluirCategoria({
    aberto,
    aoFechar,
    org,
    categoria,
    aoExcluir,
}: DialogoExcluirCategoriaProps) {
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
                    Não consegui contar os produtos e agentes desta categoria. Se houver algum, a
                    API recusa a exclusão e mostra a contagem.
                </p>
            );
        }
        if (semVinculo) {
            return (
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    Nenhum produto nem agente usa esta categoria. Só o cadastro sai do sistema.
                </p>
            );
        }
        const nomes = vinculos.agentes.map((agente) => agente.name).join(", ");
        return (
            <div className="poph-pilha" style={{ gap: "var(--pop-spacing-sm, 8px)" }}>
                {vinculos.produtos.length > 0 && (
                    <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                        Esta categoria tem {plural(vinculos.produtos.length, "produto", "produtos")}{" "}
                        (de qualquer situação). Excluir apaga {vinculos.produtos.length === 1 ? "ele" : "todos eles"}:{" "}
                        <strong>
                            {vinculos.produtos.map((produto) => produto.name).join(", ")}
                        </strong>
                        .
                    </p>
                )}
                {vinculos.agentes.length > 0 && (
                    <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                        {plural(vinculos.agentes.length, "agente usa", "agentes usam")} esta
                        categoria e ficam sem ela: <strong>{nomes}</strong>.
                    </p>
                )}
            </div>
        );
    }

    const rotuloForcar =
        quantidadeProdutos > 0
            ? `Apagar também os ${quantidadeProdutos} ${quantidadeProdutos === 1 ? "produto desta categoria" : "produtos desta categoria"}`
            : "Tirar a categoria dos agentes e excluir";

    return (
        <DialogoExclusao
            aberto={aberto}
            aoFechar={aoFechar}
            titulo="Excluir categoria?"
            descricao={`A categoria ${categoria.name} sai do sistema. Não há como desfazer.`}
            recurso={categoria.name}
            rotuloRecurso="nome da categoria"
            rotuloConfirmar="Excluir categoria"
            consequencia={consequencia()}
            permiteForcar={!semVinculo}
            rotuloForcar={rotuloForcar}
            descricaoForcar="Sem esta confirmação, a API recusa a exclusão enquanto a categoria tiver produtos ou agentes vinculados."
            aoConfirmar={async (forcar) => {
                await excluirCategoria(org, categoria.id, forcar);
            }}
            aoExcluir={aoExcluir}
        />
    );
}

/**
 * Exclusão de grupo da base (HR-41), com a consequência escrita antes da confirmação.
 *
 * Ao abrir, conta os trechos do grupo e lista os agentes que o usam. O `DELETE` da API exige
 * `force=true` sempre — o grupo leva junto itens e vínculos —, então a confirmação por nome é o que
 * substitui o parâmetro: a tela só chama a API depois de o operador digitar o nome exato.
 */

import { useEffect, useState } from "react";
import { SofaSkeleton } from "@sofacoding/sofa-ui";
import { listarItens, excluirGrupo, type GrupoConhecimento, type ItemConhecimento } from "../api/conhecimento";
import { DialogoExclusao } from "./DialogoExclusao";

export type DialogoExcluirGrupoProps = {
    aberto: boolean;
    aoFechar: () => void;
    org: string;
    grupo: GrupoConhecimento | null;
    /** Id do agente → nome, para escrever quem usa o grupo. */
    nomesDosAgentes: Record<string, string>;
    /** Chamado depois da exclusão, para a lista se atualizar. */
    aoExcluir: () => void;
};

function plural(quantidade: number, singular: string, plurala: string): string {
    return `${quantidade} ${quantidade === 1 ? singular : plurala}`;
}

export function DialogoExcluirGrupo({
    aberto,
    aoFechar,
    org,
    grupo,
    nomesDosAgentes,
    aoExcluir,
}: DialogoExcluirGrupoProps) {
    const [carregando, setCarregando] = useState(false);
    const [itens, setItens] = useState<ItemConhecimento[] | null>(null);
    const [falhaAoContar, setFalhaAoContar] = useState(false);

    useEffect(() => {
        if (!aberto || !grupo) return;
        let cancelado = false;
        setCarregando(true);
        setItens(null);
        setFalhaAoContar(false);
        listarItens(org, grupo.id)
            .then((lista) => {
                if (!cancelado) setItens(lista);
            })
            .catch(() => {
                if (!cancelado) setFalhaAoContar(true);
            })
            .finally(() => {
                if (!cancelado) setCarregando(false);
            });
        return () => {
            cancelado = true;
        };
    }, [aberto, grupo, org]);

    if (!grupo) return null;

    const agentes = grupo.agents ?? [];

    function consequencia() {
        if (carregando) {
            return <SofaSkeleton lines={2} />;
        }
        if (falhaAoContar || itens === null) {
            return (
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    Não consegui contar os trechos deste grupo. A exclusão apaga o grupo com todos
                    os trechos e vínculos dele.
                </p>
            );
        }
        return (
            <div className="poph-pilha" style={{ gap: "var(--pop-spacing-sm, 8px)" }}>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {itens.length === 0
                        ? "O grupo está vazio: nenhum trecho será apagado."
                        : `O grupo tem ${plural(itens.length, "trecho", "trechos")}. Excluir apaga ${
                              itens.length === 1 ? "esse trecho" : "todos eles"
                          }: ${itens
                              .slice(0, 6)
                              .map((item) => item.title)
                              .join(", ")}${
                              itens.length > 6 ? ` e mais ${itens.length - 6}` : ""
                          }.`}
                </p>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {agentes.length === 0
                        ? "Nenhum agente usa este grupo hoje."
                        : `${plural(agentes.length, "agente usa", "agentes usam")} este grupo e ${
                              agentes.length === 1 ? "fica" : "ficam"
                          } sem ele: ${agentes
                              .map((id) => nomesDosAgentes[id] ?? id)
                              .join(", ")}.`}
                </p>
            </div>
        );
    }

    return (
        <DialogoExclusao
            aberto={aberto}
            aoFechar={aoFechar}
            titulo="Excluir grupo?"
            descricao={`O grupo ${grupo.name} sai do sistema com os trechos e os vínculos. Não há como desfazer.`}
            recurso={grupo.name}
            rotuloRecurso="nome do grupo"
            rotuloConfirmar="Excluir grupo"
            consequencia={consequencia()}
            permiteForcar={false}
            aoConfirmar={async () => {
                // A API exige `force=true` sempre; a trava aqui é a digitação do nome.
                await excluirGrupo(org, grupo.id);
            }}
            aoExcluir={aoExcluir}
        />
    );
}

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
import { useI18n } from "../i18n";
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

export function DialogoExcluirGrupo({
    aberto,
    aoFechar,
    org,
    grupo,
    nomesDosAgentes,
    aoExcluir,
}: DialogoExcluirGrupoProps) {
    const { translate: t } = useI18n();
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
                    {t("knowledge.deleteGroupCannotCount")}
                </p>
            );
        }
        const titles =
            itens
                .slice(0, 6)
                .map((item) => item.title)
                .join(", ") +
            (itens.length > 6
                ? t("knowledge.deleteGroupMoreItems", { count: itens.length - 6 })
                : "");
        return (
            <div className="poph-pilha" style={{ gap: "var(--pop-spacing-sm, 8px)" }}>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {itens.length === 0
                        ? t("knowledge.deleteGroupEmpty")
                        : t(
                              itens.length === 1
                                  ? "knowledge.deleteGroupItemsOne"
                                  : "knowledge.deleteGroupItemsMany",
                              { count: itens.length, titles },
                          )}
                </p>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {agentes.length === 0
                        ? t("knowledge.deleteGroupNoAgents")
                        : t(
                              agentes.length === 1
                                  ? "knowledge.deleteGroupAgentsOne"
                                  : "knowledge.deleteGroupAgentsMany",
                              {
                                  count: agentes.length,
                                  names: agentes
                                      .map((id) => nomesDosAgentes[id] ?? id)
                                      .join(", "),
                              },
                          )}
                </p>
            </div>
        );
    }

    return (
        <DialogoExclusao
            aberto={aberto}
            aoFechar={aoFechar}
            titulo={t("knowledge.deleteGroupTitle")}
            descricao={t("knowledge.deleteGroupDescription", { name: grupo.name })}
            recurso={grupo.name}
            rotuloRecurso={t("knowledge.deleteGroupResourceLabel")}
            rotuloConfirmar={t("knowledge.deleteGroupConfirm")}
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

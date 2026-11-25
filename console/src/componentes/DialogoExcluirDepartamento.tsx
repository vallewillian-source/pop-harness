/**
 * Exclusão de departamento (HR-39b), com a consequência escrita antes do botão.
 *
 * Ao abrir, lista os agentes que usam o departamento (padrão ou transferência) e diz quais ficam
 * sem ele. A API não pede `force` para isso: a CLI documenta o mesmo efeito ("agentes que o usavam
 * ficam sem ele"), então a tela só deixa claro o que vai acontecer e segue a regra digitada
 * (`DialogoExclusao`: digitar o nome exato).
 */

import { useEffect, useState } from "react";
import { SofaSkeleton } from "@sofacoding/sofa-ui";
import {
    agentesQueUsam,
    excluirDepartamento,
    listarAgentes,
    type AgenteResumo,
    type Departamento,
} from "../api/estrutura";
import { useI18n } from "../i18n";
import { DialogoExclusao } from "./DialogoExclusao";

export type DialogoExcluirDepartamentoProps = {
    aberto: boolean;
    aoFechar: () => void;
    org: string;
    departamento: Departamento | null;
    /** Chamado depois da exclusão, para a lista se atualizar. */
    aoExcluir: () => void;
};

export function DialogoExcluirDepartamento({
    aberto,
    aoFechar,
    org,
    departamento,
    aoExcluir,
}: DialogoExcluirDepartamentoProps) {
    const { translate } = useI18n();
    const [carregando, setCarregando] = useState(false);
    const [afetados, setAfetados] = useState<AgenteResumo[] | null>(null);
    const [falhaAoVerificar, setFalhaAoVerificar] = useState(false);

    useEffect(() => {
        if (!aberto || !departamento) return;
        let cancelado = false;
        setCarregando(true);
        setAfetados(null);
        setFalhaAoVerificar(false);
        listarAgentes(org)
            .then((agentes) => {
                if (!cancelado) setAfetados(agentesQueUsam(agentes, departamento.id));
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
    }, [aberto, departamento, org]);

    if (!departamento) return null;

    function consequencia() {
        if (carregando) {
            return <SofaSkeleton lines={2} />;
        }
        if (falhaAoVerificar || afetados === null) {
            return (
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {translate("structure.deleteDeptCheckFailed")}
                </p>
            );
        }
        if (afetados.length === 0) {
            return (
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {translate("structure.deleteDeptNoAgents")}
                </p>
            );
        }
        const nomes = afetados.map((agente) => agente.name).join(", ");
        return (
            <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                {afetados.length === 1
                    ? translate("structure.deleteDeptAgentsOnePrefix")
                    : translate("structure.deleteDeptAgentsManyPrefix", { count: afetados.length })}
                {" "}{translate("structure.deleteDeptAgentsMiddle")} <strong>{nomes}</strong>.{" "}
                {translate("structure.deleteDeptAgentsSuffix")}
            </p>
        );
    }

    return (
        <DialogoExclusao
            aberto={aberto}
            aoFechar={aoFechar}
            titulo={translate("structure.deleteDeptTitle")}
            descricao={translate("structure.deleteDeptDescription", { name: departamento.name })}
            recurso={departamento.name}
            rotuloRecurso={translate("structure.deleteDeptResourceLabel")}
            rotuloConfirmar={translate("structure.deleteDeptConfirm")}
            consequencia={consequencia()}
            aoConfirmar={async () => {
                await excluirDepartamento(org, departamento.id);
            }}
            aoExcluir={aoExcluir}
        />
    );
}

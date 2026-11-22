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
                    Não consegui verificar quais agentes usam este departamento. Os agentes que o
                    usam, como padrão ou para transferência, ficam sem ele depois da exclusão.
                </p>
            );
        }
        if (afetados.length === 0) {
            return (
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    Nenhum agente usa este departamento hoje. Só o cadastro sai do sistema.
                </p>
            );
        }
        const nomes = afetados.map((agente) => agente.name).join(", ");
        return (
            <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                {afetados.length === 1
                    ? "1 agente usa este departamento"
                    : `${afetados.length} agentes usam este departamento`}
                {" "}(padrão ou transferência) e ficam sem ele: <strong>{nomes}</strong>. Depois,
                esses agentes precisam de outro departamento para transferir ou receber conversas
                novas.
            </p>
        );
    }

    return (
        <DialogoExclusao
            aberto={aberto}
            aoFechar={aoFechar}
            titulo="Excluir departamento?"
            descricao={`O departamento ${departamento.name} sai do sistema. Não há como desfazer.`}
            recurso={departamento.name}
            rotuloRecurso="nome do departamento"
            rotuloConfirmar="Excluir departamento"
            consequencia={consequencia()}
            aoConfirmar={async () => {
                await excluirDepartamento(org, departamento.id);
            }}
            aoExcluir={aoExcluir}
        />
    );
}


import { useEffect, useState } from "react";
import { SofaSkeleton } from "@sofacoding/sofa-ui";
import {
    agentsUsing,
    deleteDepartment,
    listAgents,
    type AgentSummary,
    type Departamento,
} from "../api/structure";
import { useI18n } from "../i18n";
import { DialogExclusion } from "../components/DialogExclusion";

export type DialogDeleteDepartmentProps = {
    opened: boolean;
    onClose: () => void;
    org: string;
    departamento: Departamento | null;
    onDelete: () => void;
};

export function DialogDeleteDepartment({
    opened: isOpen,
    onClose,
    org,
    departamento,
    onDelete,
}: DialogDeleteDepartmentProps) {
    const { translate } = useI18n();
    const [loading, setLoading] = useState(false);
    const [afetados, setAfetados] = useState<AgentSummary[] | null>(null);
    const [failureVerify, setFailureVerify] = useState(false);

    useEffect(() => {
        if (!isOpen || !departamento) return;
        let cancelled = false;
        setLoading(true);
        setAfetados(null);
        setFailureVerify(false);
        listAgents(org)
            .then((agents) => {
                if (!cancelled) setAfetados(agentsUsing(agents, departamento.id));
            })
            .catch(() => {
                if (!cancelled) setFailureVerify(true);
            })
            .finally(() => {
                if (!cancelled) setLoading(false);
            });
        return () => {
            cancelled = true;
        };
    }, [isOpen, departamento, org]);

    if (!departamento) return null;

    function consequencia() {
        if (loading) {
            return <SofaSkeleton lines={2} />;
        }
        if (failureVerify || afetados === null) {
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
        const names = afetados.map((agente) => agente.name).join(", ");
        return (
            <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                {afetados.length === 1
                    ? translate("structure.deleteDeptAgentsOnePrefix")
                    : translate("structure.deleteDeptAgentsManyPrefix", { count: afetados.length })}
                {" "}{translate("structure.deleteDeptAgentsMiddle")} <strong>{names}</strong>.{" "}
                {translate("structure.deleteDeptAgentsSuffix")}
            </p>
        );
    }

    return (
        <DialogExclusion
            opened={isOpen}
            onClose={onClose}
            title={translate("structure.deleteDeptTitle")}
            descricao={translate("structure.deleteDeptDescription", { name: departamento.name })}
            resource={departamento.name}
            labelResource={translate("structure.deleteDeptResourceLabel")}
            labelConfirm={translate("structure.deleteDeptConfirm")}
            consequencia={consequencia()}
            onConfirm={async () => {
                await deleteDepartment(org, departamento.id);
            }}
            onDelete={onDelete}
        />
    );
}

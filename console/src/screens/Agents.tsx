
import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { createColumnHelper } from "@tanstack/react-table";
import {
    IconRefresh,
    IconUsers,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaTable,
} from "@sofacoding/sofa-ui";
import { listAgents, type Agente } from "../api/agents";
import { listStructure, type Departamento } from "../api/structure";
import { getOrg } from "../api/orgs";
import { translate, useI18n } from "../i18n";
import "../styles/agents.css";

const column = createColumnHelper<Agente>();

function classStatus(situacao: string): string {
    switch (situacao) {
        case "active":
            return "poph-status--active";
        case "inactive":
        case "archived":
            return "poph-status--inactive";
        default:
            return "poph-status--inactive";
    }
}

function labelStatus(situacao: string): string {
    switch (situacao) {
        case "active":
            return translate("agents.agentStatusActive");
        case "inactive":
            return translate("agents.agentStatusInactive");
        case "archived":
            return translate("agents.agentStatusArchived");
        default:
            return situacao;
    }
}

export function Agents() {
    const { locale, translate: t } = useI18n();
    const { orgId = "" } = useParams();
    const navigate = useNavigate();
    const [agents, setAgents] = useState<Agente[]>([]);
    const [departments, setDepartments] = useState<Departamento[]>([]);
    const [nameOrg, setOrgName] = useState<string | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);

    const load = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const list = await listAgents(orgId);
            setAgents(list);
        } catch (failure) {
            setError(failure instanceof Error ? failure.message : t("agents.loadFailed"));
        } finally {
            setLoading(false);
        }
    }, [orgId, t]);

    useEffect(() => {
        void load();
        getOrg(orgId)
            .then((org) => setOrgName(org.name))
            .catch(() => setOrgName(null));
        listStructure("departamento", orgId)
            .then((items) => setDepartments(items as Departamento[]))
            .catch(() => setDepartments([]));
    }, [load, orgId]);

    const nameDepartment = useMemo(() => {
        const map = new Map(departments.map((departamento) => [departamento.id, departamento.name]));
        return (id: string | null): string => {
            if (!id) return "—";
            return map.get(id) ?? id;
        };
    }, [departments]);

    const columns = useMemo(
        () => [
            column.accessor("name", {
                header: t("agents.listAgent"),
                cell: (info) => {
                    const agente = info.row.original;
                    const atenuado = agente.status !== "active";
                    return (
                        <div className="poph-agents__identity">
                            <span className={atenuado ? "poph-agents__name--dimmed" : "poph-agents__name"}>
                                {info.getValue()}
                            </span>
                            <span className="poph-mono-muted">{agente.slug}</span>
                        </div>
                    );
                },
            }),
            column.accessor("status", {
                header: t("agents.listStatus"),
                size: 130,
                cell: (info) => (
                    <span className={`poph-status ${classStatus(info.getValue())}`}>
                        <span className="poph-status__point" aria-hidden="true" />
                        {labelStatus(info.getValue())}
                    </span>
                ),
            }),
            column.accessor("default_department_id", {
                header: t("agents.listDefaultDepartment"),
                cell: (info) => {
                    const atenuado = info.row.original.status !== "active";
                    return (
                        <span className={atenuado ? "poph-agents__cell--dimmed" : undefined}>
                            {nameDepartment(info.getValue())}
                        </span>
                    );
                },
            }),
            column.accessor("version", {
                header: t("agents.listVersion"),
                size: 90,
                cell: (info) => <span className="poph-mono-muted">v{info.getValue()}</span>,
            }),
            column.display({
                id: "acoes",
                header: t("agents.listActions"),
                cell: (info) => (
                    <div className="poph-agents__actions">
                        <SofaButton
                            variant="subtle"
                            size="sm"
                            onClick={() =>
                                navigate(`/orgs/${orgId}/agents/${info.row.original.id}/instructions`)
                            }
                        >
                            {t("agents.openInstructions")}
                        </SofaButton>
                    </div>
                ),
            }),
        ],
        [navigate, orgId, nameDepartment, t, locale],
    );

    return (
        <div className="poph-page">
            <div className="poph-page__top">
                <div>
                    <p className="poph-agents__context">
                        {nameOrg
                            ? t("agents.orgContextWithName", { name: nameOrg })
                            : t("agents.orgContext")}
                    </p>
                    <h1 className="poph-page__title">
                        {t("agents.pageTitle")}
                        {!loading && (
                            <span className="poph-agents__count">({agents.length})</span>
                        )}
                    </h1>
                    <p className="poph-page__description">
                        {t("agents.pageDescription")}
                    </p>
                </div>
                <div className="poph-page__actions">
                    <SofaButton
                        variant="secondary"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void load()}
                        loading={loading}
                    >
                        {t("agents.refresh")}
                    </SofaButton>
                </div>
            </div>

            {error && !loading ? (
                <SofaErrorState
                    title={t("agents.loadErrorTitle")}
                    description={t("agents.loadErrorDescription")}
                    details={error}
                    retry={{ label: t("agents.retry"), onClick: () => void load() }}
                />
            ) : (
                <SofaTable
                    className="poph-agents__table"
                    label={t("agents.pageTitle")}
                    columns={columns}
                    data={agents}
                    getRowId={(agente) => agente.id}
                    isLoading={loading}
                    empty={
                        <SofaEmptyState
                            icon={<IconUsers aria-hidden="true" />}
                            title={t("agents.emptyTitle")}
                            description={t("agents.emptyDescription")}
                        />
                    }
                    footerMeta={({ total }) =>
                        total === 1
                            ? t("agents.countOne")
                            : t("agents.countMany", { total })
                    }
                />
            )}
        </div>
    );
}

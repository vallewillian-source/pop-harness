
import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconKey,
    IconMoreHorizontal,
    IconPencil,
    IconPlus,
    IconRefresh,
    IconTrash,
    IconWorkflow,
    SofaBanner,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaIconButton,
    SofaMenu,
    SofaSkeleton,
    type SofaMenuItemSpec,
} from "@sofacoding/sofa-ui";
import { getAgent, type Agente } from "../api/agents";
import {
    KIND_WHATSAPP,
    deleteIntegration,
    listIntegrations,
    connectionLabel,
    type Integracao,
} from "../api/integrations";
import { getOrg } from "../api/orgs";
import { TabsAgent, HeaderAgent } from "../components/TabsAgent";
import { DialogExclusion } from "../components/DialogExclusion";
import { useGuardChanges } from "../components/GuardChanges";
import { ModalIntegration } from "../components/ModalIntegration";
import { PanelWhatsApp } from "../components/PanelWhatsApp";
import { translate, useI18n } from "../i18n";
import "../styles/integrations.css";

function classStatus(situacao: string): string {
    switch (situacao) {
        case "active":
            return "poph-status--active";
        case "inactive":
            return "poph-status--attention";
        default:
            return "poph-status--inactive";
    }
}

function labelStatus(situacao: string): string {
    switch (situacao) {
        case "active":
            return translate("integrations.statusActive");
        case "inactive":
            return translate("integrations.statusInactive");
        case "archived":
            return translate("integrations.statusArchived");
        default:
            return situacao;
    }
}

function classConnection(estado: string): string {
    switch (estado) {
        case "connected":
            return "poph-status--active";
        case "awaiting_scan":
            return "poph-status--attention";
        case "disconnected":
            return "poph-status--error";
        default:
            return "poph-status--inactive";
    }
}

export function AgentIntegrations() {
    const { translate: t } = useI18n();
    const { orgId = "", agentId = "" } = useParams();
    const guard = useGuardChanges();

    const [agente, setAgent] = useState<Agente | null>(null);
    const [nameOrg, setOrgName] = useState<string | null>(null);
    const [errorLoad, setLoadError] = useState<string | null>(null);

    const [integrations, setIntegrations] = useState<Integracao[]>([]);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);

    const [modalOpened, setModalOpened] = useState(false);
    const [edit, setInEdit] = useState<Integracao | null>(null);
    const [toDelete, setToDelete] = useState<Integracao | null>(null);
    const [notice, setNotice] = useState<string | null>(null);

    const loadIntegrations = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            setIntegrations(await listIntegrations(orgId, agentId));
        } catch (failure) {
            setError(
                failure instanceof Error
                    ? failure.message
                    : t("integrations.loadIntegrationsFallbackError"),
            );
            setIntegrations([]);
        } finally {
            setLoading(false);
        }
    }, [orgId, agentId, t]);

    const updateIntegration = useCallback((updated: Integracao) => {
        setIntegrations((current) =>
            current.map((item) => (item.id === updated.id ? updated : item)),
        );
    }, []);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            setAgent(await getAgent(orgId, agentId));
        } catch (failure) {
            setLoadError(
                failure instanceof Error ? failure.message : t("integrations.loadAgentFallbackError"),
            );
            return;
        }
        void loadIntegrations();
    }, [orgId, agentId, loadIntegrations, t]);

    useEffect(() => {
        void load();
        getOrg(orgId)
            .then((org) => setOrgName(org.name))
            .catch(() => setOrgName(null));
    }, [load, orgId]);

    const doWhatsapp = useMemo(
        () => integrations.filter((integracao) => integracao.kind === KIND_WHATSAPP),
        [integrations],
    );

    if (!agente && errorLoad) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("integrations.loadAgentErrorTitle")}
                    description={t("integrations.loadAgentErrorDescription")}
                    details={errorLoad}
                    retry={{ label: t("integrations.retry"), onClick: () => void load() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guard.tryNavigate(`/orgs/${orgId}/agents`)}
                        >
                            {t("integrations.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente) {
        return (
            <div className="poph-page" aria-busy="true">
                <div className="poph-page__top">
                    <div>
                        <SofaSkeleton width={260} height={26} />
                        <div className="poph-integrations__skeleton-line">
                            <SofaSkeleton width={360} height={12} />
                        </div>
                    </div>
                </div>
                <SofaSkeleton lines={6} />
            </div>
        );
    }

    function itemsMenu(integracao: Integracao): SofaMenuItemSpec[] {
        return [
            {
                id: "editar",
                label: t("integrations.actionEdit"),
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => {
                    setNotice(null);
                    setInEdit(integracao);
                    setModalOpened(true);
                },
            },
            {
                id: "excluir",
                label: t("integrations.actionDelete"),
                icon: <IconTrash size={14} aria-hidden="true" />,
                danger: true,
                onSelect: () => {
                    setNotice(null);
                    setToDelete(integracao);
                },
            },
        ];
    }

    let corpo;
    if (loading && integrations.length === 0) {
        corpo = <SofaSkeleton lines={5} />;
    } else if (error) {
        corpo = (
            <SofaErrorState
                title={t("integrations.loadListErrorTitle")}
                description={t("integrations.loadListErrorDescription")}
                details={error}
                retry={{ label: t("integrations.retry"), onClick: () => void loadIntegrations() }}
            />
        );
    } else if (integrations.length === 0) {
        corpo = (
            <SofaEmptyState
                icon={<IconWorkflow aria-hidden="true" />}
                title={t("integrations.emptyTitle")}
                description={t("integrations.emptyDescription")}
                action={
                    <SofaButton
                        variant="primary"
                        icon={<IconPlus aria-hidden="true" />}
                        onClick={() => {
                            setInEdit(null);
                            setModalOpened(true);
                        }}
                    >
                        {t("integrations.emptyAction")}
                    </SofaButton>
                }
            />
        );
    } else {
        corpo = (
            <>
            <div
                className="poph-table poph-integrations__table"
                role="table"
                aria-label={t("integrations.tableAriaLabel")}
            >
                <div className="poph-table__header" role="row">
                    <span role="columnheader">{t("integrations.colType")}</span>
                    <span role="columnheader">{t("integrations.colStatus")}</span>
                    <span role="columnheader">{t("integrations.colConnection")}</span>
                    <span role="columnheader">{t("integrations.colSecret")}</span>
                    <span role="columnheader">{t("integrations.colConfig")}</span>
                    <span role="columnheader">{t("integrations.colVersion")}</span>
                    <span role="columnheader" className="poph-table__cell--actions">
                        {t("integrations.colActions")}
                    </span>
                </div>
                {integrations.map((integracao) => {
                    const configuracao = integracao.configuracao;
                    const keys =
                        configuracao && typeof configuracao === "object"
                            ? Object.keys(configuracao).length
                            : 0;
                    const classes = [
                        "poph-table__line",
                        integracao.status !== "active" ? "poph-table__line--dimmed" : "",
                    ]
                        .filter(Boolean)
                        .join(" ");
                    const nameMenu = t("integrations.rowMenuLabel", {
                        kind: integracao.kind,
                        id: integracao.id,
                    });
                    return (
                        <div key={integracao.id} role="row" className={classes}>
                            <div role="cell" className="poph-integrations__identity">
                                <span className="poph-integrations__kind">{integracao.kind}</span>
                                <span className="poph-mono-muted">{integracao.id}</span>
                            </div>
                            <div role="cell">
                                <span className={`poph-status ${classStatus(integracao.status)}`}>
                                    <span className="poph-status__point" aria-hidden="true" />
                                    {labelStatus(integracao.status)}
                                </span>
                            </div>
                            <div role="cell">
                                <span
                                    className={`poph-status ${classConnection(integracao.connection_state)}`}
                                >
                                    <span className="poph-status__point" aria-hidden="true" />
                                    {connectionLabel(integracao.connection_state)}
                                </span>
                            </div>
                            <div role="cell" className="poph-table__cell--mono">
                                {integracao.tem_segredo
                                    ? t("integrations.secretSavedCell")
                                    : t("integrations.secretNoneCell")}
                            </div>
                            <div role="cell" className="poph-table__cell--mono">
                                {keys === 0
                                    ? t("integrations.configEmptyCell")
                                    : keys === 1
                                      ? t("integrations.configOneField")
                                      : t("integrations.configManyFields", { count: keys })}
                            </div>
                            <div role="cell" className="poph-table__cell--mono">
                                v{integracao.version}
                            </div>
                            <div
                                role="cell"
                                className="poph-table__cell--actions poph-integrations__actions"
                            >
                                <SofaMenu
                                    label={nameMenu}
                                    align="end"
                                    trigger={
                                        <SofaIconButton label={nameMenu} size={28}>
                                            <IconMoreHorizontal size={16} aria-hidden="true" />
                                        </SofaIconButton>
                                    }
                                    items={itemsMenu(integracao)}
                                />
                            </div>
                        </div>
                    );
                })}
            </div>
            <p className="poph-table__footer">
                {integrations.length === 1
                    ? t("integrations.footerOne")
                    : t("integrations.footerMany", { count: integrations.length })}
            </p>
            </>
        );
    }

    return (
        <>
            <div className="poph-page poph-integrations">
                <HeaderAgent
                    org={orgId}
                    nome={agente.name}
                    slug={agente.slug}
                    situacao={agente.status}
                    organization={nameOrg}
                    descricao={t("integrations.headerDescription")}
                    actions={
                        <>
                            <SofaButton
                                variant="secondary"
                                icon={<IconRefresh aria-hidden="true" />}
                                onClick={() => void loadIntegrations()}
                                loading={loading && integrations.length > 0}
                            >
                                {t("integrations.refresh")}
                            </SofaButton>
                            <SofaButton
                                variant="primary"
                                icon={<IconPlus aria-hidden="true" />}
                                onClick={() => {
                                    setInEdit(null);
                                    setModalOpened(true);
                                }}
                            >
                                {t("integrations.newIntegration")}
                            </SofaButton>
                        </>
                    }
                />

                <TabsAgent org={orgId} agente={agentId} active="integrations" nome={agente.name} model={agente.model ?? null} />

                {notice && (
                    <p className="poph-mono-muted" style={{ margin: 0 }} role="status">
                        {notice}
                    </p>
                )}

                <SofaBanner tone="info" title={t("integrations.pluginFormBannerTitle")}>
                    {t("integrations.pluginFormBannerBody", { whatsapp: KIND_WHATSAPP })}
                </SofaBanner>

                <section className="poph-section">
                    <div className="poph-integrations__header-section">
                        <div className="poph-integrations__title-block">
                            <h2 className="poph-section__title">
                                {t("integrations.sectionIntegrationsTitle")}
                            </h2>
                            <p className="poph-integrations__help">
                                {t("integrations.sectionIntegrationsHelp")}
                            </p>
                        </div>
                    </div>
                    {corpo}
                </section>

                <section className="poph-section">
                    <div className="poph-integrations__header-section">
                        <div className="poph-integrations__title-block">
                            <h2 className="poph-section__title">
                                {t("integrations.sectionWhatsappTitle")}
                            </h2>
                            <p className="poph-integrations__help">
                                {t("integrations.sectionWhatsappHelp")}
                            </p>
                        </div>
                        <div className="poph-integrations__actions-section">
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                icon={<IconRefresh aria-hidden="true" />}
                                onClick={() => void loadIntegrations()}
                                loading={loading && integrations.length > 0}
                            >
                                {t("integrations.refresh")}
                            </SofaButton>
                        </div>
                    </div>
                    {loading && doWhatsapp.length === 0 ? (
                        <SofaSkeleton lines={2} />
                    ) : doWhatsapp.length === 0 ? (
                        <p className="poph-mono-muted" style={{ margin: 0 }}>
                            {t("integrations.whatsappEmpty", { whatsapp: KIND_WHATSAPP })}
                        </p>
                    ) : (
                        <div className="poph-integrations__panels">
                            {doWhatsapp.map((integracao) => (
                                <PanelWhatsApp
                                    key={integracao.id}
                                    org={orgId}
                                    agente={agentId}
                                    integracao={integracao}
                                    onUpdateIntegration={updateIntegration}
                                />
                            ))}
                        </div>
                    )}
                </section>

                <section className="poph-section">
                    <div className="poph-integrations__header-section">
                        <div className="poph-integrations__title-block">
                            <h2 className="poph-section__title">
                                {t("integrations.sectionSecretsTitle")}
                            </h2>
                            <p className="poph-integrations__help">
                                {t("integrations.sectionSecretsHelp")}
                            </p>
                        </div>
                    </div>
                    <div className="poph-integrations__secrets">
                        <IconKey aria-hidden="true" />
                        <span className="poph-mono-muted">
                            {t("integrations.secretsFooterNote")}
                        </span>
                    </div>
                </section>
            </div>

            <ModalIntegration
                org={orgId}
                agente={agentId}
                opened={modalOpened}
                onClose={() => {
                    setModalOpened(false);
                    setInEdit(null);
                }}
                integracao={edit}
                onSave={(saved) => {
                    setIntegrations((current) => {
                        const exists = current.some((item) => item.id === saved.id);
                        return exists
                            ? current.map((item) => (item.id === saved.id ? saved : item))
                            : [...current, saved];
                    });
                    setNotice(
                        edit
                            ? t("integrations.savedNotice", {
                                  kind: saved.kind,
                                  version: saved.version,
                              })
                            : t("integrations.createdNotice", { kind: saved.kind }),
                    );
                }}
            />

            <DialogExclusion
                opened={toDelete !== null}
                onClose={() => setToDelete(null)}
                title={t("integrations.deleteModalTitle")}
                descricao={
                    toDelete
                        ? t("integrations.deleteModalDescription", { kind: toDelete.kind })
                        : ""
                }
                resource={toDelete?.kind ?? ""}
                labelResource={t("integrations.deleteModalResourceLabel")}
                labelConfirm={t("integrations.deleteModalConfirmLabel")}
                consequencia={
                    <span className="poph-mono-muted">
                        {t("integrations.deleteModalConsequence", {
                            id: toDelete?.id ?? "",
                        })}
                    </span>
                }
                onConfirm={async () => {
                    if (!toDelete) return;
                    await deleteIntegration(orgId, agentId, toDelete.id);
                }}
                onDelete={() => {
                    if (toDelete) {
                        setIntegrations((current) =>
                            current.filter((item) => item.id !== toDelete.id),
                        );
                        setNotice(t("integrations.deletedNotice", { kind: toDelete.kind }));
                    }
                }}
            />
        </>
    );
}

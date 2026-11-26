
import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import {
    IconBot,
    IconBuilding,
    IconCheck,
    IconChevronRight,
    IconClose,
    IconCopy,
    IconTrash,
    SofaButton,
    SofaIconButton,
} from "@sofacoding/sofa-ui";
import { listAgents, type Agente } from "../api/agents";
import type { Org } from "../api/client";
import { readMatrixState, type MatrixState } from "../api/matrix";
import { translate, useI18n } from "../i18n";
import { formatDate, formatTime } from "../i18n/format";
import "../styles/inspector.css";

export type OrgNode = { id: string; situacao: string };

export type InspectorOrgProps = {
    org: Org;
    no?: OrgNode | null;
    canManage?: boolean;
    remote?: boolean;
    onClose: () => void;
    onAccess: (org: Org) => void;
    onEdit?: (org: Org) => void;
    onExport?: (org: Org) => void;
    aoExcluir?: (org: Org) => void;
};

const DEADLINE_READ_MS = 12_000;

const AGENTS_BLOCK = 8;

type Resource<T> =
    | { estado: "carregando" }
    | { estado: "pronto"; dados: T }
    | { estado: "erro"; mensagem: string };

function useRead<T>(chave: string, read: () => Promise<T>): Resource<T> {
    const [resource, setResource] = useState<Resource<T>>({ estado: "carregando" });
    const readNow = useRef(read);
    readNow.current = read;

    useEffect(() => {
        let ended = false;
        setResource({ estado: "carregando" });
        const deadline = setTimeout(() => {
            if (ended) return;
            ended = true;
            setResource({ estado: "erro", mensagem: translate("orgs.inspectorBacklogTimeout") });
        }, DEADLINE_READ_MS);
        readNow
            .current()
            .then((dados) => {
                if (ended) return;
                ended = true;
                clearTimeout(deadline);
                setResource({ estado: "pronto", dados });
            })
            .catch((failure: unknown) => {
                if (ended) return;
                ended = true;
                clearTimeout(deadline);
                setResource({ estado: "erro", mensagem: messageError(failure) });
            });
        return () => {
            ended = true;
            clearTimeout(deadline);
        };
    }, [chave]);

    return resource;
}

function labelSituation(situacao: string): string {
    switch (situacao) {
        case "active":
            return translate("orgs.statusActive");
        case "inactive":
            return translate("orgs.statusInactive");
        case "archived":
            return translate("orgs.statusArchived");
        default:
            return situacao;
    }
}

function toneSituation(situacao: string): string {
    return situacao === "active" ? "poph-status--active" : "poph-status--inactive";
}

function labelAgent(situacao: string): string {
    switch (situacao) {
        case "active":
            return translate("orgs.agentStatusActive");
        case "inactive":
            return translate("orgs.agentStatusInactive");
        case "archived":
            return translate("orgs.agentStatusArchived");
        default:
            return situacao;
    }
}

function retencao(org: Org): string {
    const config = org.config as Record<string, unknown> | undefined;
    const value = config?.["retention_days"] ?? config?.["retencao_dias"];
    if (typeof value === "number") {
        return value === 0
            ? translate("orgs.retentionNever")
            : translate("orgs.retentionDays", { days: value });
    }
    return translate("orgs.retentionDefault");
}

function dataEHora(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    return translate("orgs.dateTimeAt", { date: formatDate(ms), time: formatTime(ms) });
}

function messageError(failure: unknown): string {
    if (failure instanceof Error && failure.message) return failure.message;
    return translate("orgs.inspectorQueryFailed");
}

function nodeText(no: OrgNode | null | undefined): { chip: string; valor: string } | null {
    if (!no) return null;
    if (no.id === "") {
        return { chip: translate("orgs.nodeLocal"), valor: translate("orgs.nodeLocal") };
    }
    switch (no.situacao) {
        case "local":
            return {
                chip: translate("orgs.nodePrimary"),
                valor: translate("orgs.nodePrimaryValue", { id: no.id }),
            };
        case "ok":
            return {
                chip: translate("orgs.nodeChip", { id: no.id }),
                valor: translate("orgs.nodeRemoteOnline", { id: no.id }),
            };
        case "inacessivel":
            return {
                chip: translate("orgs.nodeChip", { id: no.id }),
                valor: translate("orgs.nodeRemoteUnreachable", { id: no.id }),
            };
        default:
            return { chip: translate("orgs.nodeChip", { id: no.id }), valor: no.id };
    }
}

function mirrorMatrix(estado: MatrixState): { valor: string; detalhe: string } {
    const pending = estado.backlog?.messages ?? 0;
    const age = estado.backlog?.oldest_seconds ?? 0;
    const parts: string[] = [];
    if (pending > 0) {
        parts.push(
            translate(pending === 1 ? "orgs.matrixBacklogOne" : "orgs.matrixBacklogMany", {
                count: pending,
            }),
        );
        if (age > 0) parts.push(translate("orgs.matrixOldest", { seconds: age }));
    }
    if (estado.pending_failures > 0) {
        parts.push(
            translate(
                estado.pending_failures === 1 ? "orgs.matrixFailureOne" : "orgs.matrixFailureMany",
                { count: estado.pending_failures },
            ),
        );
    }
    if (estado.last_failure) {
        parts.push(translate("orgs.matrixLastFailure", { reason: estado.last_failure }));
    }
    const detail =
        parts.length > 0 ? parts.join(" · ") : translate("orgs.matrixNothingPending");

    if (estado.connected === null || estado.connected === undefined) {
        return {
            valor: translate("orgs.matrixNotConfigured"),
            detalhe: translate("orgs.matrixNotConfiguredDetail"),
        };
    }
    if (!estado.enabled) {
        return {
            valor: translate("orgs.matrixDisabled"),
            detalhe: translate("orgs.matrixDisabledDetail", { detail: detail }),
        };
    }
    if (estado.connected) {
        return { valor: translate("orgs.matrixConnected"), detalhe: detail };
    }
    return {
        valor: translate("orgs.matrixDisconnected"),
        detalhe: translate("orgs.matrixDisconnectedDetail", { detail: detail }),
    };
}

function metaAgent(agente: Agente): string {
    const modelo = agente.model?.trim()
        ? agente.model.trim()
        : translate("orgs.agentModelDefault");
    return `${modelo} · ${labelAgent(agente.status)}`;
}

export function InspectorOrg({
    org,
    no = null,
    canManage = true,
    remote = false,
    onClose,
    onAccess,
    onEdit,
    onExport,
    aoExcluir: onDelete,
}: InspectorOrgProps) {
    const { translate: t } = useI18n();
    const drawer = useRef<HTMLElement | null>(null);
    const [copied, setCopied] = useState(false);
    const [failureCopy, setFailureCopy] = useState<string | null>(null);
    const clockCopied = useRef<ReturnType<typeof setTimeout> | null>(null);

    const agents = useRead(`agentes:${org.id}`, useCallback(() => listAgents(org.id), [org.id]));
    const matrix = useRead(`matrix:${org.id}`, useCallback(() => readMatrixState(org.id), [org.id]));

    useEffect(() => {
        drawer.current?.focus();
    }, []);

    const close = useCallback(() => onClose(), [onClose]);

    useEffect(() => {
        const onPress = (evento: KeyboardEvent) => {
            if (evento.key !== "Escape" || evento.defaultPrevented) return;
            const target = evento.target as HTMLElement | null;
            if (target?.closest?.('[role="menu"], [role="dialog"], [role="listbox"]')) return;
            evento.preventDefault();
            close();
        };
        window.addEventListener("keydown", onPress);
        return () => window.removeEventListener("keydown", onPress);
    }, [close]);

    useEffect(() => {
        setCopied(false);
        setFailureCopy(null);
    }, [org.id]);

    useEffect(
        () => () => {
            if (clockCopied.current !== null) clearTimeout(clockCopied.current);
        },
        [],
    );

    const agentsList = agents.estado === "pronto" ? agents.dados : [];
    const firstActive = agentsList.find((agente) => agente.status === "active") ?? null;
    const command = `poph chat --org ${org.slug} ${firstActive?.slug ?? "<agente>"}`;
    const describeNode = nodeText(no);
    const idShort = org.id.length > 8 ? `${org.id.slice(0, 8)}…` : org.id;
    const agentsBlock = agentsList.slice(0, AGENTS_BLOCK);

    async function copy() {
        setFailureCopy(null);
        try {
            if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
                throw new Error("clipboard indisponível");
            }
            await navigator.clipboard.writeText(command);
        } catch {
            setCopied(false);
            setFailureCopy(t("orgs.inspectorCopyFailed"));
            return;
        }
        setCopied(true);
        if (clockCopied.current !== null) clearTimeout(clockCopied.current);
        clockCopied.current = setTimeout(() => setCopied(false), 4000);
    }

    return (
        <aside
            ref={drawer}
            className="poph-drawer poph-inspector"
            role="complementary"
            aria-label={t("orgs.inspectorAriaLabel", { name: org.name })}
            tabIndex={-1}
        >
            <header className="poph-inspector__top">
                <div className="poph-inspector__top-line">
                    <span className="poph-inspector__tile" aria-hidden="true">
                        <IconBuilding size={14} aria-hidden="true" />
                    </span>
                    <h2 className="poph-inspector__title">{org.name}</h2>
                    <SofaIconButton label={t("orgs.inspectorClose")} size={24} onClick={close}>
                        <IconClose aria-hidden="true" />
                    </SofaIconButton>
                </div>
                <p className="poph-inspector__subtitle">
                    {t("orgs.inspectorSubtitle", { slug: org.slug, version: org.version })}
                </p>
            </header>

            <div className="poph-inspector__body">
                <div className="poph-inspector__badges">
                    <span className={`poph-status ${toneSituation(org.status)}`}>
                        <span className="poph-status__point" aria-hidden="true" />
                        {labelSituation(org.status)}
                    </span>
                    {describeNode && (
                        <span
                            className="poph-inspector__chip"
                            title={describeNode.chip === describeNode.valor ? undefined : describeNode.valor}
                        >
                            {describeNode.chip}
                        </span>
                    )}
                    <span className="poph-inspector__chip poph-inspector__chip--mono" title={org.id}>
                        ID: {idShort}
                    </span>
                </div>

                <section className="poph-inspector__section" aria-label={t("orgs.inspectorCommandAria")}>
                    <div className="poph-terminal">
                        <code className="poph-terminal__command">{command}</code>
                        <SofaIconButton
                            label={t("orgs.inspectorCopyCommand")}
                            size={24}
                            className="poph-terminal__copy"
                            onClick={() => void copy()}
                        >
                            <IconCopy size={13} aria-hidden="true" />
                        </SofaIconButton>
                    </div>
                    <p
                        className={[
                            "poph-inspector__notice",
                            copied || failureCopy ? "poph-inspector__notice--visible" : "",
                            failureCopy ? "poph-inspector__notice--error" : "",
                        ]
                            .filter(Boolean)
                            .join(" ")}
                        role="status"
                        aria-live="polite"
                    >
                        {copied ? (
                            <>
                                <IconCheck aria-hidden="true" /> {t("orgs.inspectorCopied")}
                            </>
                        ) : (
                            failureCopy ?? ""
                        )}
                    </p>
                </section>

                <section
                    className="poph-inspector__section poph-inspector__section--properties"
                    aria-label={t("orgs.inspectorPropertiesAria")}
                >
                    <dl className="poph-properties">
                        <div className="poph-property">
                            <dt className="poph-property__key">{t("orgs.inspectorRetention")}</dt>
                            <dd className="poph-property__value">{retencao(org)}</dd>
                        </div>
                        <div className="poph-property">
                            <dt className="poph-property__key">{t("orgs.inspectorMatrix")}</dt>
                            <dd className="poph-property__value">
                                {matrix.estado === "carregando" && (
                                    <span className="poph-inspector__resource">
                                        {t("orgs.inspectorLoading")}
                                    </span>
                                )}
                                {matrix.estado === "erro" && (
                                    <span
                                        className="poph-inspector__resource poph-inspector__resource--error"
                                        title={matrix.mensagem}
                                    >
                                        {t("orgs.inspectorUnavailable")}
                                    </span>
                                )}
                                {matrix.estado === "pronto" &&
                                    (() => {
                                        const mirror = mirrorMatrix(matrix.dados);
                                        return (
                                            <span className="poph-inspector__with-hint" title={mirror.detalhe}>
                                                {mirror.valor}
                                            </span>
                                        );
                                    })()}
                            </dd>
                        </div>
                        <div className="poph-property">
                            <dt className="poph-property__key">{t("orgs.inspectorInstance")}</dt>
                            <dd className="poph-property__value">
                                {describeNode ? (
                                    <span className="poph-property__value--mono">{describeNode.valor}</span>
                                ) : (
                                    <span
                                        className="poph-inspector__resource"
                                        title={t("orgs.inspectorInstanceUnknown")}
                                    >
                                        —
                                    </span>
                                )}
                            </dd>
                        </div>
                        <div className="poph-property">
                            <dt className="poph-property__key">{t("orgs.inspectorUpdatedAt")}</dt>
                            <dd className="poph-property__value">{dataEHora(org.updated_at)}</dd>
                        </div>
                        <div className="poph-property">
                            <dt className="poph-property__key">{t("orgs.inspectorCreatedAt")}</dt>
                            <dd className="poph-property__value">{dataEHora(org.created_at)}</dd>
                        </div>
                    </dl>
                </section>

                <section className="poph-inspector__section" aria-labelledby="poph-inspetor-agentes">
                    <h3 id="poph-inspetor-agentes" className="poph-inspector__section-title">
                        {t("orgs.inspectorAgentsTitle")}
                        {agents.estado === "pronto" && agentsList.length > 0 && (
                            <span className="poph-inspector__count"> ({agentsList.length})</span>
                        )}
                    </h3>
                    {agents.estado === "carregando" && (
                        <p className="poph-inspector__resource">{t("orgs.inspectorAgentsLoading")}</p>
                    )}
                    {agents.estado === "erro" && (
                        <p
                            className="poph-inspector__resource poph-inspector__resource--error"
                            title={agents.mensagem}
                        >
                            {t("orgs.inspectorAgentsError")}
                        </p>
                    )}
                    {agents.estado === "pronto" && agentsList.length === 0 && (
                        <p className="poph-inspector__resource">{t("orgs.inspectorAgentsEmpty")}</p>
                    )}
                    {agents.estado === "pronto" && agentsList.length > 0 && (
                        <>
                            <ul className="poph-inspector__agents">
                                {agentsBlock.map((agente) => {
                                    const content = (
                                        <>
                                            <span className="poph-inspector__agent-tile" aria-hidden="true">
                                                <IconBot size={14} aria-hidden="true" />
                                            </span>
                                            <span className="poph-inspector__agent-text">
                                                <span className="poph-inspector__agent-name">{agente.name}</span>
                                                <span className="poph-inspector__agent-meta">
                                                    {metaAgent(agente)}
                                                </span>
                                            </span>
                                            <IconChevronRight size={14} aria-hidden="true" />
                                        </>
                                    );
                                    return (
                                        <li key={agente.id}>
                                            {remote ? (
                                                <div
                                                    className="poph-inspector__agent poph-inspector__agent--static"
                                                    title={t("orgs.inspectorAgentRemote")}
                                                >
                                                    {content}
                                                </div>
                                            ) : (
                                                <Link
                                                    className="poph-inspector__agent"
                                                    to={`/orgs/${org.id}/agents/${agente.id}`}
                                                >
                                                    {content}
                                                </Link>
                                            )}
                                        </li>
                                    );
                                })}
                            </ul>
                            {agentsList.length > agentsBlock.length && (
                                <Link className="poph-inspector__see-all" to={`/orgs/${org.id}/agents`}>
                                    {t("orgs.inspectorSeeAll", { count: agentsList.length })}
                                </Link>
                            )}
                        </>
                    )}
                </section>
            </div>

            <footer className="poph-inspector__footer">
                <SofaButton
                    variant="primary"
                    fullWidth
                    disabled={remote}
                    title={remote ? t("orgs.inspectorRemoteTitle") : undefined}
                    onClick={() => onAccess(org)}
                >
                    {t("orgs.inspectorAccess")}
                </SofaButton>
                {canManage && (
                    <div className="poph-inspector__footer-secondary">
                        <SofaButton
                            variant="secondary"
                            size="sm"
                            disabled={remote || onEdit === undefined}
                            onClick={() => onEdit?.(org)}
                        >
                            {t("orgs.inspectorEdit")}
                        </SofaButton>
                        <SofaButton
                            variant="secondary"
                            size="sm"
                            disabled={onExport === undefined}
                            onClick={() => onExport?.(org)}
                        >
                            {t("orgs.inspectorExport")}
                        </SofaButton>
                        <SofaIconButton
                            label={t("orgs.inspectorDeleteLabel", { name: org.name })}
                            variant="danger"
                            size={32}
                            disabled={remote || onDelete === undefined}
                            onClick={() => onDelete?.(org)}
                        >
                            <IconTrash size={14} aria-hidden="true" />
                        </SofaIconButton>
                    </div>
                )}
            </footer>
        </aside>
    );
}

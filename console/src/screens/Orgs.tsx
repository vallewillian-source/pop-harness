
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { createColumnHelper } from "@tanstack/react-table";
import {
    IconBuilding,
    IconDownload,
    IconEye,
    IconMoreHorizontal,
    IconPencil,
    IconPlay,
    IconPlus,
    IconRefresh,
    IconTrash,
    SofaBanner,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaIconButton,
    SofaMenu,
    SofaSearchField,
    SofaTable,
    type SofaMenuItemSpec,
} from "@sofacoding/sofa-ui";
import {
    request,
    type ListInstance,
    type ClusterOrgList,
    type Org,
    type OrgInList,
} from "../api/client";
import { updateOrg, versionConflict, deleteOrg } from "../api/orgs";
import { fetchMetrics, fetchHealth } from "../api/operation";
import { downloadFile, exportOrg, backupFailure } from "../api/backup";
import { DialogExclusion } from "../components/DialogExclusion";
import { useGuardChanges } from "../components/GuardChanges";
import { InspectorOrg, type OrgNode } from "../components/InspectorOrg";
import { ModalOrg } from "../components/ModalOrg";
import { useSession } from "../session/Session";
import { translate, useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import { formatDate } from "../i18n/format";
import "../styles/orgs.css";

const column = createColumnHelper<OrgInList>();

type Tab = "todas" | "ativas" | "inativas" | "arquivadas";

const TABS: Tab[] = ["todas", "ativas", "inativas", "arquivadas"];

const LABEL_TAB: Record<Tab, TranslationKey> = {
    todas: "orgs.tabAll",
    ativas: "orgs.tabActive",
    inativas: "orgs.tabInactive",
    arquivadas: "orgs.tabArchived",
};

const STATUSES_TAB: Record<Tab, string[]> = {
    todas: ["active", "inactive", "archived"],
    ativas: ["active"],
    inativas: ["inactive"],
    arquivadas: ["archived"],
};

function noAccent(texto: string): string {
    return texto
        .normalize("NFD")
        .replace(/[\u0300-\u036f]/g, "")
        .toLowerCase();
}

function toneStatus(situacao: string): string {
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
            return translate("orgs.statusActive");
        case "inactive":
            return translate("orgs.statusInactive");
        case "archived":
            return translate("orgs.statusArchived");
        default:
            return situacao;
    }
}

function shortDate(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    return formatDate(ms);
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

function label(instancia: ListInstance | null | undefined): string {
    switch (instancia?.situacao) {
        case "local":
            return translate("orgs.nodeOnline");
        case "ok":
            return translate("orgs.nodeRemoteOnlineShort");
        case "inacessivel":
            return translate("orgs.nodeRemoteUnreachableShort");
        default:
            return "";
    }
}

function formatUptime(seconds: number): string {
    const total = Math.max(0, Math.floor(seconds));
    const dias = Math.floor(total / 86_400);
    const hours = Math.floor((total % 86_400) / 3_600);
    const minutes = Math.floor((total % 3_600) / 60);
    if (dias > 0) return `${dias}d ${hours}h`;
    if (hours > 0) return `${hours}h ${minutes}m`;
    return `${minutes}m`;
}

function formatMemory(bytes: number): string {
    return `${Math.round(bytes / (1024 * 1024))} MB`;
}

type Telemetry = {
    situacao: "ok" | "degradada" | "sem_resposta";
    uptimeSeconds?: number;
    memoryBytes?: number;
};

async function readTelemetry(): Promise<Telemetry> {
    const [healthRead, metricsRead] = await Promise.allSettled([fetchHealth(), fetchMetrics()]);
    return {
        situacao: healthRead.status === "fulfilled" ? healthRead.value.situacao : "sem_resposta",
        uptimeSeconds:
            healthRead.status === "fulfilled" ? healthRead.value.health.uptime_seconds : undefined,
        memoryBytes: metricsRead.status === "fulfilled" ? metricsRead.value.processMemory : undefined,
    };
}

type Notice = { tom: "warn" | "danger"; titulo: string; texto: string };

export function Orgs() {
    const { translate: t, locale } = useI18n();
    const { sessao, reloadOrgs } = useSession();
    const navigate = useNavigate();
    const { tryNavigate } = useGuardChanges();
    const [orgs, setOrgs] = useState<OrgInList[]>(sessao?.orgs ?? []);
    const [instances, setInstances] = useState<ListInstance[] | null>(null);
    const [telemetry, setTelemetry] = useState<Telemetry | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [tab, setTab] = useState<Tab>("todas");
    const [filter, setFilter] = useState("");
    const [selected, setSelectedItem] = useState<string | null>(null);
    const [formOpened, setFormOpened] = useState(false);
    const [edit, setInEdit] = useState<Org | null>(null);
    const [toDelete, setToDelete] = useState<Org | null>(null);
    const [reativando, setReativando] = useState<string | null>(null);
    const [notice, setNotice] = useState<Notice | null>(null);
    const lastClick = useRef<{ id: string; quando: number } | null>(null);
    const pendingFocus = useRef<string | null>(null);

    const canManage = sessao?.escopo !== "org";

    const mapInstances = useMemo(
        () => new Map((instances ?? []).map((instancia) => [instancia.id, instancia])),
        [instances],
    );

    const localInstance = useMemo(
        () => instances?.find((instancia) => instancia.situacao === "local") ?? null,
        [instances],
    );

    const isFromOtherNode = useCallback(
        (org: OrgInList): boolean =>
            localInstance !== null && !!org.instancia && org.instancia !== localInstance.id,
        [localInstance],
    );

    const orgNode = useCallback(
        (org: OrgInList): OrgNode | null => {
            if (instances === null) return null;
            const id = org.instancia ?? "";
            if (id === "") return { id: "", situacao: "local" };
            return { id, situacao: mapInstances.get(id)?.situacao ?? "desconhecido" };
        },
        [instances, mapInstances],
    );

    const load = useCallback(async () => {
        setLoading(true);
        setError(null);
        const telemetryReady = readTelemetry();
        try {
            const list = await request<ClusterOrgList>("/admin/v1/orgs?cluster=true");
            setOrgs(list.items);
            setInstances(list.instancias ?? null);
            await reloadOrgs();
        } catch (failure) {
            setError(failure instanceof Error ? failure.message : t("orgs.orgLoadFailed"));
        } finally {
            setLoading(false);
        }
        setTelemetry(await telemetryReady);
    }, [reloadOrgs]);

    useEffect(() => {
        void load();
    }, [load]);

    const counts = useMemo(() => {
        const byStatus = (situacao: string) => orgs.filter((org) => org.status === situacao).length;
        return {
            todas: orgs.length,
            ativas: byStatus("active"),
            inativas: byStatus("inactive"),
            arquivadas: byStatus("archived"),
        } satisfies Record<Tab, number>;
    }, [orgs]);

    const tabsVisible = useMemo(
        () => TABS.filter((candidate) => candidate !== "arquivadas" || counts.arquivadas > 0),
        [counts.arquivadas],
    );

    useEffect(() => {
        if (!tabsVisible.includes(tab)) setTab("todas");
    }, [tab, tabsVisible]);

    const visible = useMemo(() => {
        const term = noAccent(filter.trim());
        const statuses = STATUSES_TAB[tab];
        return orgs.filter((org) => {
            if (!statuses.includes(org.status)) return false;
            if (!term) return true;
            return noAccent(org.name).includes(term) || noAccent(org.slug).includes(term);
        });
    }, [orgs, tab, filter]);

    function openCreation() {
        setInEdit(null);
        setFormOpened(true);
    }

    function openEdit(org: Org) {
        setInEdit(org);
        setFormOpened(true);
    }

    function selectRow(org: OrgInList) {
        const now = Date.now();
        const previous = lastClick.current;
        lastClick.current = { id: org.id, quando: now };
        setSelectedItem(org.id);
        if (
            previous !== null &&
            previous.id === org.id &&
            now - previous.quando < 400 &&
            !isFromOtherNode(org)
        ) {
            lastClick.current = null;
            navigate(`/orgs/${org.id}`);
        }
    }

    function closeInspector() {
        pendingFocus.current = selected;
        setSelectedItem(null);
    }

    useEffect(() => {
        const id = pendingFocus.current;
        if (id === null) return;
        pendingFocus.current = null;
        const target = document.querySelector<HTMLAnchorElement>(`[data-poph-org="${id}"]`);
        target?.focus();
    }, [selected]);

    function accessOrg(org: OrgInList) {
        if (isFromOtherNode(org)) {
            setNotice({
                tom: "warn",
                titulo: t("orgs.noticeRemoteTitle"),
                texto: t("orgs.noticeRemoteText", {
                    name: org.name,
                    node: org.instancia ?? "",
                }),
            });
            return;
        }
        tryNavigate(`/orgs/${org.id}`);
    }

    async function doExport(org: OrgInList) {
        setNotice(null);
        try {
            downloadFile(await exportOrg(org.id, false));
        } catch (failure) {
            const detail = backupFailure(failure, "exportar");
            setNotice({ tom: "danger", titulo: detail.title, texto: detail.texto });
        }
    }

    async function reativar(org: OrgInList) {
        setReativando(org.id);
        setNotice(null);
        try {
            await updateOrg(org.id, org.version, { status: "active" });
            await load();
        } catch (failure) {
            const disputa = versionConflict(failure);
            if (disputa) {
                setNotice({
                    tom: "warn",
                    titulo: t("orgs.formConflictTitle"),
                    texto:
                        typeof disputa.current === "number"
                            ? t("orgs.noticeConflictWithVersion", {
                                  name: org.name,
                                  version: disputa.current,
                              })
                            : t("orgs.noticeConflictPlain", { name: org.name }),
                });
                await load();
                return;
            }
            setNotice({
                tom: "danger",
                titulo: t("orgs.noticeReactivateTitle"),
                texto:
                    failure instanceof Error
                        ? failure.message
                        : t("orgs.noticeReactivateUnexpected"),
            });
        } finally {
            setReativando(null);
        }
    }

    function itemsMenu(org: OrgInList): SofaMenuItemSpec[] {
        const remote = isFromOtherNode(org);
        const items: SofaMenuItemSpec[] = [
            {
                id: "detalhes",
                label: t("orgs.actionDetails"),
                icon: <IconEye size={14} aria-hidden="true" />,
                onSelect: () => setSelectedItem(org.id),
            },
            {
                id: "editar",
                label: t("orgs.actionEdit"),
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => openEdit(org),
            },
            {
                id: "exportar",
                label: t("orgs.actionExport"),
                icon: <IconDownload size={14} aria-hidden="true" />,
                onSelect: () => void doExport(org),
            },
        ];
        if (org.status !== "active") {
            items.push({
                id: "acessar",
                label: t("orgs.actionAccessOrg"),
                icon: <IconPlay size={14} aria-hidden="true" />,
                disabled: remote,
                onSelect: () => accessOrg(org),
            });
            items.push({
                id: "reativar",
                label: t("orgs.actionReactivate"),
                icon: <IconPlay size={14} aria-hidden="true" />,
                disabled: remote,
                onSelect: () => void reativar(org),
            });
        }
        items.push({
            id: "excluir",
            label: t("orgs.actionDelete"),
            icon: <IconTrash size={14} aria-hidden="true" />,
            danger: true,
            onSelect: () => setToDelete(org),
        });
        return items;
    }

    const columns = useMemo(
        () => [
            column.accessor("name", {
                header: t("orgs.columnName"),
                size: 210,
                cell: (info) => {
                    const org = info.row.original;
                    const atenuada = org.status !== "active";
                    const nameClass = [
                        "poph-orgs__name",
                        atenuada ? "poph-orgs__name--dimmed" : "",
                        selected === org.id ? "poph-orgs__name--selected" : "",
                    ]
                        .filter(Boolean)
                        .join(" ");
                    return (
                        <div className="poph-orgs__identity-line">
                            <span
                                className={atenuada ? "poph-orgs__tile poph-orgs__tile--dimmed" : "poph-orgs__tile"}
                                aria-hidden="true"
                            >
                                <IconBuilding size={14} aria-hidden="true" />
                            </span>
                            <div className="poph-orgs__identity">
                                {isFromOtherNode(org) ? (
                                    <span className={nameClass}>{info.getValue()}</span>
                                ) : (
                                    <Link
                                        to={`/orgs/${org.id}`}
                                        data-poph-org={org.id}
                                        className={nameClass}
                                        onClick={(evento) => evento.stopPropagation()}
                                    >
                                        {info.getValue()}
                                    </Link>
                                )}
                                <span className="poph-mono-muted">{org.slug}</span>
                            </div>
                        </div>
                    );
                },
            }),
            column.accessor("status", {
                header: t("orgs.columnStatus"),
                size: 100,
                cell: (info) => (
                    <span className={`poph-status ${toneStatus(info.getValue())}`}>
                        <span className="poph-status__point" aria-hidden="true" />
                        {labelStatus(info.getValue())}
                    </span>
                ),
            }),
            column.accessor("instancia", {
                header: t("orgs.columnNode"),
                size: 130,
                cell: (info) => {
                    if (instances === null) {
                        return (
                            <span
                                className="poph-orgs__cell--dimmed"
                                title={t("orgs.cellNodeUnknown")}
                            >
                                —
                            </span>
                        );
                    }
                    const id = info.getValue() ?? "";
                    const nodo = id === "" ? localInstance : (mapInstances.get(id) ?? null);
                    const caption = label(nodo);
                    return (
                        <div className="poph-orgs__identity">
                            <span className="poph-mono">{id || t("orgs.nodeLocal")}</span>
                            {caption !== "" && <span className="poph-mono-muted">{caption}</span>}
                        </div>
                    );
                },
            }),
            column.accessor("agents_active_count", {
                header: t("orgs.columnAgents"),
                size: 110,
                cell: (info) => {
                    const total = info.getValue();
                    if (typeof total !== "number") {
                        return (
                            <span
                                className="poph-orgs__cell--dimmed"
                                title={t("orgs.cellAgentsUnknown")}
                            >
                                —
                            </span>
                        );
                    }
                    const org = info.row.original;
                    return (
                        <span className={org.status === "active" ? undefined : "poph-orgs__cell--dimmed"}>
                            {total === 1
                                ? t("orgs.agentsCountOne")
                                : t("orgs.agentsCountMany", { count: total })}
                        </span>
                    );
                },
            }),
            column.display({
                id: "retention",
                header: t("orgs.columnRetention"),
                size: 90,
                cell: (info) => {
                    const org = info.row.original;
                    return (
                        <span className={org.status === "active" ? undefined : "poph-orgs__cell--dimmed"}>
                            {retencao(org)}
                        </span>
                    );
                },
            }),
            column.accessor("updated_at", {
                header: t("orgs.columnUpdated"),
                size: 120,
                cell: (info) => {
                    const org = info.row.original;
                    return (
                        <div className="poph-orgs__identity">
                            <span className={org.status === "active" ? undefined : "poph-orgs__cell--dimmed"}>
                                {shortDate(info.getValue())}
                            </span>
                            <span className="poph-mono-muted">v{org.version}</span>
                        </div>
                    );
                },
            }),
            column.display({
                id: "actions",
                header: t("orgs.columnActions"),
                size: 170,
                cell: (info) => {
                    const org = info.row.original;
                    const opened = selected === org.id;
                    const reactivateInRow = canManage && org.status === "inactive";
                    return (
                        <div className="poph-orgs__actions" onClick={(evento) => evento.stopPropagation()}>
                            {reactivateInRow ? (
                                <SofaButton
                                    variant="secondary"
                                    size="sm"
                                    loading={reativando === org.id}
                                    disabled={isFromOtherNode(org)}
                                    onClick={() => void reativar(org)}
                                >
                                    {t("orgs.actionReactivate")}
                                </SofaButton>
                            ) : (
                                <SofaButton
                                    variant={opened ? "primary" : "subtle"}
                                    size="sm"
                                    disabled={isFromOtherNode(org)}
                                    onClick={() => accessOrg(org)}
                                >
                                    {t("orgs.actionAccess")}
                                </SofaButton>
                            )}
                            {canManage && (
                                <SofaMenu
                                    label={t("orgs.actionMenuLabel", { name: org.name })}
                                    align="end"
                                    trigger={
                                        <SofaIconButton
                                            label={t("orgs.actionMenuLabel", { name: org.name })}
                                            size={28}
                                        >
                                            <IconMoreHorizontal size={16} aria-hidden="true" />
                                        </SofaIconButton>
                                    }
                                    items={itemsMenu(org)}
                                />
                            )}
                        </div>
                    );
                },
            }),
        ],
        [navigate, canManage, selected, reativando, instances, localInstance, mapInstances, isFromOtherNode, tryNavigate, locale],
    );

    const filtering = tab !== "todas" || filter.trim() !== "";
    const orgSelected = selected === null ? null : (orgs.find((org) => org.id === selected) ?? null);

    const telemetryParts: string[] = [];
    if (instances) {
        const connected = instances.filter((i) => i.situacao === "local" || i.situacao === "ok").length;
        const inacessiveis = instances.filter((i) => i.situacao === "inacessivel").length;
        telemetryParts.push(
            t(connected === 1 ? "orgs.telemetryNodesOne" : "orgs.telemetryNodesMany", {
                count: connected,
            }),
        );
        if (inacessiveis > 0) {
            telemetryParts.push(
                t(
                    inacessiveis === 1
                        ? "orgs.telemetryNodesOfflineOne"
                        : "orgs.telemetryNodesOfflineMany",
                    { count: inacessiveis },
                ),
            );
        }
    }
    if (telemetry?.situacao === "sem_resposta") {
        telemetryParts.push(t("orgs.telemetryNoResponse"));
    }
    if (telemetry?.situacao === "degradada") {
        telemetryParts.push(t("orgs.telemetryDegraded"));
    }
    if (telemetry?.uptimeSeconds !== undefined) {
        telemetryParts.push(
            t("orgs.telemetryUptime", { value: formatUptime(telemetry.uptimeSeconds) }),
        );
    }
    if (telemetry?.memoryBytes !== undefined) {
        telemetryParts.push(
            t("orgs.telemetryMemory", { value: formatMemory(telemetry.memoryBytes) }),
        );
    }
    const toneTelemetry =
        telemetry?.situacao === "sem_resposta"
            ? "failure"
            : telemetry?.situacao === "degradada" || (instances?.some((i) => i.situacao === "inacessivel") ?? false)
              ? "notice"
              : "ok";

    return (
        <div className={orgSelected ? "poph-orgs poph-with-drawer" : "poph-orgs"}>
            <div className="poph-page poph-orgs__panel">
                <div className="poph-page__top">
                    <div>
                        <div className="poph-orgs__title-line">
                            <h1 className="poph-page__title">{t("orgs.listTitle")}</h1>
                            <span className="poph-orgs__count">{orgs.length}</span>
                        </div>
                        <p className="poph-page__description">{t("orgs.tenantDescription")}</p>
                    </div>
                    <div className="poph-orgs__actions-top">
                        <SofaSearchField
                            label={t("orgs.listFilterLabel")}
                            value={filter}
                            onValueChange={setFilter}
                            placeholder={t("orgs.listFilterPlaceholder")}
                            size="md"
                            clearLabel={t("orgs.listFilterClear")}
                            className="poph-orgs__search"
                        />
                        <SofaIconButton
                            label={t("orgs.listRefresh")}
                            size={32}
                            loading={loading}
                            onClick={() => void load()}
                        >
                            <IconRefresh aria-hidden="true" />
                        </SofaIconButton>
                        {canManage && (
                            <SofaButton variant="primary" icon={<IconPlus aria-hidden="true" />} onClick={openCreation}>
                                {t("orgs.listNew")}
                            </SofaButton>
                        )}
                    </div>
                </div>

                {notice && (
                    <SofaBanner
                        tone={notice.tom}
                        title={notice.titulo}
                        dismissLabel={t("orgs.listDismissNotice")}
                        onDismiss={() => setNotice(null)}
                    >
                        {notice.texto}
                    </SofaBanner>
                )}

                <div className="poph-orgs__bar">
                    <div
                        className="poph-orgs__tabs"
                        role="group"
                        aria-label={t("orgs.tabsAriaLabel")}
                    >
                        {tabsVisible.map((candidate) => {
                            const active = candidate === tab;
                            return (
                                <button
                                    key={candidate}
                                    type="button"
                                    aria-pressed={active}
                                    className={active ? "poph-tab poph-tab--active" : "poph-tab"}
                                    onClick={() => setTab(candidate)}
                                >
                                    {t(LABEL_TAB[candidate])}
                                    <span className="poph-tab__count">({counts[candidate]})</span>
                                </button>
                            );
                        })}
                    </div>
                    {telemetryParts.length > 0 && (
                        <p className="poph-orgs__telemetry">
                            <span
                                className={`poph-orgs__point poph-orgs__point--${toneTelemetry}`}
                                aria-hidden="true"
                            />
                            {telemetryParts.join(" · ")}
                        </p>
                    )}
                </div>

                {error && !loading ? (
                    <SofaErrorState
                        title={t("orgs.listLoadErrorTitle")}
                        description={t("orgs.listLoadErrorDescription")}
                        details={error}
                        retry={{ label: t("orgs.failureTryAgain"), onClick: () => void load() }}
                    />
                ) : loading && orgs.length === 0 ? (
                    <div
                        className="poph-orgs__skeleton"
                        aria-busy="true"
                        aria-label={t("orgs.listLoading")}
                    >
                        {[0, 1, 2, 3, 4].map((row) => (
                            <div key={row} className="poph-orgs__skeleton-line" />
                        ))}
                    </div>
                ) : (
                    <SofaTable
                        className="poph-orgs__table"
                        label={t("orgs.listTitle")}
                        columns={columns}
                        data={visible}
                        getRowId={(org) => org.id}
                        enableSelection
                        selectedIds={selected === null ? [] : [selected]}
                        onRowClick={selectRow}
                        isLoading={loading}
                        totalCount={visible.length}
                        footerMeta={() =>
                            orgs.length === 1
                                ? t("orgs.listFooterOne", {
                                      shown: visible.length,
                                      total: orgs.length,
                                  })
                                : t("orgs.listFooterMany", {
                                      shown: visible.length,
                                      total: orgs.length,
                                  })
                        }
                        footer={
                            <span className="poph-orgs__hint">{t("orgs.listKeyboardHint")}</span>
                        }
                        empty={
                            filtering ? (
                                <SofaEmptyState
                                    title={t("orgs.listEmptyFilteredTitle")}
                                    description={t("orgs.listEmptyFilteredDescription")}
                                    action={
                                        <SofaButton
                                            variant="secondary"
                                            onClick={() => {
                                                setTab("todas");
                                                setFilter("");
                                            }}
                                        >
                                            {t("orgs.listClearFilters")}
                                        </SofaButton>
                                    }
                                />
                            ) : (
                                <SofaEmptyState
                                    icon={<IconBuilding aria-hidden="true" />}
                                    title={t("orgs.listEmptyTitle")}
                                    description={
                                        canManage
                                            ? t("orgs.listEmptyManagerDescription")
                                            : t("orgs.listEmptyOrgTokenDescription")
                                    }
                                    action={
                                        canManage ? (
                                            <SofaButton
                                                variant="primary"
                                                icon={<IconPlus aria-hidden="true" />}
                                                onClick={openCreation}
                                            >
                                                {t("orgs.listCreateFirst")}
                                            </SofaButton>
                                        ) : undefined
                                    }
                                />
                            )
                        }
                    />
                )}

                <div className="poph-orgs__cards">
                    <section className="poph-orgs__card" aria-labelledby="poph-card-matrix">
                        <div className="poph-orgs__card-head">
                            <h2 id="poph-card-matrix" className="poph-orgs__card-title">
                                {t("orgs.cardMatrixTitle")}
                            </h2>
                            <span
                                className="poph-orgs__seal"
                                title={t("orgs.cardMatrixBadgeTitle")}
                            >
                                {t("orgs.inspectorUnavailable")}
                            </span>
                        </div>
                        <p className="poph-orgs__card-text">{t("orgs.cardMatrixText")}</p>
                    </section>
                    <section className="poph-orgs__card" aria-labelledby="poph-card-isolamento">
                        <div className="poph-orgs__card-head">
                            <h2 id="poph-card-isolamento" className="poph-orgs__card-title">
                                {t("orgs.cardTenantTitle")}
                            </h2>
                        </div>
                        <p className="poph-orgs__card-text">{t("orgs.cardTenantText")}</p>
                    </section>
                    <section className="poph-orgs__card" aria-labelledby="poph-card-cli">
                        <div className="poph-orgs__card-head">
                            <h2 id="poph-card-cli" className="poph-orgs__card-title">
                                {t("orgs.cardCliTitle")}
                            </h2>
                            <span className="poph-orgs__seal">poph</span>
                        </div>
                        <ul className="poph-orgs__commands">
                            <li>
                                <code>poph org list</code>
                            </li>
                            <li>
                                <code>poph org create --name &quot;nome&quot;</code>
                            </li>
                            <li>
                                <code>poph doctor</code>
                            </li>
                        </ul>
                    </section>
                </div>

                <ModalOrg
                    opened={formOpened}
                    org={edit}
                    onClose={() => setFormOpened(false)}
                    onSave={() => {
                        void load();
                    }}
                />

                {toDelete && (
                    <DialogExclusion
                        opened={toDelete !== null}
                        onClose={() => setToDelete(null)}
                        title={t("orgs.deleteOrgTitle")}
                        descricao={t("orgs.deleteOrgDescription", { name: toDelete.name })}
                        resource={toDelete.slug}
                        labelResource={t("orgs.deleteOrgResourceLabel")}
                        labelConfirm={t("orgs.deleteOrgConfirm")}
                        allowForce
                        labelForce={t("orgs.deleteOrgForceLabel")}
                        consequencia={
                            <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                                {t("orgs.deleteOrgConsequence")}
                            </p>
                        }
                        onConfirm={(forcar) => deleteOrg(toDelete.id, forcar)}
                        onDelete={() => {
                            setToDelete(null);
                            void load();
                        }}
                    />
                )}
            </div>

            {orgSelected && (
                <InspectorOrg
                    org={orgSelected}
                    no={orgNode(orgSelected)}
                    canManage={canManage}
                    remote={isFromOtherNode(orgSelected)}
                    onClose={closeInspector}
                    onAccess={accessOrg}
                    onEdit={openEdit}
                    onExport={(org) => void doExport(org)}
                    onDelete={(org) => setToDelete(org)}
                />
            )}
        </div>
    );
}

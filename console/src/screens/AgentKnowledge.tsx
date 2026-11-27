
import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconDatabase,
    IconFolder,
    IconLink,
    IconMoreHorizontal,
    IconPencil,
    IconPlus,
    IconRefresh,
    IconTrash,
    SofaBanner,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaIconButton,
    SofaMenu,
    SofaSkeleton,
    type SofaMenuItemSpec,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { listAgents, getAgent, type Agente } from "../api/agents";
import {
    unlinkGroup,
    deleteItem,
    listGroups,
    listItems,
    getGroup,
    reindexAgent,
    linkGroup,
    type KnowledgeGroup,
    type KnowledgeItem,
    type IndexReport,
} from "../api/knowledge";
import { getOrg } from "../api/orgs";
import { TabsAgent, HeaderAgent } from "../components/TabsAgent";
import { DialogDeleteGroup } from "../components/DialogDeleteGroup";
import { DialogExclusion } from "../components/DialogExclusion";
import { useGuardChanges } from "../components/GuardChanges";
import { ModalGroup } from "../components/ModalGroup";
import { ModalItem } from "../components/ModalItem";
import { translate, useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import "../styles/knowledge.css";

type StateIndex =
    | { tipo: "nao-verificado" }
    | { tipo: "atualizado"; origin: TranslationKey; report?: IndexReport }
    | { tipo: "pendente"; mensagem: string };

function classStatus(situacao: string): string {
    return situacao === "active" ? "poph-status--active" : "poph-status--inactive";
}

function labelStatus(situacao: string): string {
    switch (situacao) {
        case "active":
            return translate("knowledge.statusActive");
        case "inactive":
            return translate("knowledge.statusInactive");
        case "archived":
            return translate("knowledge.statusArchived");
        default:
            return situacao;
    }
}

function countLabel(count: number, oneKey: TranslationKey, manyKey: TranslationKey): string {
    return translate(count === 1 ? oneKey : manyKey, { count });
}

function labelIndex(index: StateIndex): string {
    switch (index.tipo) {
        case "atualizado":
            return translate("knowledge.indexUpdated");
        case "pendente":
            return translate("knowledge.indexPending");
        default:
            return translate("knowledge.indexUnverified");
    }
}

function classIndex(index: StateIndex): string {
    switch (index.tipo) {
        case "atualizado":
            return "poph-status--active";
        case "pendente":
            return "poph-knowledge__index-pending";
        default:
            return "poph-status--inactive";
    }
}

function summaryReport(report: IndexReport): string {
    return [
        countLabel(report.itens_novos, "knowledge.reindexNewOne", "knowledge.reindexNewMany"),
        countLabel(
            report.itens_alterados,
            "knowledge.reindexChangedOne",
            "knowledge.reindexChangedMany",
        ),
        countLabel(
            report.itens_inalterados,
            "knowledge.reindexUnchangedOne",
            "knowledge.reindexUnchangedMany",
        ),
        countLabel(
            report.itens_removidos,
            "knowledge.reindexRemovedOne",
            "knowledge.reindexRemovedMany",
        ),
        countLabel(
            report.embeddings_gerados,
            "knowledge.reindexEmbeddingsOne",
            "knowledge.reindexEmbeddingsMany",
        ),
        countLabel(
            report.trechos_gravados,
            "knowledge.reindexChunksOne",
            "knowledge.reindexChunksMany",
        ),
    ].join(" · ");
}

function noticeIndex(response: { index_warning?: string | null }): string | null {
    return typeof response.index_warning === "string" && response.index_warning
        ? response.index_warning
        : null;
}

export function AgentKnowledge() {
    const { translate: t } = useI18n();
    const { orgId = "", agentId = "" } = useParams();
    const guard = useGuardChanges();

    const [agente, setAgent] = useState<Agente | null>(null);
    const [nameOrg, setOrgName] = useState<string | null>(null);
    const [loading, setLoading] = useState(true);
    const [errorLoad, setLoadError] = useState<string | null>(null);

    const [groups, setGroups] = useState<KnowledgeGroup[]>([]);
    const [namesAgents, setAgentNames] = useState<Record<string, string>>({});
    const [loadingGroups, setLoadingGroups] = useState(true);
    const [errorGroups, setGroupsError] = useState<string | null>(null);

    const [selected, setSelected] = useState<string | null>(null);
    const [items, setItems] = useState<KnowledgeItem[]>([]);
    const [loadingItems, setLoadingItems] = useState(false);
    const [errorItems, setItemsError] = useState<string | null>(null);

    const [index, setIndex] = useState<StateIndex>({ tipo: "nao-verificado" });
    const [reindexing, setReindexing] = useState(false);
    const [reindexError, setReindexError] = useState<string | null>(null);

    const [link, setLinking] = useState<string | null>(null);
    const [errorLink, setLinkError] = useState<string | null>(null);

    const [groupModal, setGroupModal] = useState<{ opened: boolean; group: KnowledgeGroup | null }>({
        opened: false,
        group: null,
    });
    const [groupDelete, setGroupToDelete] = useState<KnowledgeGroup | null>(null);

    const [modalItem, setModalItem] = useState<{ opened: boolean; item: KnowledgeItem | null }>({
        opened: false,
        item: null,
    });
    const [itemDelete, setItemToDelete] = useState<KnowledgeItem | null>(null);
    const [errorItem, setItemError] = useState<string | null>(null);
    const [removingItem, setRemovingItem] = useState(false);

    const groupOpen = useMemo(
        () => groups.find((group) => group.id === selected) ?? null,
        [groups, selected],
    );

    const loadGroups = useCallback(async () => {
        setLoadingGroups(true);
        setGroupsError(null);
        try {
            const list = await listGroups(orgId);
            const withAgents = await Promise.all(
                list.map(async (group) => {
                    try {
                        return await getGroup(orgId, group.id);
                    } catch {
                        return group;
                    }
                }),
            );
            setGroups(withAgents);
            setSelected((current) => {
                if (current && withAgents.some((group) => group.id === current)) return current;
                const linked = withAgents.find((group) => group.agents?.includes(agentId));
                return linked?.id ?? withAgents[0]?.id ?? null;
            });
        } catch (failure) {
            setGroupsError(
                failure instanceof Error ? failure.message : t("knowledge.groupsLoadFailed"),
            );
            setGroups([]);
        } finally {
            setLoadingGroups(false);
        }
    }, [orgId, agentId, t]);

    const loadItems = useCallback(
        async (group: string | null) => {
            if (!group) {
                setItems([]);
                setItemsError(null);
                return;
            }
            setLoadingItems(true);
            setItemsError(null);
            try {
                setItems(await listItems(orgId, group));
            } catch (failure) {
                setItemsError(
                    failure instanceof Error ? failure.message : t("knowledge.itemsLoadFailed"),
                );
                setItems([]);
            } finally {
                setLoadingItems(false);
            }
        },
        [orgId, t],
    );

    const load = useCallback(async () => {
        setLoading(true);
        setLoadError(null);
        try {
            const dados = await getAgent(orgId, agentId);
            setAgent(dados);
        } catch (failure) {
            setLoadError(
                failure instanceof Error ? failure.message : t("knowledge.agentLoadFailed"),
            );
            setLoading(false);
            return;
        }
        setLoading(false);
        void loadGroups();
    }, [orgId, agentId, loadGroups, t]);

    useEffect(() => {
        void load();
        getOrg(orgId)
            .then((org) => setOrgName(org.name))
            .catch(() => setOrgName(null));
        listAgents(orgId)
            .then((list) =>
                setAgentNames(
                    Object.fromEntries(list.map((candidate) => [candidate.id, candidate.name])),
                ),
            )
            .catch(() => setAgentNames({}));
    }, [load, orgId]);

    useEffect(() => {
        void loadItems(selected);
    }, [loadItems, selected]);

    function registerIndex(notice: string | null, source: TranslationKey) {
        setIndex(
            notice
                ? { tipo: "pendente", mensagem: notice }
                : { tipo: "atualizado", origin: source },
        );
    }

    async function toggleLink(group: KnowledgeGroup) {
        const linked = group.agents?.includes(agentId) ?? false;
        setLinking(group.id);
        setLinkError(null);
        try {
            if (linked) {
                await unlinkGroup(orgId, agentId, group.id);
            } else {
                await linkGroup(orgId, agentId, group.id);
            }
            await loadGroups();
            setIndex({ tipo: "atualizado", origin: "knowledge.indexOriginLink" });
        } catch (failure) {
            setLinkError(
                failure instanceof Error ? failure.message : t("knowledge.linkFailed"),
            );
        } finally {
            setLinking(null);
        }
    }

    async function reindexar() {
        setReindexing(true);
        setReindexError(null);
        try {
            const resultado = await reindexAgent(orgId, agentId);
            setIndex({
                tipo: "atualizado",
                origin: "knowledge.indexOriginReindexed",
                report: resultado.indexed,
            });
        } catch (failure) {
            if (failure instanceof ApiError && failure.code === "embedding_unavailable") {
                setReindexError(
                    t("knowledge.embeddingUnavailable", { message: failure.message }),
                );
            } else {
                setReindexError(
                    failure instanceof Error ? failure.message : t("knowledge.reindexFailed"),
                );
            }
        } finally {
            setReindexing(false);
        }
    }

    function onSaveGroup(group: KnowledgeGroup) {
        const previous = groups.find((candidate) => candidate.id === group.id);
        const linked = (group.agents ?? previous?.agents)?.includes(agentId) ?? false;
        setGroups((current) => {
            const exists = current.some((candidate) => candidate.id === group.id);
            return exists
                ? current.map((candidate) => (candidate.id === group.id ? { ...candidate, ...group } : candidate))
                : [...current, group];
        });
        if (!selected) setSelected(group.id);
        if (linked) {
            registerIndex(noticeIndex(group), "knowledge.indexOriginGroupSaved");
        }
        void loadGroups();
    }

    function onSaveItem(item: KnowledgeItem) {
        setItems((current) => {
            const exists = current.some((candidate) => candidate.id === item.id);
            return exists
                ? current.map((candidate) => (candidate.id === item.id ? item : candidate))
                : [...current, item];
        });
        if (groupOpen?.agents?.includes(agentId)) {
            registerIndex(noticeIndex(item), "knowledge.indexOriginItemSaved");
        }
        if (selected) void loadItems(selected);
    }

    async function deleteExcerpt() {
        if (!itemDelete) return;
        const groupItem = itemDelete.group_id;
        setRemovingItem(true);
        setItemError(null);
        try {
            await deleteItem(orgId, groupItem, itemDelete.id);
            setItems((current) => current.filter((item) => item.id !== itemDelete.id));
            setItemToDelete(null);
            if (groupOpen?.agents?.includes(agentId)) {
                setIndex({
                    tipo: "pendente",
                    mensagem: t("knowledge.indexItemRemoved"),
                });
            }
        } catch (failure) {
            setItemError(failure instanceof Error ? failure.message : t("knowledge.itemDeleteFailed"));
        } finally {
            setRemovingItem(false);
        }
    }

    if (loading && !agente) {
        return (
            <div className="poph-page" aria-busy="true">
                <div className="poph-page__top">
                    <div>
                        <SofaSkeleton width={260} height={26} />
                        <div className="poph-knowledge__skeleton-line">
                            <SofaSkeleton width={360} height={12} />
                        </div>
                    </div>
                </div>
                <SofaSkeleton lines={6} />
            </div>
        );
    }

    if (errorLoad && !agente) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("knowledge.loadErrorTitle")}
                    description={t("knowledge.loadErrorDescription")}
                    details={errorLoad}
                    retry={{ label: t("knowledge.retry"), onClick: () => void load() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guard.tryNavigate(`/orgs/${orgId}/agents`)}
                        >
                            {t("knowledge.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente) return null;

    const linkedAgent = (group: KnowledgeGroup) =>
        group.agents?.includes(agentId) ?? false;

    function itemsMenuGroup(group: KnowledgeGroup): SofaMenuItemSpec[] {
        const linked = linkedAgent(group);
        return [
            {
                id: "editar",
                label: t("knowledge.edit"),
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => setGroupModal({ opened: true, group: group }),
            },
            {
                id: "vinculo",
                label: linked ? t("knowledge.unlink") : t("knowledge.link"),
                icon: <IconLink size={14} aria-hidden="true" />,
                disabled: link === group.id,
                onSelect: () => void toggleLink(group),
            },
            {
                id: "excluir",
                label: t("knowledge.remove"),
                icon: <IconTrash size={14} aria-hidden="true" />,
                danger: true,
                onSelect: () => setGroupToDelete(group),
            },
        ];
    }

    function itemsMenuExcerpt(excerpt: KnowledgeItem): SofaMenuItemSpec[] {
        return [
            {
                id: "editar",
                label: t("knowledge.edit"),
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => setModalItem({ opened: true, item: excerpt }),
            },
            {
                id: "excluir",
                label: t("knowledge.remove"),
                icon: <IconTrash size={14} aria-hidden="true" />,
                danger: true,
                onSelect: () => setItemToDelete(excerpt),
            },
        ];
    }

    return (
        <>
            <div className="poph-page poph-knowledge">
                <HeaderAgent
                    org={orgId}
                    nome={agente.name}
                    slug={agente.slug}
                    situacao={agente.status}
                    organization={nameOrg}
                    descricao={t("knowledge.headerDescription")}
                    actions={
                        <>
                            <span className={`poph-status ${classIndex(index)}`}>
                                <span className="poph-status__point" aria-hidden="true" />
                                {labelIndex(index)}
                            </span>
                            <SofaButton
                                variant="secondary"
                                icon={<IconRefresh aria-hidden="true" />}
                                loading={reindexing}
                                onClick={() => void reindexar()}
                            >
                                {t("knowledge.reindexAgent")}
                            </SofaButton>
                        </>
                    }
                />

                <TabsAgent org={orgId} agente={agentId} active="knowledge-base" nome={agente.name} model={agente.model ?? null} />

                {errorLink && (
                    <SofaBanner tone="danger" title={t("knowledge.linkErrorTitle")}>
                        {errorLink}
                    </SofaBanner>
                )}

                <section className="poph-section">
                    <div className="poph-knowledge__title-block">
                        <h2 className="poph-section__title">{t("knowledge.indexTitle")}</h2>
                        <p className="poph-knowledge__help">
                            {t("knowledge.indexHelp")}
                        </p>
                    </div>

                    {index.tipo === "nao-verificado" && (
                        <p className="poph-knowledge__text">
                            {t("knowledge.indexUnverifiedBefore")}
                            <code>{t("knowledge.indexCommand")}</code>
                            {t("knowledge.indexUnverifiedAfter")}
                        </p>
                    )}

                    {index.tipo === "atualizado" && (
                        <p className="poph-knowledge__text">
                            {t(index.origin)}.{" "}
                            {index.report
                                ? summaryReport(index.report)
                                : t("knowledge.syncNoWarning")}
                        </p>
                    )}

                    {index.tipo === "pendente" && (
                        <SofaBanner tone="warn" title={t("knowledge.indexPendingTitle")}>
                            {t("knowledge.indexPendingText", { message: index.mensagem })}
                        </SofaBanner>
                    )}

                    {reindexError && (
                        <SofaBanner tone="danger" title={t("knowledge.reindexErrorTitle")}>
                            {reindexError}
                        </SofaBanner>
                    )}

                    <p className="poph-mono-muted poph-knowledge__note">
                        {t("knowledge.reindexNote")}
                    </p>
                </section>

                <section className="poph-section">
                    <div className="poph-knowledge__header-section">
                        <div className="poph-knowledge__title-block">
                            <h2 className="poph-section__title">{t("knowledge.groupsTitle")}</h2>
                            <p className="poph-knowledge__help">
                                {t("knowledge.groupsHelp")}
                            </p>
                        </div>
                        <div className="poph-knowledge__actions-section">
                            <SofaButton
                                variant="subtle"
                                icon={<IconRefresh aria-hidden="true" />}
                                onClick={() => void loadGroups()}
                                loading={loadingGroups && groups.length > 0}
                            >
                                {t("knowledge.refresh")}
                            </SofaButton>
                            <SofaButton
                                variant="primary"
                                icon={<IconPlus aria-hidden="true" />}
                                onClick={() => setGroupModal({ opened: true, group: null })}
                            >
                                {t("knowledge.newGroup")}
                            </SofaButton>
                        </div>
                    </div>

                    {loadingGroups && groups.length === 0 ? (
                        <SofaSkeleton lines={5} />
                    ) : errorGroups ? (
                        <SofaErrorState
                            title={t("knowledge.groupsErrorTitle")}
                            description={t("knowledge.groupsErrorDescription")}
                            details={errorGroups}
                            retry={{
                                label: t("knowledge.retry"),
                                onClick: () => void loadGroups(),
                            }}
                        />
                    ) : groups.length === 0 ? (
                        <SofaEmptyState
                            icon={<IconFolder aria-hidden="true" />}
                            title={t("knowledge.groupsEmptyTitle")}
                            description={t("knowledge.groupsEmptyDescription")}
                            action={
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => setGroupModal({ opened: true, group: null })}
                                >
                                    {t("knowledge.createFirstGroup")}
                                </SofaButton>
                            }
                        />
                    ) : (
                        <div
                            className="poph-table poph-knowledge__groups"
                            role="table"
                            aria-label={t("knowledge.groupsTableLabel")}
                        >
                            <div className="poph-table__header" role="row">
                                <span role="columnheader">{t("knowledge.columnGroup")}</span>
                                <span role="columnheader">{t("knowledge.columnLink")}</span>
                                <span role="columnheader">{t("knowledge.columnStatus")}</span>
                                <span role="columnheader">{t("knowledge.columnVersion")}</span>
                                <span role="columnheader" className="poph-table__cell--actions">
                                    {t("knowledge.columnActions")}
                                </span>
                            </div>
                            {groups.map((group) => {
                                const linked = linkedAgent(group);
                                const opened = group.id === selected;
                                const rowClassName = [
                                    "poph-table__line",
                                    opened ? "poph-table__line--selected" : "",
                                    group.status !== "active" ? "poph-table__line--dimmed" : "",
                                ]
                                    .filter(Boolean)
                                    .join(" ");
                                return (
                                    <div
                                        key={group.id}
                                        role="row"
                                        className={rowClassName}
                                        data-selected={opened ? "true" : undefined}
                                        onClick={() => setSelected(group.id)}
                                    >
                                        <div
                                            role="cell"
                                            className="poph-knowledge__identity"
                                        >
                                            <button
                                                type="button"
                                                className="poph-knowledge__name"
                                                aria-pressed={opened}
                                                onClick={(evento) => {
                                                    evento.stopPropagation();
                                                    setSelected(group.id);
                                                }}
                                            >
                                                {group.name}
                                            </button>
                                            {group.description && (
                                                <span className="poph-knowledge__sub">
                                                    {group.description}
                                                </span>
                                            )}
                                        </div>
                                        <div role="cell">
                                            <span
                                                className={
                                                    linked
                                                        ? "poph-knowledge__link"
                                                        : "poph-knowledge__link poph-knowledge__link--weak"
                                                }
                                            >
                                                {linked ? t("knowledge.linkedToAgent") : t("knowledge.noLink")}
                                            </span>
                                        </div>
                                        <div role="cell">
                                            <span className={`poph-status ${classStatus(group.status)}`}>
                                                <span className="poph-status__point" aria-hidden="true" />
                                                {labelStatus(group.status)}
                                            </span>
                                        </div>
                                        <div role="cell" className="poph-table__cell--mono">
                                            v{group.version}
                                        </div>
                                        <div
                                            role="cell"
                                            className="poph-table__cell--actions"
                                            onClick={(evento) => evento.stopPropagation()}
                                        >
                                            <SofaMenu
                                                label={t("knowledge.actionsOf", { name: group.name })}
                                                align="end"
                                                trigger={
                                                    <SofaIconButton
                                                        label={t("knowledge.actionsOf", { name: group.name })}
                                                        size={28}
                                                        loading={link === group.id}
                                                    >
                                                        <IconMoreHorizontal
                                                            size={16}
                                                            aria-hidden="true"
                                                        />
                                                    </SofaIconButton>
                                                }
                                                items={itemsMenuGroup(group)}
                                            />
                                        </div>
                                    </div>
                                );
                            })}
                        </div>
                    )}
                </section>

                <section className="poph-section">
                    <div className="poph-knowledge__header-section">
                        <div className="poph-knowledge__title-block">
                            <h2 className="poph-section__title">
                                {groupOpen
                                    ? t("knowledge.itemsTitleWithGroup", { group: groupOpen.name })
                                    : t("knowledge.itemsTitle")}
                            </h2>
                            <p className="poph-knowledge__help">
                                {groupOpen
                                    ? t("knowledge.itemsHelp")
                                    : t("knowledge.itemsHelpNoGroup")}
                            </p>
                        </div>
                        {groupOpen && (
                            <div className="poph-knowledge__actions-section">
                                <span
                                    className="poph-knowledge__count"
                                    aria-label={t("knowledge.itemsCountLabel")}
                                >
                                    {loadingItems
                                        ? "…"
                                        : t(
                                              items.length === 1
                                                  ? "knowledge.itemsCountOne"
                                                  : "knowledge.itemsCountMany",
                                              { count: items.length },
                                          )}
                                </span>
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => setModalItem({ opened: true, item: null })}
                                >
                                    {t("knowledge.newItem")}
                                </SofaButton>
                            </div>
                        )}
                    </div>

                    {!groupOpen ? (
                        <SofaEmptyState
                            icon={<IconDatabase aria-hidden="true" />}
                            title={t("knowledge.itemsNoGroupTitle")}
                            description={t("knowledge.itemsNoGroupDescription")}
                        />
                    ) : loadingItems && items.length === 0 ? (
                        <SofaSkeleton lines={5} />
                    ) : errorItems ? (
                        <SofaErrorState
                            title={t("knowledge.itemsErrorTitle")}
                            description={t("knowledge.itemsErrorDescription")}
                            details={errorItems}
                            retry={{
                                label: t("knowledge.retry"),
                                onClick: () => void loadItems(selected),
                            }}
                        />
                    ) : items.length === 0 ? (
                        <SofaEmptyState
                            icon={<IconDatabase aria-hidden="true" />}
                            title={t("knowledge.itemsEmptyTitle")}
                            description={t("knowledge.itemsEmptyDescription")}
                            action={
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => setModalItem({ opened: true, item: null })}
                                >
                                    {t("knowledge.createFirstItem")}
                                </SofaButton>
                            }
                        />
                    ) : (
                        <div
                            className="poph-table poph-knowledge__passages"
                            role="table"
                            aria-label={t("knowledge.itemsTableLabel")}
                        >
                            <div className="poph-table__header" role="row">
                                <span role="columnheader">{t("knowledge.columnItem")}</span>
                                <span role="columnheader">{t("knowledge.columnStatus")}</span>
                                <span role="columnheader">{t("knowledge.columnDetails")}</span>
                                <span role="columnheader" className="poph-table__cell--actions">
                                    {t("knowledge.columnActions")}
                                </span>
                            </div>
                            {items.map((excerpt) => (
                                <div
                                    key={excerpt.id}
                                    role="row"
                                    className={
                                        excerpt.status === "active"
                                            ? "poph-table__line"
                                            : "poph-table__line poph-table__line--dimmed"
                                    }
                                >
                                    <div role="cell" className="poph-knowledge__identity">
                                        <span className="poph-knowledge__name-static">
                                            {excerpt.title}
                                        </span>
                                        <span className="poph-knowledge__sub">
                                            {excerpt.content.length > 160
                                                ? `${excerpt.content.slice(0, 160)}…`
                                                : excerpt.content}
                                        </span>
                                    </div>
                                    <div role="cell">
                                        <span className={`poph-status ${classStatus(excerpt.status)}`}>
                                            <span className="poph-status__point" aria-hidden="true" />
                                            {labelStatus(excerpt.status)}
                                        </span>
                                    </div>
                                    <div role="cell" className="poph-knowledge__details">
                                        <span>
                                            {t("knowledge.itemDetails", {
                                                count: excerpt.content.length,
                                                position: excerpt.position,
                                                version: excerpt.version,
                                            })}
                                        </span>
                                        {excerpt.tags.length > 0 && (
                                            <span className="poph-knowledge__tags">
                                                {excerpt.tags.join(" · ")}
                                            </span>
                                        )}
                                    </div>
                                    <div role="cell" className="poph-table__cell--actions">
                                        <SofaMenu
                                            label={t("knowledge.actionsOf", { name: excerpt.title })}
                                            align="end"
                                            trigger={
                                                <SofaIconButton
                                                    label={t("knowledge.actionsOf", { name: excerpt.title })}
                                                    size={28}
                                                >
                                                    <IconMoreHorizontal
                                                        size={16}
                                                        aria-hidden="true"
                                                    />
                                                </SofaIconButton>
                                            }
                                            items={itemsMenuExcerpt(excerpt)}
                                        />
                                    </div>
                                </div>
                            ))}
                        </div>
                    )}

                    {errorItem && (
                        <SofaBanner tone="danger" title={t("knowledge.itemDeleteErrorTitle")}>
                            {errorItem}
                        </SofaBanner>
                    )}
                </section>
            </div>

            <ModalGroup
                org={orgId}
                opened={groupModal.opened}
                group={groupModal.group}
                onClose={() => setGroupModal({ opened: false, group: null })}
                onSave={onSaveGroup}
            />

            {groupOpen && (
                <ModalItem
                    org={orgId}
                    group={groupOpen.id}
                    opened={modalItem.opened}
                    item={modalItem.item}
                    onClose={() => setModalItem({ opened: false, item: null })}
                    onSave={onSaveItem}
                />
            )}

            <DialogDeleteGroup
                opened={groupDelete !== null}
                onClose={() => setGroupToDelete(null)}
                org={orgId}
                group={groupDelete}
                namesAgents={namesAgents}
                onDelete={() => {
                    const removed = groupDelete?.id;
                    const wasAgent = groupDelete?.agents?.includes(agentId) ?? false;
                    setGroupToDelete(null);
                    if (removed) {
                        setGroups((current) => current.filter((group) => group.id !== removed));
                        if (selected === removed) setSelected(null);
                    }
                    if (wasAgent) {
                        setIndex({
                            tipo: "pendente",
                            mensagem: t("knowledge.indexGroupRemoved"),
                        });
                    }
                    void loadGroups();
                }}
            />

            <DialogExclusion
                opened={itemDelete !== null}
                onClose={() => {
                    if (!removingItem) setItemToDelete(null);
                }}
                title={t("knowledge.deleteItemTitle")}
                descricao={t("knowledge.deleteItemDescription", {
                    title: itemDelete?.title ?? "",
                })}
                resource={itemDelete?.title ?? ""}
                labelResource={t("knowledge.deleteItemResourceLabel")}
                labelConfirm={t("knowledge.deleteItemConfirm")}
                onConfirm={deleteExcerpt}
            />
        </>
    );
}

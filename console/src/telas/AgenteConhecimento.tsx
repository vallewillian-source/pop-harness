/**
 * Aba Base de conhecimento do agente (UX_SPEC §3.6, HR-41).
 *
 * Duas seções planas, uma embaixo da outra (RL-11: sem cartão): os **grupos** da organização com
 * o vínculo com este agente e, abaixo, os **trechos** do grupo selecionado. O painel do índice
 * (o que dá para saber pela API) fica no topo, com a reindexação no cabeçalho da página e o
 * detalhe do estado na primeira seção.
 *
 * O que a API expõe e o que não expõe (importa para não inventar estado):
 * - grupos, itens, vínculo e reindexação existem em `/admin/v1`;
 * - **não existe rota de estado do índice**. O único sinal é `index_warning` nas respostas de
 *   escrita (sincronização falhou) e o relatório do `reindex` (que é síncrono). Antes de qualquer
 *   escrita ou reindex, o cabeçalho diz "Índice não verificado" e a seção aponta `poph kb status`,
 *   que lê o índice offline.
 *
 * Escritas: `If-Match` nas alterações (412 relê e mantém o rascunho, como nas outras telas) e
 * exclusão com confirmação por nome — o grupo leva itens e vínculos junto.
 *
 * Cada linha tem o menu "⋯" (padrão da RL-05): o grupo oferece Editar, Vincular/Desvincular e
 * Excluir; o trecho, Editar e Excluir. Selecionar a linha abre os trechos daquele grupo.
 */

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

type EstadoDoIndice =
    | { tipo: "nao-verificado" }
    | { tipo: "atualizado"; origem: TranslationKey; relatorio?: IndexReport }
    | { tipo: "pendente"; mensagem: string };

/** Classe do ponto de status (6 px + texto), no padrão de Orgs e OrgPainel. */
function classeDaSituacao(situacao: string): string {
    return situacao === "active" ? "poph-status--active" : "poph-status--inactive";
}

function rotuloDaSituacao(situacao: string): string {
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

/** Estado do índice na linguagem do cabeçalho (ponto + texto, sem pílula). */
function rotuloDoIndice(indice: EstadoDoIndice): string {
    switch (indice.tipo) {
        case "atualizado":
            return translate("knowledge.indexUpdated");
        case "pendente":
            return translate("knowledge.indexPending");
        default:
            return translate("knowledge.indexUnverified");
    }
}

function classeDoIndice(indice: EstadoDoIndice): string {
    switch (indice.tipo) {
        case "atualizado":
            return "poph-status--active";
        case "pendente":
            return "poph-knowledge__index-pending";
        default:
            return "poph-status--inactive";
    }
}

/** Resumo do relatório do reindex em uma linha. */
function resumoDoRelatorio(relatorio: IndexReport): string {
    return [
        countLabel(relatorio.itens_novos, "knowledge.reindexNewOne", "knowledge.reindexNewMany"),
        countLabel(
            relatorio.itens_alterados,
            "knowledge.reindexChangedOne",
            "knowledge.reindexChangedMany",
        ),
        countLabel(
            relatorio.itens_inalterados,
            "knowledge.reindexUnchangedOne",
            "knowledge.reindexUnchangedMany",
        ),
        countLabel(
            relatorio.itens_removidos,
            "knowledge.reindexRemovedOne",
            "knowledge.reindexRemovedMany",
        ),
        countLabel(
            relatorio.embeddings_gerados,
            "knowledge.reindexEmbeddingsOne",
            "knowledge.reindexEmbeddingsMany",
        ),
        countLabel(
            relatorio.trechos_gravados,
            "knowledge.reindexChunksOne",
            "knowledge.reindexChunksMany",
        ),
    ].join(" · ");
}

/** Lê o aviso de índice de uma resposta de escrita. */
function avisoDeIndice(resposta: { index_warning?: string | null }): string | null {
    return typeof resposta.index_warning === "string" && resposta.index_warning
        ? resposta.index_warning
        : null;
}

export function AgentKnowledge() {
    const { translate: t } = useI18n();
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardChanges();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [carregando, setCarregando] = useState(true);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [grupos, setGrupos] = useState<KnowledgeGroup[]>([]);
    const [nomesDosAgentes, setNomesDosAgentes] = useState<Record<string, string>>({});
    const [carregandoGrupos, setCarregandoGrupos] = useState(true);
    const [erroGrupos, setErroGrupos] = useState<string | null>(null);

    const [selecionado, setSelecionado] = useState<string | null>(null);
    const [itens, setItens] = useState<KnowledgeItem[]>([]);
    const [carregandoItens, setCarregandoItens] = useState(false);
    const [erroItens, setErroItens] = useState<string | null>(null);

    const [indice, setIndice] = useState<EstadoDoIndice>({ tipo: "nao-verificado" });
    const [reindexando, setReindexando] = useState(false);
    const [erroReindex, setErroReindex] = useState<string | null>(null);

    const [emVinculo, setEmVinculo] = useState<string | null>(null);
    const [erroDeVinculo, setErroDeVinculo] = useState<string | null>(null);

    const [modalGrupo, setModalGrupo] = useState<{ aberto: boolean; grupo: KnowledgeGroup | null }>({
        aberto: false,
        grupo: null,
    });
    const [grupoParaExcluir, setGrupoParaExcluir] = useState<KnowledgeGroup | null>(null);

    const [modalItem, setModalItem] = useState<{ aberto: boolean; item: KnowledgeItem | null }>({
        aberto: false,
        item: null,
    });
    const [itemParaExcluir, setItemParaExcluir] = useState<KnowledgeItem | null>(null);
    const [erroDeItem, setErroDeItem] = useState<string | null>(null);
    const [removendoItem, setRemovendoItem] = useState(false);

    const grupoAberto = useMemo(
        () => grupos.find((grupo) => grupo.id === selecionado) ?? null,
        [grupos, selecionado],
    );

    /** Relê os grupos e o vínculo de cada um com este agente (o detalhe traz os agentes). */
    const carregarGrupos = useCallback(async () => {
        setCarregandoGrupos(true);
        setErroGrupos(null);
        try {
            const lista = await listGroups(orgId);
            // O vínculo só aparece no detalhe do grupo; uma falha isolada não derruba a lista.
            const comAgentes = await Promise.all(
                lista.map(async (grupo) => {
                    try {
                        return await getGroup(orgId, grupo.id);
                    } catch {
                        return grupo;
                    }
                }),
            );
            setGrupos(comAgentes);
            setSelecionado((atual) => {
                if (atual && comAgentes.some((grupo) => grupo.id === atual)) return atual;
                const vinculado = comAgentes.find((grupo) => grupo.agents?.includes(agentId));
                return vinculado?.id ?? comAgentes[0]?.id ?? null;
            });
        } catch (falha) {
            setErroGrupos(
                falha instanceof Error ? falha.message : t("knowledge.groupsLoadFailed"),
            );
            setGrupos([]);
        } finally {
            setCarregandoGrupos(false);
        }
    }, [orgId, agentId, t]);

    const carregarItens = useCallback(
        async (grupo: string | null) => {
            if (!grupo) {
                setItens([]);
                setErroItens(null);
                return;
            }
            setCarregandoItens(true);
            setErroItens(null);
            try {
                setItens(await listItems(orgId, grupo));
            } catch (falha) {
                setErroItens(
                    falha instanceof Error ? falha.message : t("knowledge.itemsLoadFailed"),
                );
                setItens([]);
            } finally {
                setCarregandoItens(false);
            }
        },
        [orgId, t],
    );

    const carregar = useCallback(async () => {
        setCarregando(true);
        setErroDeCarga(null);
        try {
            const dados = await getAgent(orgId, agentId);
            setAgente(dados);
        } catch (falha) {
            setErroDeCarga(
                falha instanceof Error ? falha.message : t("knowledge.agentLoadFailed"),
            );
            setCarregando(false);
            return;
        }
        setCarregando(false);
        void carregarGrupos();
    }, [orgId, agentId, carregarGrupos, t]);

    useEffect(() => {
        void carregar();
        getOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
        listAgents(orgId)
            .then((lista) =>
                setNomesDosAgentes(
                    Object.fromEntries(lista.map((candidato) => [candidato.id, candidato.name])),
                ),
            )
            .catch(() => setNomesDosAgentes({}));
    }, [carregar, orgId]);

    useEffect(() => {
        void carregarItens(selecionado);
    }, [carregarItens, selecionado]);

    /** Depois de uma escrita: atualiza o estado do índice com o que o servidor disse. */
    function registrarIndice(aviso: string | null, origem: TranslationKey) {
        setIndice(
            aviso
                ? { tipo: "pendente", mensagem: aviso }
                : { tipo: "atualizado", origem },
        );
    }

    async function alternarVinculo(grupo: KnowledgeGroup) {
        const vinculado = grupo.agents?.includes(agentId) ?? false;
        setEmVinculo(grupo.id);
        setErroDeVinculo(null);
        try {
            if (vinculado) {
                await unlinkGroup(orgId, agentId, grupo.id);
            } else {
                await linkGroup(orgId, agentId, grupo.id);
            }
            await carregarGrupos();
            // O vínculo sincroniza o índice do agente; relê o detalhe para saber se atrasou.
            setIndice({ tipo: "atualizado", origem: "knowledge.indexOriginLink" });
        } catch (falha) {
            setErroDeVinculo(
                falha instanceof Error ? falha.message : t("knowledge.linkFailed"),
            );
        } finally {
            setEmVinculo(null);
        }
    }

    async function reindexar() {
        setReindexando(true);
        setErroReindex(null);
        try {
            const resultado = await reindexAgent(orgId, agentId);
            setIndice({
                tipo: "atualizado",
                origem: "knowledge.indexOriginReindexed",
                relatorio: resultado.indexed,
            });
        } catch (falha) {
            if (falha instanceof ApiError && falha.codigo === "embedding_unavailable") {
                setErroReindex(
                    t("knowledge.embeddingUnavailable", { message: falha.message }),
                );
            } else {
                setErroReindex(
                    falha instanceof Error ? falha.message : t("knowledge.reindexFailed"),
                );
            }
        } finally {
            setReindexando(false);
        }
    }

    function aoSalvarGrupo(grupo: KnowledgeGroup) {
        const anterior = grupos.find((candidato) => candidato.id === grupo.id);
        const vinculado = (grupo.agents ?? anterior?.agents)?.includes(agentId) ?? false;
        setGrupos((atuais) => {
            const existe = atuais.some((candidato) => candidato.id === grupo.id);
            return existe
                ? atuais.map((candidato) => (candidato.id === grupo.id ? { ...candidato, ...grupo } : candidato))
                : [...atuais, grupo];
        });
        if (!selecionado) setSelecionado(grupo.id);
        // Só há índice a sincronizar se este agente usa o grupo; sem vínculo, nada muda.
        // A resposta do `PATCH` não traz `agents`, então o vínculo vem do estado anterior.
        if (vinculado) {
            registrarIndice(avisoDeIndice(grupo), "knowledge.indexOriginGroupSaved");
        }
        void carregarGrupos();
    }

    function aoSalvarItem(item: KnowledgeItem) {
        setItens((atuais) => {
            const existe = atuais.some((candidato) => candidato.id === item.id);
            return existe
                ? atuais.map((candidato) => (candidato.id === item.id ? item : candidato))
                : [...atuais, item];
        });
        // Trecho de grupo não vinculado a este agente não entra no índice dele.
        if (grupoAberto?.agents?.includes(agentId)) {
            registrarIndice(avisoDeIndice(item), "knowledge.indexOriginItemSaved");
        }
        if (selecionado) void carregarItens(selecionado);
    }

    async function excluirTrecho() {
        if (!itemParaExcluir) return;
        const grupoDoItem = itemParaExcluir.group_id;
        setRemovendoItem(true);
        setErroDeItem(null);
        try {
            await deleteItem(orgId, grupoDoItem, itemParaExcluir.id);
            setItens((atuais) => atuais.filter((item) => item.id !== itemParaExcluir.id));
            setItemParaExcluir(null);
            if (grupoAberto?.agents?.includes(agentId)) {
                setIndice({
                    tipo: "pendente",
                    mensagem: t("knowledge.indexItemRemoved"),
                });
            }
        } catch (falha) {
            setErroDeItem(falha instanceof Error ? falha.message : t("knowledge.itemDeleteFailed"));
        } finally {
            setRemovendoItem(false);
        }
    }

    if (carregando && !agente) {
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

    if (erroDeCarga && !agente) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("knowledge.loadErrorTitle")}
                    description={t("knowledge.loadErrorDescription")}
                    details={erroDeCarga}
                    retry={{ label: t("knowledge.retry"), onClick: () => void carregar() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guarda.tentarNavegar(`/orgs/${orgId}/agents`)}
                        >
                            {t("knowledge.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente) return null;

    const vinculadoAoAgente = (grupo: KnowledgeGroup) =>
        grupo.agents?.includes(agentId) ?? false;

    function itensDoMenuDoGrupo(grupo: KnowledgeGroup): SofaMenuItemSpec[] {
        const vinculado = vinculadoAoAgente(grupo);
        return [
            {
                id: "editar",
                label: t("knowledge.edit"),
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => setModalGrupo({ aberto: true, grupo }),
            },
            {
                id: "vinculo",
                label: vinculado ? t("knowledge.unlink") : t("knowledge.link"),
                icon: <IconLink size={14} aria-hidden="true" />,
                disabled: emVinculo === grupo.id,
                onSelect: () => void alternarVinculo(grupo),
            },
            {
                id: "excluir",
                label: t("knowledge.remove"),
                icon: <IconTrash size={14} aria-hidden="true" />,
                danger: true,
                onSelect: () => setGrupoParaExcluir(grupo),
            },
        ];
    }

    function itensDoMenuDoTrecho(trecho: KnowledgeItem): SofaMenuItemSpec[] {
        return [
            {
                id: "editar",
                label: t("knowledge.edit"),
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => setModalItem({ aberto: true, item: trecho }),
            },
            {
                id: "excluir",
                label: t("knowledge.remove"),
                icon: <IconTrash size={14} aria-hidden="true" />,
                danger: true,
                onSelect: () => setItemParaExcluir(trecho),
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
                    organizacao={nomeDaOrg}
                    descricao={t("knowledge.headerDescription")}
                    acoes={
                        <>
                            <span className={`poph-status ${classeDoIndice(indice)}`}>
                                <span className="poph-status__point" aria-hidden="true" />
                                {rotuloDoIndice(indice)}
                            </span>
                            <SofaButton
                                variant="secondary"
                                icon={<IconRefresh aria-hidden="true" />}
                                loading={reindexando}
                                onClick={() => void reindexar()}
                            >
                                {t("knowledge.reindexAgent")}
                            </SofaButton>
                        </>
                    }
                />

                <TabsAgent org={orgId} agente={agentId} active="knowledge-base" nome={agente.name} model={agente.model ?? null} />

                {erroDeVinculo && (
                    <SofaBanner tone="danger" title={t("knowledge.linkErrorTitle")}>
                        {erroDeVinculo}
                    </SofaBanner>
                )}

                <section className="poph-section">
                    <div className="poph-knowledge__title-block">
                        <h2 className="poph-section__title">{t("knowledge.indexTitle")}</h2>
                        <p className="poph-knowledge__help">
                            {t("knowledge.indexHelp")}
                        </p>
                    </div>

                    {indice.tipo === "nao-verificado" && (
                        <p className="poph-knowledge__text">
                            {t("knowledge.indexUnverifiedBefore")}
                            <code>{t("knowledge.indexCommand")}</code>
                            {t("knowledge.indexUnverifiedAfter")}
                        </p>
                    )}

                    {indice.tipo === "atualizado" && (
                        <p className="poph-knowledge__text">
                            {t(indice.origem)}.{" "}
                            {indice.relatorio
                                ? resumoDoRelatorio(indice.relatorio)
                                : t("knowledge.syncNoWarning")}
                        </p>
                    )}

                    {indice.tipo === "pendente" && (
                        <SofaBanner tone="warn" title={t("knowledge.indexPendingTitle")}>
                            {t("knowledge.indexPendingText", { message: indice.mensagem })}
                        </SofaBanner>
                    )}

                    {erroReindex && (
                        <SofaBanner tone="danger" title={t("knowledge.reindexErrorTitle")}>
                            {erroReindex}
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
                                onClick={() => void carregarGrupos()}
                                loading={carregandoGrupos && grupos.length > 0}
                            >
                                {t("knowledge.refresh")}
                            </SofaButton>
                            <SofaButton
                                variant="primary"
                                icon={<IconPlus aria-hidden="true" />}
                                onClick={() => setModalGrupo({ aberto: true, grupo: null })}
                            >
                                {t("knowledge.newGroup")}
                            </SofaButton>
                        </div>
                    </div>

                    {carregandoGrupos && grupos.length === 0 ? (
                        <SofaSkeleton lines={5} />
                    ) : erroGrupos ? (
                        <SofaErrorState
                            title={t("knowledge.groupsErrorTitle")}
                            description={t("knowledge.groupsErrorDescription")}
                            details={erroGrupos}
                            retry={{
                                label: t("knowledge.retry"),
                                onClick: () => void carregarGrupos(),
                            }}
                        />
                    ) : grupos.length === 0 ? (
                        <SofaEmptyState
                            icon={<IconFolder aria-hidden="true" />}
                            title={t("knowledge.groupsEmptyTitle")}
                            description={t("knowledge.groupsEmptyDescription")}
                            action={
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => setModalGrupo({ aberto: true, grupo: null })}
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
                            {grupos.map((grupo) => {
                                const vinculado = vinculadoAoAgente(grupo);
                                const aberto = grupo.id === selecionado;
                                const classes = [
                                    "poph-table__line",
                                    aberto ? "poph-table__line--selected" : "",
                                    grupo.status !== "active" ? "poph-table__line--dimmed" : "",
                                ]
                                    .filter(Boolean)
                                    .join(" ");
                                return (
                                    <div
                                        key={grupo.id}
                                        role="row"
                                        className={classes}
                                        data-selecionado={aberto ? "true" : undefined}
                                        onClick={() => setSelecionado(grupo.id)}
                                    >
                                        <div
                                            role="cell"
                                            className="poph-knowledge__identity"
                                        >
                                            <button
                                                type="button"
                                                className="poph-knowledge__name"
                                                aria-pressed={aberto}
                                                onClick={(evento) => {
                                                    evento.stopPropagation();
                                                    setSelecionado(grupo.id);
                                                }}
                                            >
                                                {grupo.name}
                                            </button>
                                            {grupo.description && (
                                                <span className="poph-knowledge__sub">
                                                    {grupo.description}
                                                </span>
                                            )}
                                        </div>
                                        <div role="cell">
                                            <span
                                                className={
                                                    vinculado
                                                        ? "poph-knowledge__link"
                                                        : "poph-knowledge__link poph-knowledge__link--weak"
                                                }
                                            >
                                                {vinculado ? t("knowledge.linkedToAgent") : t("knowledge.noLink")}
                                            </span>
                                        </div>
                                        <div role="cell">
                                            <span className={`poph-status ${classeDaSituacao(grupo.status)}`}>
                                                <span className="poph-status__point" aria-hidden="true" />
                                                {rotuloDaSituacao(grupo.status)}
                                            </span>
                                        </div>
                                        <div role="cell" className="poph-table__cell--mono">
                                            v{grupo.version}
                                        </div>
                                        <div
                                            role="cell"
                                            className="poph-table__cell--actions"
                                            onClick={(evento) => evento.stopPropagation()}
                                        >
                                            <SofaMenu
                                                label={t("knowledge.actionsOf", { name: grupo.name })}
                                                align="end"
                                                trigger={
                                                    <SofaIconButton
                                                        label={t("knowledge.actionsOf", { name: grupo.name })}
                                                        size={28}
                                                        loading={emVinculo === grupo.id}
                                                    >
                                                        <IconMoreHorizontal
                                                            size={16}
                                                            aria-hidden="true"
                                                        />
                                                    </SofaIconButton>
                                                }
                                                items={itensDoMenuDoGrupo(grupo)}
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
                                {grupoAberto
                                    ? t("knowledge.itemsTitleWithGroup", { group: grupoAberto.name })
                                    : t("knowledge.itemsTitle")}
                            </h2>
                            <p className="poph-knowledge__help">
                                {grupoAberto
                                    ? t("knowledge.itemsHelp")
                                    : t("knowledge.itemsHelpNoGroup")}
                            </p>
                        </div>
                        {grupoAberto && (
                            <div className="poph-knowledge__actions-section">
                                <span
                                    className="poph-knowledge__count"
                                    aria-label={t("knowledge.itemsCountLabel")}
                                >
                                    {carregandoItens
                                        ? "…"
                                        : t(
                                              itens.length === 1
                                                  ? "knowledge.itemsCountOne"
                                                  : "knowledge.itemsCountMany",
                                              { count: itens.length },
                                          )}
                                </span>
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => setModalItem({ aberto: true, item: null })}
                                >
                                    {t("knowledge.newItem")}
                                </SofaButton>
                            </div>
                        )}
                    </div>

                    {!grupoAberto ? (
                        <SofaEmptyState
                            icon={<IconDatabase aria-hidden="true" />}
                            title={t("knowledge.itemsNoGroupTitle")}
                            description={t("knowledge.itemsNoGroupDescription")}
                        />
                    ) : carregandoItens && itens.length === 0 ? (
                        <SofaSkeleton lines={5} />
                    ) : erroItens ? (
                        <SofaErrorState
                            title={t("knowledge.itemsErrorTitle")}
                            description={t("knowledge.itemsErrorDescription")}
                            details={erroItens}
                            retry={{
                                label: t("knowledge.retry"),
                                onClick: () => void carregarItens(selecionado),
                            }}
                        />
                    ) : itens.length === 0 ? (
                        <SofaEmptyState
                            icon={<IconDatabase aria-hidden="true" />}
                            title={t("knowledge.itemsEmptyTitle")}
                            description={t("knowledge.itemsEmptyDescription")}
                            action={
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => setModalItem({ aberto: true, item: null })}
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
                            {itens.map((trecho) => (
                                <div
                                    key={trecho.id}
                                    role="row"
                                    className={
                                        trecho.status === "active"
                                            ? "poph-table__line"
                                            : "poph-table__line poph-table__line--dimmed"
                                    }
                                >
                                    <div role="cell" className="poph-knowledge__identity">
                                        <span className="poph-knowledge__name-static">
                                            {trecho.title}
                                        </span>
                                        <span className="poph-knowledge__sub">
                                            {trecho.content.length > 160
                                                ? `${trecho.content.slice(0, 160)}…`
                                                : trecho.content}
                                        </span>
                                    </div>
                                    <div role="cell">
                                        <span className={`poph-status ${classeDaSituacao(trecho.status)}`}>
                                            <span className="poph-status__point" aria-hidden="true" />
                                            {rotuloDaSituacao(trecho.status)}
                                        </span>
                                    </div>
                                    <div role="cell" className="poph-knowledge__details">
                                        <span>
                                            {t("knowledge.itemDetails", {
                                                count: trecho.content.length,
                                                position: trecho.position,
                                                version: trecho.version,
                                            })}
                                        </span>
                                        {trecho.tags.length > 0 && (
                                            <span className="poph-knowledge__tags">
                                                {trecho.tags.join(" · ")}
                                            </span>
                                        )}
                                    </div>
                                    <div role="cell" className="poph-table__cell--actions">
                                        <SofaMenu
                                            label={t("knowledge.actionsOf", { name: trecho.title })}
                                            align="end"
                                            trigger={
                                                <SofaIconButton
                                                    label={t("knowledge.actionsOf", { name: trecho.title })}
                                                    size={28}
                                                >
                                                    <IconMoreHorizontal
                                                        size={16}
                                                        aria-hidden="true"
                                                    />
                                                </SofaIconButton>
                                            }
                                            items={itensDoMenuDoTrecho(trecho)}
                                        />
                                    </div>
                                </div>
                            ))}
                        </div>
                    )}

                    {erroDeItem && (
                        <SofaBanner tone="danger" title={t("knowledge.itemDeleteErrorTitle")}>
                            {erroDeItem}
                        </SofaBanner>
                    )}
                </section>
            </div>

            <ModalGroup
                org={orgId}
                aberto={modalGrupo.aberto}
                grupo={modalGrupo.grupo}
                onClose={() => setModalGrupo({ aberto: false, grupo: null })}
                onSave={aoSalvarGrupo}
            />

            {grupoAberto && (
                <ModalItem
                    org={orgId}
                    grupo={grupoAberto.id}
                    aberto={modalItem.aberto}
                    item={modalItem.item}
                    onClose={() => setModalItem({ aberto: false, item: null })}
                    onSave={aoSalvarItem}
                />
            )}

            <DialogDeleteGroup
                aberto={grupoParaExcluir !== null}
                onClose={() => setGrupoParaExcluir(null)}
                org={orgId}
                grupo={grupoParaExcluir}
                namesAgents={nomesDosAgentes}
                aoExcluir={() => {
                    const removido = grupoParaExcluir?.id;
                    const eraDoAgente = grupoParaExcluir?.agents?.includes(agentId) ?? false;
                    setGrupoParaExcluir(null);
                    if (removido) {
                        setGrupos((atuais) => atuais.filter((grupo) => grupo.id !== removido));
                        if (selecionado === removido) setSelecionado(null);
                    }
                    if (eraDoAgente) {
                        setIndice({
                            tipo: "pendente",
                            mensagem: t("knowledge.indexGroupRemoved"),
                        });
                    }
                    void carregarGrupos();
                }}
            />

            <DialogExclusion
                aberto={itemParaExcluir !== null}
                onClose={() => {
                    if (!removendoItem) setItemParaExcluir(null);
                }}
                titulo={t("knowledge.deleteItemTitle")}
                descricao={t("knowledge.deleteItemDescription", {
                    title: itemParaExcluir?.title ?? "",
                })}
                resource={itemParaExcluir?.title ?? ""}
                labelResource={t("knowledge.deleteItemResourceLabel")}
                labelConfirm={t("knowledge.deleteItemConfirm")}
                aoConfirmar={excluirTrecho}
            />
        </>
    );
}

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
import { ErroApi } from "../api/cliente";
import { listarAgentes, obterAgente, type Agente } from "../api/agentes";
import {
    desvincularGrupo,
    excluirItem,
    listarGrupos,
    listarItens,
    obterGrupo,
    reindexarAgente,
    vincularGrupo,
    type GrupoConhecimento,
    type ItemConhecimento,
    type RelatorioIndexacao,
} from "../api/conhecimento";
import { obterOrg } from "../api/orgs";
import { AbasDoAgente, CabecalhoDoAgente } from "../componentes/AbasDoAgente";
import { DialogoExcluirGrupo } from "../componentes/DialogoExcluirGrupo";
import { DialogoExclusao } from "../componentes/DialogoExclusao";
import { useGuardaDeAlteracoes } from "../componentes/GuardaAlteracoes";
import { ModalGrupo } from "../componentes/ModalGrupo";
import { ModalItem } from "../componentes/ModalItem";
import { translate, useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import "../estilos/conhecimento.css";

type EstadoDoIndice =
    | { tipo: "nao-verificado" }
    | { tipo: "atualizado"; origem: TranslationKey; relatorio?: RelatorioIndexacao }
    | { tipo: "pendente"; mensagem: string };

/** Classe do ponto de status (6 px + texto), no padrão de Orgs e OrgPainel. */
function classeDaSituacao(situacao: string): string {
    return situacao === "active" ? "poph-status--ativo" : "poph-status--inativo";
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
            return "poph-status--ativo";
        case "pendente":
            return "poph-conhecimento__indice-pendente";
        default:
            return "poph-status--inativo";
    }
}

/** Resumo do relatório do reindex em uma linha. */
function resumoDoRelatorio(relatorio: RelatorioIndexacao): string {
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

export function AgenteConhecimento() {
    const { translate: t } = useI18n();
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardaDeAlteracoes();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [carregando, setCarregando] = useState(true);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [grupos, setGrupos] = useState<GrupoConhecimento[]>([]);
    const [nomesDosAgentes, setNomesDosAgentes] = useState<Record<string, string>>({});
    const [carregandoGrupos, setCarregandoGrupos] = useState(true);
    const [erroGrupos, setErroGrupos] = useState<string | null>(null);

    const [selecionado, setSelecionado] = useState<string | null>(null);
    const [itens, setItens] = useState<ItemConhecimento[]>([]);
    const [carregandoItens, setCarregandoItens] = useState(false);
    const [erroItens, setErroItens] = useState<string | null>(null);

    const [indice, setIndice] = useState<EstadoDoIndice>({ tipo: "nao-verificado" });
    const [reindexando, setReindexando] = useState(false);
    const [erroReindex, setErroReindex] = useState<string | null>(null);

    const [emVinculo, setEmVinculo] = useState<string | null>(null);
    const [erroDeVinculo, setErroDeVinculo] = useState<string | null>(null);

    const [modalGrupo, setModalGrupo] = useState<{ aberto: boolean; grupo: GrupoConhecimento | null }>({
        aberto: false,
        grupo: null,
    });
    const [grupoParaExcluir, setGrupoParaExcluir] = useState<GrupoConhecimento | null>(null);

    const [modalItem, setModalItem] = useState<{ aberto: boolean; item: ItemConhecimento | null }>({
        aberto: false,
        item: null,
    });
    const [itemParaExcluir, setItemParaExcluir] = useState<ItemConhecimento | null>(null);
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
            const lista = await listarGrupos(orgId);
            // O vínculo só aparece no detalhe do grupo; uma falha isolada não derruba a lista.
            const comAgentes = await Promise.all(
                lista.map(async (grupo) => {
                    try {
                        return await obterGrupo(orgId, grupo.id);
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
                setItens(await listarItens(orgId, grupo));
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
            const dados = await obterAgente(orgId, agentId);
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
        obterOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
        listarAgentes(orgId)
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

    async function alternarVinculo(grupo: GrupoConhecimento) {
        const vinculado = grupo.agents?.includes(agentId) ?? false;
        setEmVinculo(grupo.id);
        setErroDeVinculo(null);
        try {
            if (vinculado) {
                await desvincularGrupo(orgId, agentId, grupo.id);
            } else {
                await vincularGrupo(orgId, agentId, grupo.id);
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
            const resultado = await reindexarAgente(orgId, agentId);
            setIndice({
                tipo: "atualizado",
                origem: "knowledge.indexOriginReindexed",
                relatorio: resultado.indexed,
            });
        } catch (falha) {
            if (falha instanceof ErroApi && falha.codigo === "embedding_unavailable") {
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

    function aoSalvarGrupo(grupo: GrupoConhecimento) {
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

    function aoSalvarItem(item: ItemConhecimento) {
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
            await excluirItem(orgId, grupoDoItem, itemParaExcluir.id);
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
            <div className="poph-pagina" aria-busy="true">
                <div className="poph-pagina__topo">
                    <div>
                        <SofaSkeleton width={260} height={26} />
                        <div className="poph-conhecimento__esqueleto-linha">
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
            <div className="poph-pagina">
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

    const vinculadoAoAgente = (grupo: GrupoConhecimento) =>
        grupo.agents?.includes(agentId) ?? false;

    function itensDoMenuDoGrupo(grupo: GrupoConhecimento): SofaMenuItemSpec[] {
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

    function itensDoMenuDoTrecho(trecho: ItemConhecimento): SofaMenuItemSpec[] {
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
            <div className="poph-pagina poph-conhecimento">
                <CabecalhoDoAgente
                    org={orgId}
                    nome={agente.name}
                    slug={agente.slug}
                    situacao={agente.status}
                    organizacao={nomeDaOrg}
                    descricao={t("knowledge.headerDescription")}
                    acoes={
                        <>
                            <span className={`poph-status ${classeDoIndice(indice)}`}>
                                <span className="poph-status__ponto" aria-hidden="true" />
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

                <AbasDoAgente org={orgId} agente={agentId} ativa="knowledge-base" nome={agente.name} model={agente.model ?? null} />

                {erroDeVinculo && (
                    <SofaBanner tone="danger" title={t("knowledge.linkErrorTitle")}>
                        {erroDeVinculo}
                    </SofaBanner>
                )}

                <section className="poph-secao">
                    <div className="poph-conhecimento__titulo-bloco">
                        <h2 className="poph-secao__titulo">{t("knowledge.indexTitle")}</h2>
                        <p className="poph-conhecimento__ajuda">
                            {t("knowledge.indexHelp")}
                        </p>
                    </div>

                    {indice.tipo === "nao-verificado" && (
                        <p className="poph-conhecimento__texto">
                            {t("knowledge.indexUnverifiedBefore")}
                            <code>{t("knowledge.indexCommand")}</code>
                            {t("knowledge.indexUnverifiedAfter")}
                        </p>
                    )}

                    {indice.tipo === "atualizado" && (
                        <p className="poph-conhecimento__texto">
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

                    <p className="poph-mono-mudo poph-conhecimento__nota">
                        {t("knowledge.reindexNote")}
                    </p>
                </section>

                <section className="poph-secao">
                    <div className="poph-conhecimento__cabecalho-secao">
                        <div className="poph-conhecimento__titulo-bloco">
                            <h2 className="poph-secao__titulo">{t("knowledge.groupsTitle")}</h2>
                            <p className="poph-conhecimento__ajuda">
                                {t("knowledge.groupsHelp")}
                            </p>
                        </div>
                        <div className="poph-conhecimento__acoes-secao">
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
                            className="poph-tabela poph-conhecimento__grupos"
                            role="table"
                            aria-label={t("knowledge.groupsTableLabel")}
                        >
                            <div className="poph-tabela__cabecalho" role="row">
                                <span role="columnheader">{t("knowledge.columnGroup")}</span>
                                <span role="columnheader">{t("knowledge.columnLink")}</span>
                                <span role="columnheader">{t("knowledge.columnStatus")}</span>
                                <span role="columnheader">{t("knowledge.columnVersion")}</span>
                                <span role="columnheader" className="poph-tabela__celula--acoes">
                                    {t("knowledge.columnActions")}
                                </span>
                            </div>
                            {grupos.map((grupo) => {
                                const vinculado = vinculadoAoAgente(grupo);
                                const aberto = grupo.id === selecionado;
                                const classes = [
                                    "poph-tabela__linha",
                                    aberto ? "poph-tabela__linha--selecionada" : "",
                                    grupo.status !== "active" ? "poph-tabela__linha--atenuada" : "",
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
                                            className="poph-conhecimento__identidade"
                                        >
                                            <button
                                                type="button"
                                                className="poph-conhecimento__nome"
                                                aria-pressed={aberto}
                                                onClick={(evento) => {
                                                    evento.stopPropagation();
                                                    setSelecionado(grupo.id);
                                                }}
                                            >
                                                {grupo.name}
                                            </button>
                                            {grupo.description && (
                                                <span className="poph-conhecimento__sub">
                                                    {grupo.description}
                                                </span>
                                            )}
                                        </div>
                                        <div role="cell">
                                            <span
                                                className={
                                                    vinculado
                                                        ? "poph-conhecimento__vinculo"
                                                        : "poph-conhecimento__vinculo poph-conhecimento__vinculo--fraco"
                                                }
                                            >
                                                {vinculado ? t("knowledge.linkedToAgent") : t("knowledge.noLink")}
                                            </span>
                                        </div>
                                        <div role="cell">
                                            <span className={`poph-status ${classeDaSituacao(grupo.status)}`}>
                                                <span className="poph-status__ponto" aria-hidden="true" />
                                                {rotuloDaSituacao(grupo.status)}
                                            </span>
                                        </div>
                                        <div role="cell" className="poph-tabela__celula--mono">
                                            v{grupo.version}
                                        </div>
                                        <div
                                            role="cell"
                                            className="poph-tabela__celula--acoes"
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

                <section className="poph-secao">
                    <div className="poph-conhecimento__cabecalho-secao">
                        <div className="poph-conhecimento__titulo-bloco">
                            <h2 className="poph-secao__titulo">
                                {grupoAberto
                                    ? t("knowledge.itemsTitleWithGroup", { group: grupoAberto.name })
                                    : t("knowledge.itemsTitle")}
                            </h2>
                            <p className="poph-conhecimento__ajuda">
                                {grupoAberto
                                    ? t("knowledge.itemsHelp")
                                    : t("knowledge.itemsHelpNoGroup")}
                            </p>
                        </div>
                        {grupoAberto && (
                            <div className="poph-conhecimento__acoes-secao">
                                <span
                                    className="poph-conhecimento__contagem"
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
                            className="poph-tabela poph-conhecimento__trechos"
                            role="table"
                            aria-label={t("knowledge.itemsTableLabel")}
                        >
                            <div className="poph-tabela__cabecalho" role="row">
                                <span role="columnheader">{t("knowledge.columnItem")}</span>
                                <span role="columnheader">{t("knowledge.columnStatus")}</span>
                                <span role="columnheader">{t("knowledge.columnDetails")}</span>
                                <span role="columnheader" className="poph-tabela__celula--acoes">
                                    {t("knowledge.columnActions")}
                                </span>
                            </div>
                            {itens.map((trecho) => (
                                <div
                                    key={trecho.id}
                                    role="row"
                                    className={
                                        trecho.status === "active"
                                            ? "poph-tabela__linha"
                                            : "poph-tabela__linha poph-tabela__linha--atenuada"
                                    }
                                >
                                    <div role="cell" className="poph-conhecimento__identidade">
                                        <span className="poph-conhecimento__nome-estatico">
                                            {trecho.title}
                                        </span>
                                        <span className="poph-conhecimento__sub">
                                            {trecho.content.length > 160
                                                ? `${trecho.content.slice(0, 160)}…`
                                                : trecho.content}
                                        </span>
                                    </div>
                                    <div role="cell">
                                        <span className={`poph-status ${classeDaSituacao(trecho.status)}`}>
                                            <span className="poph-status__ponto" aria-hidden="true" />
                                            {rotuloDaSituacao(trecho.status)}
                                        </span>
                                    </div>
                                    <div role="cell" className="poph-conhecimento__detalhes">
                                        <span>
                                            {t("knowledge.itemDetails", {
                                                count: trecho.content.length,
                                                position: trecho.position,
                                                version: trecho.version,
                                            })}
                                        </span>
                                        {trecho.tags.length > 0 && (
                                            <span className="poph-conhecimento__tags">
                                                {trecho.tags.join(" · ")}
                                            </span>
                                        )}
                                    </div>
                                    <div role="cell" className="poph-tabela__celula--acoes">
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

            <ModalGrupo
                org={orgId}
                aberto={modalGrupo.aberto}
                grupo={modalGrupo.grupo}
                aoFechar={() => setModalGrupo({ aberto: false, grupo: null })}
                aoSalvar={aoSalvarGrupo}
            />

            {grupoAberto && (
                <ModalItem
                    org={orgId}
                    grupo={grupoAberto.id}
                    aberto={modalItem.aberto}
                    item={modalItem.item}
                    aoFechar={() => setModalItem({ aberto: false, item: null })}
                    aoSalvar={aoSalvarItem}
                />
            )}

            <DialogoExcluirGrupo
                aberto={grupoParaExcluir !== null}
                aoFechar={() => setGrupoParaExcluir(null)}
                org={orgId}
                grupo={grupoParaExcluir}
                nomesDosAgentes={nomesDosAgentes}
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

            <DialogoExclusao
                aberto={itemParaExcluir !== null}
                aoFechar={() => {
                    if (!removendoItem) setItemParaExcluir(null);
                }}
                titulo={t("knowledge.deleteItemTitle")}
                descricao={t("knowledge.deleteItemDescription", {
                    title: itemParaExcluir?.title ?? "",
                })}
                recurso={itemParaExcluir?.title ?? ""}
                rotuloRecurso={t("knowledge.deleteItemResourceLabel")}
                rotuloConfirmar={t("knowledge.deleteItemConfirm")}
                aoConfirmar={excluirTrecho}
            />
        </>
    );
}

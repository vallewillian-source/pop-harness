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
import "../estilos/conhecimento.css";

type EstadoDoIndice =
    | { tipo: "nao-verificado" }
    | { tipo: "atualizado"; origem: string; relatorio?: RelatorioIndexacao }
    | { tipo: "pendente"; mensagem: string };

/** Classe do ponto de status (6 px + texto), no padrão de Orgs e OrgPainel. */
function classeDaSituacao(situacao: string): string {
    return situacao === "active" ? "poph-status--ativo" : "poph-status--inativo";
}

function rotuloDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return "Ativo";
        case "inactive":
            return "Inativo";
        case "archived":
            return "Arquivado";
        default:
            return situacao;
    }
}

function plural(quantidade: number, singular: string, plurala: string): string {
    return `${quantidade} ${quantidade === 1 ? singular : plurala}`;
}

/** Estado do índice na linguagem do cabeçalho (ponto + texto, sem pílula). */
function rotuloDoIndice(indice: EstadoDoIndice): string {
    switch (indice.tipo) {
        case "atualizado":
            return "Índice atualizado";
        case "pendente":
            return "Reindexação pendente";
        default:
            return "Índice não verificado";
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
        plural(relatorio.itens_novos, "novo", "novos"),
        `${relatorio.itens_alterados} alterado${relatorio.itens_alterados === 1 ? "" : "s"}`,
        `${relatorio.itens_inalterados} inalterado${relatorio.itens_inalterados === 1 ? "" : "s"}`,
        `${relatorio.itens_removidos} removido${relatorio.itens_removidos === 1 ? "" : "s"}`,
        plural(relatorio.embeddings_gerados, "embedding gerado", "embeddings gerados"),
        plural(relatorio.trechos_gravados, "trecho indexado", "trechos indexados"),
    ].join(" · ");
}

/** Lê o aviso de índice de uma resposta de escrita. */
function avisoDeIndice(resposta: { index_warning?: string | null }): string | null {
    return typeof resposta.index_warning === "string" && resposta.index_warning
        ? resposta.index_warning
        : null;
}

export function AgenteConhecimento() {
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
                falha instanceof Error ? falha.message : "falha ao carregar os grupos da base",
            );
            setGrupos([]);
        } finally {
            setCarregandoGrupos(false);
        }
    }, [orgId, agentId]);

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
                    falha instanceof Error ? falha.message : "falha ao carregar os trechos",
                );
                setItens([]);
            } finally {
                setCarregandoItens(false);
            }
        },
        [orgId],
    );

    const carregar = useCallback(async () => {
        setCarregando(true);
        setErroDeCarga(null);
        try {
            const dados = await obterAgente(orgId, agentId);
            setAgente(dados);
        } catch (falha) {
            setErroDeCarga(
                falha instanceof Error ? falha.message : "falha ao carregar o agente",
            );
            setCarregando(false);
            return;
        }
        setCarregando(false);
        void carregarGrupos();
    }, [orgId, agentId, carregarGrupos]);

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
    function registrarIndice(aviso: string | null, origem: string) {
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
            setIndice({ tipo: "atualizado", origem: "vínculo do grupo atualizado" });
        } catch (falha) {
            setErroDeVinculo(
                falha instanceof Error ? falha.message : "falha ao mudar o vínculo do grupo",
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
                origem: "reindexação concluída agora",
                relatorio: resultado.indexed,
            });
        } catch (falha) {
            if (falha instanceof ErroApi && falha.codigo === "embedding_unavailable") {
                setErroReindex(
                    `${falha.message}. O índice não mudou. Instale o modelo de embedding na instância e repita.`,
                );
            } else {
                setErroReindex(
                    falha instanceof Error ? falha.message : "falha ao reindexar o agente",
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
            registrarIndice(avisoDeIndice(grupo), "grupo gravado");
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
            registrarIndice(avisoDeIndice(item), "trecho gravado");
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
                    mensagem: "trecho removido; o índice precisa ser sincronizado",
                });
            }
        } catch (falha) {
            setErroDeItem(falha instanceof Error ? falha.message : "falha ao excluir o trecho");
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
                    title="Não consegui carregar a base do agente"
                    description="A instância não respondeu à leitura do agente. Tente de novo; se persistir, confira a saúde do servidor."
                    details={erroDeCarga}
                    retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guarda.tentarNavegar(`/orgs/${orgId}/agents`)}
                        >
                            Voltar para agentes
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
                label: "Editar",
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => setModalGrupo({ aberto: true, grupo }),
            },
            {
                id: "vinculo",
                label: vinculado ? "Desvincular" : "Vincular",
                icon: <IconLink size={14} aria-hidden="true" />,
                disabled: emVinculo === grupo.id,
                onSelect: () => void alternarVinculo(grupo),
            },
            {
                id: "excluir",
                label: "Excluir",
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
                label: "Editar",
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => setModalItem({ aberto: true, item: trecho }),
            },
            {
                id: "excluir",
                label: "Excluir",
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
                    descricao="trechos indexados que o agente consulta antes de responder"
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
                                Reindexar agente
                            </SofaButton>
                        </>
                    }
                />

                <AbasDoAgente org={orgId} agente={agentId} ativa="knowledge-base" nome={agente.name} />

                {erroDeVinculo && (
                    <SofaBanner tone="danger" title="Não foi possível mudar o vínculo">
                        {erroDeVinculo}
                    </SofaBanner>
                )}

                <section className="poph-secao">
                    <div className="poph-conhecimento__titulo-bloco">
                        <h2 className="poph-secao__titulo">Índice de busca do agente</h2>
                        <p className="poph-conhecimento__ajuda">
                            A base é consultada por busca híbrida (vetor + texto). Gravar um trecho
                            já tenta sincronizar o índice.
                        </p>
                    </div>

                    {indice.tipo === "nao-verificado" && (
                        <p className="poph-conhecimento__texto">
                            A API administrativa não expõe o estado do índice (não há rota de
                            status nem contagem de trechos indexados). Ele aparece aqui depois de
                            uma gravação ou de uma reindexação; para o estado real a qualquer
                            momento, use <code>poph kb status --offline</code> na instância.
                        </p>
                    )}

                    {indice.tipo === "atualizado" && (
                        <p className="poph-conhecimento__texto">
                            {indice.origem}.{" "}
                            {indice.relatorio
                                ? resumoDoRelatorio(indice.relatorio)
                                : "A sincronização rodou no servidor sem aviso de atraso."}
                        </p>
                    )}

                    {indice.tipo === "pendente" && (
                        <SofaBanner tone="warn" title="Reindexação necessária">
                            {indice.mensagem}. Clique em “Reindexar agente” para reconstruir o
                            índice deste agente.
                        </SofaBanner>
                    )}

                    {erroReindex && (
                        <SofaBanner tone="danger" title="A reindexação não concluiu">
                            {erroReindex}
                        </SofaBanner>
                    )}

                    <p className="poph-mono-mudo poph-conhecimento__nota">
                        A reindexação é síncrona: a tela espera a resposta do servidor, que pode
                        demorar (a primeira carrega o modelo de embedding). Não feche a aba durante
                        o processo.
                    </p>
                </section>

                <section className="poph-secao">
                    <div className="poph-conhecimento__cabecalho-secao">
                        <div className="poph-conhecimento__titulo-bloco">
                            <h2 className="poph-secao__titulo">Grupos da base</h2>
                            <p className="poph-conhecimento__ajuda">
                                Grupos ligados a este agente são consultados por ele. Grupos sem
                                vínculo existem na organização e não são usados.
                            </p>
                        </div>
                        <div className="poph-conhecimento__acoes-secao">
                            <SofaButton
                                variant="subtle"
                                icon={<IconRefresh aria-hidden="true" />}
                                onClick={() => void carregarGrupos()}
                                loading={carregandoGrupos && grupos.length > 0}
                            >
                                Atualizar
                            </SofaButton>
                            <SofaButton
                                variant="primary"
                                icon={<IconPlus aria-hidden="true" />}
                                onClick={() => setModalGrupo({ aberto: true, grupo: null })}
                            >
                                Novo grupo
                            </SofaButton>
                        </div>
                    </div>

                    {carregandoGrupos && grupos.length === 0 ? (
                        <SofaSkeleton lines={5} />
                    ) : erroGrupos ? (
                        <SofaErrorState
                            title="Não consegui carregar os grupos"
                            description="A lista de grupos não respondeu. Tente de novo."
                            details={erroGrupos}
                            retry={{
                                label: "Tentar de novo",
                                onClick: () => void carregarGrupos(),
                            }}
                        />
                    ) : grupos.length === 0 ? (
                        <SofaEmptyState
                            icon={<IconFolder aria-hidden="true" />}
                            title="Nenhum grupo de conhecimento"
                            description="Crie o primeiro grupo (por exemplo, “Políticas da loja”) e escreva os trechos que o agente deve consultar."
                            action={
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => setModalGrupo({ aberto: true, grupo: null })}
                                >
                                    Criar primeiro grupo
                                </SofaButton>
                            }
                        />
                    ) : (
                        <div
                            className="poph-tabela poph-conhecimento__grupos"
                            role="table"
                            aria-label="Grupos da base"
                        >
                            <div className="poph-tabela__cabecalho" role="row">
                                <span role="columnheader">Grupo</span>
                                <span role="columnheader">Vínculo</span>
                                <span role="columnheader">Situação</span>
                                <span role="columnheader">Versão</span>
                                <span role="columnheader" className="poph-tabela__celula--acoes">
                                    Ações
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
                                                {vinculado ? "usado por este agente" : "sem vínculo"}
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
                                                label={`Ações de ${grupo.name}`}
                                                align="end"
                                                trigger={
                                                    <SofaIconButton
                                                        label={`Ações de ${grupo.name}`}
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
                                {grupoAberto ? `Trechos · ${grupoAberto.name}` : "Trechos do grupo"}
                            </h2>
                            <p className="poph-conhecimento__ajuda">
                                {grupoAberto
                                    ? "Cada trecho é enviado ao agente como um bloco de consulta. Até 1000 caracteres por trecho."
                                    : "Escolha um grupo acima para ver e editar os trechos."}
                            </p>
                        </div>
                        {grupoAberto && (
                            <div className="poph-conhecimento__acoes-secao">
                                <span
                                    className="poph-conhecimento__contagem"
                                    aria-label="trechos no grupo"
                                >
                                    {carregandoItens ? "…" : plural(itens.length, "trecho", "trechos")}
                                </span>
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => setModalItem({ aberto: true, item: null })}
                                >
                                    Novo trecho
                                </SofaButton>
                            </div>
                        )}
                    </div>

                    {!grupoAberto ? (
                        <SofaEmptyState
                            icon={<IconDatabase aria-hidden="true" />}
                            title="Nenhum grupo selecionado"
                            description="Os trechos aparecem aqui quando um grupo é aberto."
                        />
                    ) : carregandoItens && itens.length === 0 ? (
                        <SofaSkeleton lines={5} />
                    ) : erroItens ? (
                        <SofaErrorState
                            title="Não consegui carregar os trechos"
                            description="A lista de trechos do grupo não respondeu. Tente de novo."
                            details={erroItens}
                            retry={{
                                label: "Tentar de novo",
                                onClick: () => void carregarItens(selecionado),
                            }}
                        />
                    ) : itens.length === 0 ? (
                        <SofaEmptyState
                            icon={<IconDatabase aria-hidden="true" />}
                            title="Grupo sem trechos"
                            description="Escreva o primeiro trecho: uma resposta completa, como o agente deve dizer ao cliente."
                            action={
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => setModalItem({ aberto: true, item: null })}
                                >
                                    Criar primeiro trecho
                                </SofaButton>
                            }
                        />
                    ) : (
                        <div
                            className="poph-tabela poph-conhecimento__trechos"
                            role="table"
                            aria-label="Trechos do grupo"
                        >
                            <div className="poph-tabela__cabecalho" role="row">
                                <span role="columnheader">Trecho</span>
                                <span role="columnheader">Situação</span>
                                <span role="columnheader">Detalhes</span>
                                <span role="columnheader" className="poph-tabela__celula--acoes">
                                    Ações
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
                                            {trecho.content.length} caracteres · ordem{" "}
                                            {trecho.position} · v{trecho.version}
                                        </span>
                                        {trecho.tags.length > 0 && (
                                            <span className="poph-conhecimento__tags">
                                                {trecho.tags.join(" · ")}
                                            </span>
                                        )}
                                    </div>
                                    <div role="cell" className="poph-tabela__celula--acoes">
                                        <SofaMenu
                                            label={`Ações de ${trecho.title}`}
                                            align="end"
                                            trigger={
                                                <SofaIconButton
                                                    label={`Ações de ${trecho.title}`}
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
                        <SofaBanner tone="danger" title="Não foi possível excluir o trecho">
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
                            mensagem:
                                "grupo removido; o índice deste agente precisa ser sincronizado",
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
                titulo="Excluir trecho?"
                descricao={`O trecho ${
                    itemParaExcluir?.title ?? ""
                } sai da base do agente. Não há como desfazer.`}
                recurso={itemParaExcluir?.title ?? ""}
                rotuloRecurso="título do trecho"
                rotuloConfirmar="Excluir trecho"
                aoConfirmar={excluirTrecho}
            />
        </>
    );
}

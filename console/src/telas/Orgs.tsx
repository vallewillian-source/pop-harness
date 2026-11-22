/**
 * Lista de organizações (PRD do re-layout §5.3, UX_SPEC §3.2) com o CRUD de organização (HR-39a).
 *
 * Token global cria, edita, reativa e exclui organização; token de org só enxerga a própria e
 * **não vê** essas ações — o servidor recusaria com `403`, e a UI não oferece o que ele recusa.
 *
 * Filtro (abas e texto) é local, sobre a lista já carregada: nenhuma chamada nova à API por aba,
 * por caractere digitado ou por linha (o PRD proíbe chamada extra por linha).
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { createColumnHelper } from "@tanstack/react-table";
import {
    IconBuilding,
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
import { pedir, type ListaOrgs, type Org } from "../api/cliente";
import { atualizarOrg, conflitoDeVersao, excluirOrg } from "../api/orgs";
import { DialogoExclusao } from "../componentes/DialogoExclusao";
import { useGuardaDeAlteracoes } from "../componentes/GuardaAlteracoes";
import { InspetorOrg } from "../componentes/InspetorOrg";
import { ModalOrg } from "../componentes/ModalOrg";
import { useSessao } from "../sessao/Sessao";
import "../estilos/orgs.css";

const coluna = createColumnHelper<Org>();

/** Abas de filtro da lista (PRD §5.3). "Arquivadas" só aparece quando existe alguma. */
type Aba = "todas" | "ativas" | "inativas" | "arquivadas";

const ABAS: Aba[] = ["todas", "ativas", "inativas", "arquivadas"];

const ROTULO_DA_ABA: Record<Aba, string> = {
    todas: "Todas",
    ativas: "Ativas",
    inativas: "Inativas",
    arquivadas: "Arquivadas",
};

/** Situações que cada aba aceita. */
const SITUACOES_DA_ABA: Record<Aba, string[]> = {
    todas: ["active", "inactive", "archived"],
    ativas: ["active"],
    inativas: ["inactive"],
    arquivadas: ["archived"],
};

/** Comparação de texto sem acento e sem caixa (o filtro é de navegador, PRD §5.3). */
function semAcento(texto: string): string {
    return texto
        .normalize("NFD")
        .replace(/[\u0300-\u036f]/g, "")
        .toLowerCase();
}

function tomDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return "poph-status--ativo";
        case "inactive":
        case "archived":
            return "poph-status--inativo";
        default:
            return "poph-status--inativo";
    }
}

function rotuloDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return "Ativa";
        case "inactive":
            return "Inativa";
        case "archived":
            return "Arquivada";
        default:
            return situacao;
    }
}

function dataCurta(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    return new Date(ms).toLocaleString("pt-BR", {
        day: "2-digit",
        month: "2-digit",
        year: "numeric",
    });
}

/** Retenção vem da configuração da org, que é JSON livre. */
function retencao(org: Org): string {
    const config = org.config as Record<string, unknown> | undefined;
    const valor = config?.["retention_days"] ?? config?.["retencao_dias"];
    if (typeof valor === "number") {
        return valor === 0 ? "nunca apaga" : `${valor} dias`;
    }
    return "padrão";
}

/** Aviso no topo da lista: conflito de versão ou falha de uma ação de linha. */
type Aviso = { tom: "warn" | "danger"; titulo: string; texto: string };

export function Orgs() {
    const { sessao, recarregarOrgs } = useSessao();
    const navegar = useNavigate();
    const { tentarNavegar } = useGuardaDeAlteracoes();
    const [orgs, setOrgs] = useState<Org[]>(sessao?.orgs ?? []);
    const [carregando, setCarregando] = useState(true);
    const [erro, setErro] = useState<string | null>(null);
    const [aba, setAba] = useState<Aba>("todas");
    const [filtro, setFiltro] = useState("");
    const [selecionada, setSelecionada] = useState<string | null>(null);
    const [formularioAberto, setFormularioAberto] = useState(false);
    const [emEdicao, setEmEdicao] = useState<Org | null>(null);
    const [paraExcluir, setParaExcluir] = useState<Org | null>(null);
    const [reativando, setReativando] = useState<string | null>(null);
    const [aviso, setAviso] = useState<Aviso | null>(null);
    /** Último clique de linha: o segundo clique na mesma linha, em seguida, abre a organização. */
    const ultimoClique = useRef<{ id: string; quando: number } | null>(null);
    /** Organização que deve receber o foco de volta assim que o inspetor fechar. */
    const focoPendente = useRef<string | null>(null);

    // Token de org não cria, edita, reativa nem exclui organização (o servidor responde 403).
    const podeGerenciar = sessao?.escopo !== "org";

    const carregar = useCallback(async () => {
        setCarregando(true);
        setErro(null);
        try {
            const lista = await pedir<ListaOrgs>("/admin/v1/orgs?limit=200");
            setOrgs(lista.items);
            // A lista também alimenta o seletor da casca.
            await recarregarOrgs();
        } catch (falha) {
            setErro(falha instanceof Error ? falha.message : "falha ao carregar as organizações");
        } finally {
            setCarregando(false);
        }
    }, [recarregarOrgs]);

    useEffect(() => {
        void carregar();
    }, [carregar]);

    const contagens = useMemo(() => {
        const porSituacao = (situacao: string) => orgs.filter((org) => org.status === situacao).length;
        return {
            todas: orgs.length,
            ativas: porSituacao("active"),
            inativas: porSituacao("inactive"),
            arquivadas: porSituacao("archived"),
        } satisfies Record<Aba, number>;
    }, [orgs]);

    // A aba "Arquivadas" some quando não há arquivada: se ela estava escolhida, volta para "Todas".
    const abasVisiveis = useMemo(
        () => ABAS.filter((candidata) => candidata !== "arquivadas" || contagens.arquivadas > 0),
        [contagens.arquivadas],
    );

    useEffect(() => {
        if (!abasVisiveis.includes(aba)) setAba("todas");
    }, [aba, abasVisiveis]);

    const visiveis = useMemo(() => {
        const termo = semAcento(filtro.trim());
        const situacoes = SITUACOES_DA_ABA[aba];
        return orgs.filter((org) => {
            if (!situacoes.includes(org.status)) return false;
            if (!termo) return true;
            return semAcento(org.name).includes(termo) || semAcento(org.slug).includes(termo);
        });
    }, [orgs, aba, filtro]);

    function abrirCriacao() {
        setEmEdicao(null);
        setFormularioAberto(true);
    }

    function abrirEdicao(org: Org) {
        setEmEdicao(org);
        setFormularioAberto(true);
    }

    /** Clique simples seleciona a linha e abre o inspetor (PRD §5.3/§5.4); duplo clique navega. */
    function selecionarLinha(org: Org) {
        const agora = Date.now();
        const anterior = ultimoClique.current;
        ultimoClique.current = { id: org.id, quando: agora };
        setSelecionada(org.id);
        if (anterior !== null && anterior.id === org.id && agora - anterior.quando < 400) {
            ultimoClique.current = null;
            navegar(`/orgs/${org.id}`);
        }
    }

    /** Fecha o inspetor devolvendo o foco à linha de onde ele foi aberto. */
    function fecharInspetor() {
        focoPendente.current = selecionada;
        setSelecionada(null);
    }

    // O foco volta depois da renderização: a tabela redesenha a linha quando a seleção sai, então
    // o elemento que existia no clique já não é o mesmo. O nome é o alvo de teclado da linha.
    useEffect(() => {
        const id = focoPendente.current;
        if (id === null) return;
        focoPendente.current = null;
        const alvo = document.querySelector<HTMLAnchorElement>(`[data-poph-org="${id}"]`);
        alvo?.focus();
    }, [selecionada]);

    /** "Acessar organização →": navegação da gaveta, passando pela guarda de alterações. */
    function acessarOrg(org: Org) {
        tentarNavegar(`/orgs/${org.id}`);
    }

    /**
     * Reativar uma organização inativa: `PATCH` com `If-Match` da versão lida.
     *
     * O `412` é tratado como o `ModalOrg` trata: nada se perde, a lista é relida e o operador vê
     * o conflito na hora, em vez de um erro genérico.
     */
    async function reativar(org: Org) {
        setReativando(org.id);
        setAviso(null);
        try {
            await atualizarOrg(org.id, org.version, { status: "active" });
            await carregar();
        } catch (falha) {
            const disputa = conflitoDeVersao(falha);
            if (disputa) {
                setAviso({
                    tom: "warn",
                    titulo: "Versão desatualizada",
                    texto: `Outra pessoa alterou ${org.name} antes de você${
                        typeof disputa.atual === "number" ? ` (agora na versão ${disputa.atual})` : ""
                    }. A lista foi atualizada: confira a situação e tente de novo.`,
                });
                await carregar();
                return;
            }
            setAviso({
                tom: "danger",
                titulo: "Não foi possível reativar",
                texto: falha instanceof Error ? falha.message : "falha inesperada ao reativar",
            });
        } finally {
            setReativando(null);
        }
    }

    function itensDoMenu(org: Org): SofaMenuItemSpec[] {
        const itens: SofaMenuItemSpec[] = [
            {
                id: "editar",
                label: "Editar",
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => abrirEdicao(org),
            },
        ];
        if (org.status !== "active") {
            itens.push({
                id: "reativar",
                label: "Reativar",
                icon: <IconPlay size={14} aria-hidden="true" />,
                onSelect: () => void reativar(org),
            });
        }
        itens.push({
            id: "excluir",
            label: "Excluir",
            icon: <IconTrash size={14} aria-hidden="true" />,
            danger: true,
            onSelect: () => setParaExcluir(org),
        });
        return itens;
    }

    const colunas = useMemo(
        () => [
            coluna.accessor("name", {
                header: "Organização",
                size: 260,
                cell: (info) => {
                    const org = info.row.original;
                    const atenuada = org.status !== "active";
                    return (
                        <div className="poph-orgs__identidade">
                            <Link
                                to={`/orgs/${org.id}`}
                                data-poph-org={org.id}
                                className={atenuada ? "poph-orgs__nome poph-orgs__nome--atenuado" : "poph-orgs__nome"}
                                onClick={(evento) => evento.stopPropagation()}
                            >
                                {info.getValue()}
                            </Link>
                            <span className="poph-mono-mudo">{org.slug}</span>
                        </div>
                    );
                },
            }),
            coluna.accessor("status", {
                header: "Situação",
                size: 130,
                cell: (info) => (
                    <span className={`poph-status ${tomDaSituacao(info.getValue())}`}>
                        <span className="poph-status__ponto" aria-hidden="true" />
                        {rotuloDaSituacao(info.getValue())}
                    </span>
                ),
            }),
            coluna.display({
                id: "retention",
                header: "Retenção",
                size: 110,
                cell: (info) => {
                    const org = info.row.original;
                    return (
                        <span
                            className={org.status === "active" ? undefined : "poph-orgs__celula--atenuada"}
                        >
                            {retencao(org)}
                        </span>
                    );
                },
            }),
            coluna.accessor("updated_at", {
                header: "Atualizada em",
                size: 150,
                cell: (info) => {
                    const org = info.row.original;
                    return (
                        <div className="poph-orgs__identidade">
                            <span
                                className={
                                    org.status === "active" ? undefined : "poph-orgs__celula--atenuada"
                                }
                            >
                                {dataCurta(info.getValue())}
                            </span>
                            <span className="poph-mono-mudo">v{org.version}</span>
                        </div>
                    );
                },
            }),
            coluna.accessor("created_at", {
                header: "Criada em",
                size: 120,
                cell: (info) => {
                    const org = info.row.original;
                    return (
                        <span
                            className={org.status === "active" ? undefined : "poph-orgs__celula--atenuada"}
                        >
                            {dataCurta(info.getValue())}
                        </span>
                    );
                },
            }),
            coluna.display({
                id: "actions",
                header: "Ações",
                cell: (info) => {
                    const org = info.row.original;
                    const aberta = selecionada === org.id;
                    return (
                        <div className="poph-orgs__acoes" onClick={(evento) => evento.stopPropagation()}>
                            <SofaButton
                                variant={aberta ? "primary" : "subtle"}
                                size="sm"
                                onClick={() => navegar(`/orgs/${org.id}`)}
                            >
                                Abrir
                            </SofaButton>
                            {podeGerenciar && (
                                <SofaMenu
                                    label={`Ações de ${org.name}`}
                                    align="end"
                                    trigger={
                                        <SofaIconButton
                                            label={`Ações de ${org.name}`}
                                            size={28}
                                            loading={reativando === org.id}
                                        >
                                            <IconMoreHorizontal size={16} aria-hidden="true" />
                                        </SofaIconButton>
                                    }
                                    items={itensDoMenu(org)}
                                />
                            )}
                        </div>
                    );
                },
            }),
        ],
        // As células mostram a seleção e o carregamento do Reativar, então as colunas são remontadas
        // quando esses estados mudam (as ações em si leem a organização pelo argumento).
        [navegar, podeGerenciar, selecionada, reativando],
    );

    const filtrando = aba !== "todas" || filtro.trim() !== "";
    /** A gaveta mostra sempre a organização da seleção atual, relida da lista. */
    const orgSelecionada = selecionada === null ? null : (orgs.find((org) => org.id === selecionada) ?? null);

    return (
        <div className={orgSelecionada ? "poph-orgs poph-com-gaveta" : "poph-orgs"}>
            <div className="poph-pagina poph-orgs__painel">
            <div className="poph-pagina__topo">
                <div>
                    <h1 className="poph-pagina__titulo">Organizações</h1>
                    <p className="poph-pagina__descricao">
                        Cada organização é um tenant isolado, com dados e limites próprios.
                    </p>
                </div>
                <div className="poph-pagina__acoes">
                    <SofaButton
                        variant="secondary"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void carregar()}
                        loading={carregando}
                    >
                        Atualizar
                    </SofaButton>
                    {podeGerenciar && (
                        <SofaButton
                            variant="primary"
                            icon={<IconPlus aria-hidden="true" />}
                            onClick={abrirCriacao}
                        >
                            Nova organização
                        </SofaButton>
                    )}
                </div>
            </div>

            {aviso && (
                <SofaBanner
                    tone={aviso.tom}
                    title={aviso.titulo}
                    dismissLabel="Fechar aviso"
                    onDismiss={() => setAviso(null)}
                >
                    {aviso.texto}
                </SofaBanner>
            )}

            <div className="poph-abas poph-orgs__filtros">
                <div className="poph-orgs__abas" role="group" aria-label="Filtrar por situação">
                    {abasVisiveis.map((candidata) => {
                        const ativa = candidata === aba;
                        return (
                            <button
                                key={candidata}
                                type="button"
                                aria-pressed={ativa}
                                className={ativa ? "poph-aba poph-aba--ativa" : "poph-aba"}
                                onClick={() => setAba(candidata)}
                            >
                                {ROTULO_DA_ABA[candidata]}
                                <span className="poph-aba__contagem">({contagens[candidata]})</span>
                            </button>
                        );
                    })}
                </div>
                <SofaSearchField
                    label="Filtrar organizações por nome ou slug"
                    value={filtro}
                    onValueChange={setFiltro}
                    placeholder="Filtrar por nome ou slug…"
                    size="md"
                    clearLabel="Limpar filtro"
                    className="poph-orgs__busca"
                />
            </div>

            {erro && !carregando ? (
                <SofaErrorState
                    title="Não consegui carregar as organizações"
                    description="A instância não respondeu à consulta. Verifique se o processo `poph serve` está no ar e tente de novo."
                    details={erro}
                    retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                />
            ) : carregando && orgs.length === 0 ? (
                <div className="poph-orgs__esqueleto" aria-busy="true" aria-label="Carregando organizações">
                    {[0, 1, 2, 3, 4].map((linha) => (
                        <div key={linha} className="poph-orgs__esqueleto-linha" />
                    ))}
                </div>
            ) : (
                <SofaTable
                    className="poph-orgs__tabela"
                    label="Organizações"
                    columns={colunas}
                    data={visiveis}
                    getRowId={(org) => org.id}
                    enableSelection
                    selectedIds={selecionada === null ? [] : [selecionada]}
                    onRowClick={selecionarLinha}
                    isLoading={carregando}
                    totalCount={visiveis.length}
                    footerMeta={() =>
                        `Exibindo ${visiveis.length} de ${orgs.length} ${
                            orgs.length === 1 ? "organização" : "organizações"
                        }`
                    }
                    footer={<span className="poph-orgs__dica">Tab percorre · Enter abre</span>}
                    empty={
                        filtrando ? (
                            <SofaEmptyState
                                title="Nenhuma organização encontrada"
                                description="Nenhuma organização corresponde à situação e ao texto escolhidos."
                                action={
                                    <SofaButton
                                        variant="secondary"
                                        onClick={() => {
                                            setAba("todas");
                                            setFiltro("");
                                        }}
                                    >
                                        Limpar filtros
                                    </SofaButton>
                                }
                            />
                        ) : (
                            <SofaEmptyState
                                icon={<IconBuilding aria-hidden="true" />}
                                title="Nenhuma organização cadastrada"
                                description={
                                    podeGerenciar
                                        ? "Comece criando a primeira organização: ela é o tenant que agrupa agentes, base de conhecimento e integrações."
                                        : "Este token enxerga uma organização só, e ela ainda não aparece na listagem."
                                }
                                action={
                                    podeGerenciar ? (
                                        <SofaButton
                                            variant="primary"
                                            icon={<IconPlus aria-hidden="true" />}
                                            onClick={abrirCriacao}
                                        >
                                            Criar primeira organização
                                        </SofaButton>
                                    ) : undefined
                                }
                            />
                        )
                    }
                />
            )}

            <ModalOrg
                aberto={formularioAberto}
                org={emEdicao}
                aoFechar={() => setFormularioAberto(false)}
                aoSalvar={() => {
                    void carregar();
                }}
            />

            {paraExcluir && (
                <DialogoExclusao
                    aberto={paraExcluir !== null}
                    aoFechar={() => setParaExcluir(null)}
                    titulo="Excluir organização?"
                    descricao={`A organização ${paraExcluir.name} e tudo que está dentro dela saem do sistema. Não há como desfazer.`}
                    recurso={paraExcluir.slug}
                    rotuloRecurso="slug da organização"
                    rotuloConfirmar="Excluir organização"
                    permiteForcar
                    rotuloForcar="Apagar também agentes, categorias, departamentos e tokens"
                    consequencia={
                        <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                            Serão apagados os agentes, as categorias, os departamentos, as conversas e
                            os tokens desta organização. A exclusão de uma organização com conteúdo
                            exige a confirmação extra abaixo.
                        </p>
                    }
                    aoConfirmar={(forcar) => excluirOrg(paraExcluir.id, forcar)}
                    aoExcluir={() => {
                        setParaExcluir(null);
                        void carregar();
                    }}
                />
            )}
            </div>

            {orgSelecionada && (
                <InspetorOrg
                    org={orgSelecionada}
                    aoFechar={fecharInspetor}
                    aoAcessar={acessarOrg}
                />
            )}
        </div>
    );
}

/**
 * Catálogo de produtos da organização (HR-42).
 *
 * Uma tabela com todos os produtos (de qualquer situação), filtros por nome, situação e categoria,
 * e a coluna "Agentes que oferecem", que conta quais agentes enxergam cada produto pela categoria
 * (ou por não ter categoria vinculada). Criar, editar e excluir passam pelos modais de produto.
 *
 * A tela fica no contexto da organização (`/orgs/:id/products`), não dentro do agente: o catálogo é
 * da organização e os agentes só ligam categorias (ver `docs/CONSOLE.md`).
 */

import { useCallback, useEffect, useId, useMemo, useState, type ReactNode } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { createColumnHelper } from "@tanstack/react-table";
import {
    IconPlus,
    IconRefresh,
    IconTag,
    SofaBanner,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaField,
    SofaInput,
    SofaSelect,
    SofaSkeleton,
    SofaTable,
} from "@sofacoding/sofa-ui";
import type { Agente } from "../api/agentes";
import type { Categoria } from "../api/estrutura";
import { obterOrg } from "../api/orgs";
import {
    agentesQueVeemProduto,
    carregarAgentesComCategorias,
    listarCategorias,
    listarProdutos,
    type Produto,
    type SituacaoProduto,
} from "../api/produtos";
import { DialogoExcluirProduto } from "../componentes/DialogoExcluirProduto";
import { ModalProduto } from "../componentes/ModalProduto";
import "../estilos/produtos.css";

type Situacao = "todas" | SituacaoProduto;

type Filtros = {
    busca: string;
    situacao: Situacao;
    categoria: string;
};

const FILTROS_INICIAIS: Filtros = { busca: "", situacao: "todas", categoria: "todas" };

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

/** Texto para comparar busca: minúsculas e sem acento. */
function normalizar(texto: string): string {
    return texto.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
}

function casaComBusca(produto: Produto, busca: string): boolean {
    if (!busca) return true;
    const alvo = normalizar(
        [produto.name, produto.slug, ...(produto.aliases ?? [])].join(" "),
    );
    return alvo.includes(normalizar(busca.trim()));
}

function textoDeAgentes(total: number | null): string {
    if (total === null) return "—";
    if (total === 0) return "nenhum";
    return total === 1 ? "1 agente" : `${total} agentes`;
}

const coluna = createColumnHelper<Produto>();

export function Produtos() {
    const { orgId = "" } = useParams();
    const navegar = useNavigate();
    const idDoCatalogo = useId();

    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [carregando, setCarregando] = useState(true);
    const [erro, setErro] = useState<string | null>(null);
    const [produtos, setProdutos] = useState<Produto[]>([]);
    const [categorias, setCategorias] = useState<Categoria[]>([]);
    const [agentes, setAgentes] = useState<Agente[] | null>(null);
    const [filtros, setFiltros] = useState<Filtros>(FILTROS_INICIAIS);
    const [avisoDoIndice, setAvisoDoIndice] = useState<string | null>(null);
    const [edicao, setEdicao] = useState<{ aberto: boolean; produto: Produto | null }>({
        aberto: false,
        produto: null,
    });
    const [paraExcluir, setParaExcluir] = useState<Produto | null>(null);

    const carregar = useCallback(async () => {
        setCarregando(true);
        setErro(null);
        try {
            const [lista, cats] = await Promise.all([
                listarProdutos(orgId),
                listarCategorias(orgId),
            ]);
            setProdutos(lista);
            setCategorias(cats);
        } catch (falha) {
            setErro(falha instanceof Error ? falha.message : "falha ao carregar produtos");
        } finally {
            setCarregando(false);
        }
    }, [orgId]);

    const carregarAgentes = useCallback(async () => {
        try {
            setAgentes(await carregarAgentesComCategorias(orgId));
        } catch {
            // Sem os agentes, a coluna mostra "—"; o catálogo segue normal.
            setAgentes(null);
        }
    }, [orgId]);

    useEffect(() => {
        setNomeDaOrg(null);
        obterOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
        void carregar();
        void carregarAgentes();
    }, [orgId, carregar, carregarAgentes]);

    const nomeDaCategoria = useMemo(() => {
        const mapa = new Map<string, string>();
        for (const categoria of categorias) mapa.set(categoria.id, categoria.name);
        return mapa;
    }, [categorias]);

    const contagemDeAgentes = useMemo(() => {
        const mapa = new Map<string, number>();
        if (agentes === null) return mapa;
        for (const produto of produtos) {
            mapa.set(produto.id, agentesQueVeemProduto(agentes, produto).length);
        }
        return mapa;
    }, [agentes, produtos]);

    const visiveis = useMemo(
        () =>
            produtos.filter(
                (produto) =>
                    casaComBusca(produto, filtros.busca) &&
                    (filtros.situacao === "todas" || produto.status === filtros.situacao) &&
                    (filtros.categoria === "todas" || produto.category_id === filtros.categoria),
            ),
        [produtos, filtros],
    );

    const colunas = useMemo(
        () => [
            coluna.accessor("name", {
                header: "Produto",
                cell: (info) => {
                    const produto = info.row.original;
                    return (
                        <div className="poph-pilha" style={{ gap: 2 }}>
                            <span style={{ color: "var(--pop-text-primary)" }}>{produto.name}</span>
                            <span className="poph-mono-mudo">{produto.slug}</span>
                            {produto.aliases?.length > 0 && (
                                <span className="poph-mono-mudo">
                                    também: {produto.aliases.join(", ")}
                                </span>
                            )}
                        </div>
                    );
                },
            }),
            coluna.accessor("category_id", {
                header: "Categoria",
                cell: (info) => nomeDaCategoria.get(info.getValue()) ?? "sem categoria",
            }),
            coluna.accessor("price_cents", {
                header: "Preço",
                cell: (info) => {
                    const produto = info.row.original;
                    return (
                        <div className="poph-pilha" style={{ gap: 2 }}>
                            <span>{produto.price_text}</span>
                            {produto.unit && <span className="poph-mono-mudo">{produto.unit}</span>}
                        </div>
                    );
                },
            }),
            coluna.accessor("status", {
                header: "Situação",
                cell: (info) => (
                    <span className={`poph-status ${classeDaSituacao(info.getValue())}`}>
                        <span className="poph-status__ponto" aria-hidden="true" />
                        {rotuloDaSituacao(info.getValue())}
                    </span>
                ),
            }),
            coluna.display({
                id: "agentes",
                header: "Agentes que oferecem",
                cell: (info) => (
                    <span
                        title={
                            agentes === null
                                ? "Não consegui ler os agentes desta organização"
                                : undefined
                        }
                    >
                        {textoDeAgentes(contagemDeAgentes.get(info.row.original.id) ?? null)}
                    </span>
                ),
            }),
            coluna.display({
                id: "acoes",
                header: "Ações",
                cell: (info) => {
                    const produto = info.row.original;
                    return (
                        <div style={{ display: "flex", gap: 4, justifyContent: "flex-end" }}>
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                aria-label={`Editar produto ${produto.name}`}
                                onClick={() => setEdicao({ aberto: true, produto })}
                            >
                                Editar
                            </SofaButton>
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                aria-label={`Excluir produto ${produto.name}`}
                                onClick={() => setParaExcluir(produto)}
                            >
                                Excluir
                            </SofaButton>
                        </div>
                    );
                },
            }),
        ],
        [agentes, contagemDeAgentes, nomeDaCategoria],
    );

    function aoSalvarProduto(salvo: Produto) {
        setProdutos((atual) => {
            const existe = atual.some((item) => item.id === salvo.id);
            return existe
                ? atual.map((item) => (item.id === salvo.id ? salvo : item))
                : [...atual, salvo];
        });
        setAvisoDoIndice(salvo.index_warning ?? null);
        void carregarAgentes();
    }

    function limparFiltros() {
        setFiltros(FILTROS_INICIAIS);
    }

    const semProdutos = !carregando && !erro && produtos.length === 0;
    const semResultado = !carregando && !erro && produtos.length > 0 && visiveis.length === 0;

    let corpo: ReactNode;
    if (erro && !carregando) {
        corpo = (
            <SofaErrorState
                title="Não consegui carregar os produtos"
                description="A instância não respondeu. Verifique se o processo `poph serve` está no ar e tente de novo."
                details={erro}
                retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
            />
        );
    } else if (carregando && produtos.length === 0) {
        corpo = (
            <div aria-busy="true" aria-label="Carregando produtos">
                <SofaSkeleton lines={4} />
            </div>
        );
    } else if (semProdutos) {
        const semCategorias = categorias.length === 0;
        corpo = (
            <SofaEmptyState
                icon={<IconTag aria-hidden="true" />}
                title="Nenhum produto cadastrado"
                description={
                    semCategorias
                        ? "Todo produto pertence a uma categoria. Crie uma categoria na tela de Estrutura primeiro."
                        : "Os agentes só conseguem informar preço e condições de produtos cadastrados aqui."
                }
                action={
                    semCategorias ? (
                        <SofaButton
                            variant="primary"
                            onClick={() => navegar(`/orgs/${orgId}/structure`)}
                        >
                            Ir para Estrutura
                        </SofaButton>
                    ) : (
                        <SofaButton
                            variant="primary"
                            icon={<IconPlus aria-hidden="true" />}
                            onClick={() => setEdicao({ aberto: true, produto: null })}
                        >
                            Criar primeiro produto
                        </SofaButton>
                    )
                }
            />
        );
    } else if (semResultado) {
        corpo = (
            <SofaEmptyState
                icon={<IconTag aria-hidden="true" />}
                title="Nenhum produto com esses filtros"
                description="Mude a busca, a situação ou a categoria para ver outros produtos."
                action={
                    <SofaButton variant="secondary" onClick={limparFiltros}>
                        Limpar filtros
                    </SofaButton>
                }
            />
        );
    } else {
        corpo = (
            <div className="poph-produtos__tabela">
                <SofaTable
                    label="Produtos da organização"
                    columns={colunas}
                    data={visiveis}
                    getRowId={(produto) => produto.id}
                    isLoading={carregando}
                    footerMeta={({ total }) =>
                        total === produtos.length
                            ? total === 1
                                ? "1 produto"
                                : `${total} produtos`
                            : `${total} de ${produtos.length} produtos`
                    }
                />
            </div>
        );
    }

    const totalDeCategorias = categorias.length;

    return (
        <>
            <div className="poph-pagina poph-produtos">
            <div className="poph-pagina__topo">
                <div>
                    <p className="poph-produtos__eyebrow">
                        {nomeDaOrg ? `Organização · ${nomeDaOrg}` : "Organização"}
                    </p>
                    <h1 className="poph-pagina__titulo">Produtos</h1>
                    <p className="poph-pagina__descricao">
                        O que o agente informa ao cliente sobre preço e condições sai daqui. Inativos
                        e arquivados saem da busca.
                    </p>
                </div>
                <div className="poph-pagina__acoes">
                    <SofaButton
                        variant="primary"
                        icon={<IconPlus aria-hidden="true" />}
                        disabled={totalDeCategorias === 0}
                        onClick={() => setEdicao({ aberto: true, produto: null })}
                    >
                        Novo produto
                    </SofaButton>
                    <SofaButton
                        variant="secondary"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => {
                            void carregar();
                            void carregarAgentes();
                        }}
                        loading={carregando}
                    >
                        Atualizar
                    </SofaButton>
                </div>
            </div>
                {avisoDoIndice && (
                    <SofaBanner
                        tone="warn"
                        title="Produto salvo, índice de busca não atualizou"
                        onDismiss={() => setAvisoDoIndice(null)}
                    >
                        Os agentes só passam a usar a mudança na busca depois que o índice for
                        reconstruído. Detalhe: <span className="poph-mono-mudo">{avisoDoIndice}</span>
                    </SofaBanner>
                )}
                {agentes === null && !carregando && (
                    <SofaBanner tone="info" title="Agentes não carregaram">
                        A coluna "Agentes que oferecem" mostra "—" até a leitura dos agentes
                        funcionar. O catálogo segue normal.
                    </SofaBanner>
                )}
                <section className="poph-secao poph-produtos__secao" aria-labelledby={idDoCatalogo}>
                    <div className="poph-produtos__cabecalho">
                        <h2 id={idDoCatalogo} className="poph-secao__titulo">
                            Catálogo
                        </h2>
                        <div className="poph-produtos__filtros">
                            <SofaField label="Buscar por nome" htmlFor="poph-produtos-busca">
                                {(controle) => (
                                    <SofaInput
                                        id={controle.id}
                                        value={filtros.busca}
                                        placeholder="Nome, identificador ou outro nome"
                                        autoComplete="off"
                                        onChange={(evento) =>
                                            setFiltros((atual) => ({
                                                ...atual,
                                                busca: evento.target.value,
                                            }))
                                        }
                                    />
                                )}
                            </SofaField>
                            <SofaField label="Situação" htmlFor="poph-produtos-situacao">
                                {(controle) => (
                                    <SofaSelect
                                        id={controle.id}
                                        value={filtros.situacao}
                                        onChange={(evento) =>
                                            setFiltros((atual) => ({
                                                ...atual,
                                                situacao: evento.target.value as Situacao,
                                            }))
                                        }
                                    >
                                        <option value="todas">Todas</option>
                                        <option value="active">Ativos</option>
                                        <option value="inactive">Inativos</option>
                                        <option value="archived">Arquivados</option>
                                    </SofaSelect>
                                )}
                            </SofaField>
                            <SofaField label="Categoria" htmlFor="poph-produtos-categoria">
                                {(controle) => (
                                    <SofaSelect
                                        id={controle.id}
                                        value={filtros.categoria}
                                        onChange={(evento) =>
                                            setFiltros((atual) => ({
                                                ...atual,
                                                categoria: evento.target.value,
                                            }))
                                        }
                                    >
                                        <option value="todas">Todas</option>
                                        {categorias.map((categoria) => (
                                            <option key={categoria.id} value={categoria.id}>
                                                {categoria.name}
                                            </option>
                                        ))}
                                    </SofaSelect>
                                )}
                            </SofaField>
                        </div>
                    </div>
                    {corpo}
                </section>
            </div>

            <ModalProduto
                org={orgId}
                aberto={edicao.aberto}
                produto={edicao.produto}
                categorias={categorias}
                aoFechar={() => setEdicao({ aberto: false, produto: null })}
                aoSalvar={aoSalvarProduto}
            />

            <DialogoExcluirProduto
                aberto={paraExcluir !== null}
                aoFechar={() => setParaExcluir(null)}
                org={orgId}
                produto={paraExcluir}
                agentesQueVeem={
                    paraExcluir && agentes !== null && contagemDeAgentes.has(paraExcluir.id)
                        ? (contagemDeAgentes.get(paraExcluir.id) ?? 0)
                        : null
                }
                aoExcluir={(aviso) => {
                    if (paraExcluir) {
                        setProdutos((atual) => atual.filter((item) => item.id !== paraExcluir.id));
                    }
                    setParaExcluir(null);
                    setAvisoDoIndice(aviso);
                    void carregarAgentes();
                }}
            />
        </>
    );
}

/**
 * Lista de organizações no layout da prancha 01 do FigCraft (CF-03b), com o CRUD de organização
 * (HR-39a).
 *
 * Token global cria, edita, reativa e exclui organização; token de org só enxerga a própria e
 * **não vê** essas ações — o servidor recusaria com `403`, e a UI não oferece o que ele recusa.
 *
 * A lista vem de uma consulta só, `GET /admin/v1/orgs?cluster=true`: dela saem o nó de cada
 * organização (`instancia`, com o estado de cada nó em `instancias`) e a contagem de agentes
 * ativos (`agents_active_count`). Nenhuma chamada por linha. O filtro (abas e texto) é local, sobre
 * a lista já carregada.
 */

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
    pedir,
    type InstanciaDaLista,
    type ListaOrgsDoCluster,
    type Org,
    type OrgNaLista,
} from "../api/cliente";
import { atualizarOrg, conflitoDeVersao, excluirOrg } from "../api/orgs";
import { consultarMetricas, consultarSaude } from "../api/operacao";
import { baixarArquivo, exportarOrganizacao, falhaDeBackup } from "../api/backup";
import { DialogoExclusao } from "../componentes/DialogoExclusao";
import { useGuardaDeAlteracoes } from "../componentes/GuardaAlteracoes";
import { InspetorOrg, type NoDaOrg } from "../componentes/InspetorOrg";
import { ModalOrg } from "../componentes/ModalOrg";
import { useSessao } from "../sessao/Sessao";
import "../estilos/orgs.css";

const coluna = createColumnHelper<OrgNaLista>();

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

/** Frase do nó na coluna "Nó / instância", a partir do estado que a listagem devolve. */
function rotuloDoNo(instancia: InstanciaDaLista | null | undefined): string {
    switch (instancia?.situacao) {
        case "local":
            return "online";
        case "ok":
            return "remoto · online";
        case "inacessivel":
            return "remoto · inacessível";
        default:
            return "";
    }
}

/** Tempo ligado do processo, em dias/horas/minutos, como no desenho ("4h 12m"). */
function formatarUptime(segundos: number): string {
    const total = Math.max(0, Math.floor(segundos));
    const dias = Math.floor(total / 86_400);
    const horas = Math.floor((total % 86_400) / 3_600);
    const minutos = Math.floor((total % 3_600) / 60);
    if (dias > 0) return `${dias}d ${horas}h`;
    if (horas > 0) return `${horas}h ${minutos}m`;
    return `${minutos}m`;
}

function formatarMemoria(bytes: number): string {
    return `${Math.round(bytes / (1024 * 1024))} MB`;
}

/** Saúde e métricas do nó que responde à tela. Falha de uma delas só apaga o pedaço dela. */
type Telemetria = {
    situacao: "ok" | "degradada" | "sem_resposta";
    uptimeSegundos?: number;
    memoriaBytes?: number;
};

async function lerTelemetria(): Promise<Telemetria> {
    const [saudeLida, metricasLidas] = await Promise.allSettled([consultarSaude(), consultarMetricas()]);
    return {
        situacao: saudeLida.status === "fulfilled" ? saudeLida.value.situacao : "sem_resposta",
        uptimeSegundos:
            saudeLida.status === "fulfilled" ? saudeLida.value.saude.uptime_seconds : undefined,
        memoriaBytes: metricasLidas.status === "fulfilled" ? metricasLidas.value.memoriaProcesso : undefined,
    };
}

/** Aviso no topo da lista: conflito de versão ou falha de uma ação de linha. */
type Aviso = { tom: "warn" | "danger"; titulo: string; texto: string };

export function Orgs() {
    const { sessao, recarregarOrgs } = useSessao();
    const navegar = useNavigate();
    const { tentarNavegar } = useGuardaDeAlteracoes();
    const [orgs, setOrgs] = useState<OrgNaLista[]>(sessao?.orgs ?? []);
    /** Estado das instâncias do cluster. `null` quando a resposta não traz (token de org). */
    const [instancias, setInstancias] = useState<InstanciaDaLista[] | null>(null);
    const [telemetria, setTelemetria] = useState<Telemetria | null>(null);
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

    const mapaDeInstancias = useMemo(
        () => new Map((instancias ?? []).map((instancia) => [instancia.id, instancia])),
        [instancias],
    );

    /** Nó local, quando a resposta traz o estado das instâncias. */
    const instanciaLocal = useMemo(
        () => instancias?.find((instancia) => instancia.situacao === "local") ?? null,
        [instancias],
    );

    /**
     * Organização que mora em outro nó: o console deste nó não tem o painel dela, então a tela não
     * abre (o botão fica desabilitado e a linha não navega). Sem dado de cluster, nada é remoto.
     */
    const ehDeOutroNo = useCallback(
        (org: OrgNaLista): boolean =>
            instanciaLocal !== null && !!org.instancia && org.instancia !== instanciaLocal.id,
        [instanciaLocal],
    );

    /**
     * Nó da organização para o inspetor, com a mesma leitura da coluna "Nó / instância":
     * `null` quando a resposta não traz o cluster (token de organização), `id` vazio na
     * instalação de nó único.
     */
    const noDaOrg = useCallback(
        (org: OrgNaLista): NoDaOrg | null => {
            if (instancias === null) return null;
            const id = org.instancia ?? "";
            if (id === "") return { id: "", situacao: "local" };
            return { id, situacao: mapaDeInstancias.get(id)?.situacao ?? "desconhecido" };
        },
        [instancias, mapaDeInstancias],
    );

    const carregar = useCallback(async () => {
        setCarregando(true);
        setErro(null);
        const telemetriaPronta = lerTelemetria();
        try {
            const lista = await pedir<ListaOrgsDoCluster>("/admin/v1/orgs?cluster=true");
            setOrgs(lista.items);
            setInstancias(lista.instancias ?? null);
            // A lista também alimenta o seletor da casca.
            await recarregarOrgs();
        } catch (falha) {
            setErro(falha instanceof Error ? falha.message : "falha ao carregar as organizações");
        } finally {
            setCarregando(false);
        }
        setTelemetria(await telemetriaPronta);
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
    function selecionarLinha(org: OrgNaLista) {
        const agora = Date.now();
        const anterior = ultimoClique.current;
        ultimoClique.current = { id: org.id, quando: agora };
        setSelecionada(org.id);
        if (
            anterior !== null &&
            anterior.id === org.id &&
            agora - anterior.quando < 400 &&
            !ehDeOutroNo(org)
        ) {
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

    /** "Acessar →": navegação da gaveta, passando pela guarda de alterações. */
    function acessarOrg(org: OrgNaLista) {
        if (ehDeOutroNo(org)) {
            setAviso({
                tom: "warn",
                titulo: "Organização em outro nó",
                texto: `${org.name} está no nó ${org.instancia}. Abra o console desse nó para acessá-la.`,
            });
            return;
        }
        tentarNavegar(`/orgs/${org.id}`);
    }

    /** Exportar: baixa o arquivo da organização (sem conversas), o mesmo que a tela de operação baixa. */
    async function exportar(org: OrgNaLista) {
        setAviso(null);
        try {
            baixarArquivo(await exportarOrganizacao(org.id, false));
        } catch (falha) {
            const detalhe = falhaDeBackup(falha, "exportar");
            setAviso({ tom: "danger", titulo: detalhe.titulo, texto: detalhe.texto });
        }
    }

    /**
     * Reativar uma organização inativa: `PATCH` com `If-Match` da versão lida.
     *
     * O `412` é tratado como o `ModalOrg` trata: nada se perde, a lista é relida e o operador vê
     * o conflito na hora, em vez de um erro genérico.
     */
    async function reativar(org: OrgNaLista) {
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

    function itensDoMenu(org: OrgNaLista): SofaMenuItemSpec[] {
        const remota = ehDeOutroNo(org);
        const itens: SofaMenuItemSpec[] = [
            {
                id: "detalhes",
                label: "Ver detalhes",
                icon: <IconEye size={14} aria-hidden="true" />,
                onSelect: () => setSelecionada(org.id),
            },
            {
                id: "editar",
                label: "Editar",
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => abrirEdicao(org),
            },
            {
                id: "exportar",
                label: "Exportar",
                icon: <IconDownload size={14} aria-hidden="true" />,
                onSelect: () => void exportar(org),
            },
        ];
        if (org.status !== "active") {
            // Inativa: a linha mostra "Reativar", então o acesso à organização fica aqui.
            itens.push({
                id: "acessar",
                label: "Acessar organização",
                icon: <IconPlay size={14} aria-hidden="true" />,
                disabled: remota,
                onSelect: () => acessarOrg(org),
            });
            itens.push({
                id: "reativar",
                label: "Reativar",
                icon: <IconPlay size={14} aria-hidden="true" />,
                disabled: remota,
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
                header: "Organização / slug",
                size: 210,
                cell: (info) => {
                    const org = info.row.original;
                    const atenuada = org.status !== "active";
                    const classeDoNome = [
                        "poph-orgs__nome",
                        atenuada ? "poph-orgs__nome--atenuado" : "",
                        selecionada === org.id ? "poph-orgs__nome--selecionado" : "",
                    ]
                        .filter(Boolean)
                        .join(" ");
                    return (
                        <div className="poph-orgs__identidade-linha">
                            <span
                                className={atenuada ? "poph-orgs__tile poph-orgs__tile--atenuado" : "poph-orgs__tile"}
                                aria-hidden="true"
                            >
                                <IconBuilding size={14} aria-hidden="true" />
                            </span>
                            <div className="poph-orgs__identidade">
                                {ehDeOutroNo(org) ? (
                                    <span className={classeDoNome}>{info.getValue()}</span>
                                ) : (
                                    <Link
                                        to={`/orgs/${org.id}`}
                                        data-poph-org={org.id}
                                        className={classeDoNome}
                                        onClick={(evento) => evento.stopPropagation()}
                                    >
                                        {info.getValue()}
                                    </Link>
                                )}
                                <span className="poph-mono-mudo">{org.slug}</span>
                            </div>
                        </div>
                    );
                },
            }),
            coluna.accessor("status", {
                header: "Situação",
                size: 100,
                cell: (info) => (
                    <span className={`poph-status ${tomDaSituacao(info.getValue())}`}>
                        <span className="poph-status__ponto" aria-hidden="true" />
                        {rotuloDaSituacao(info.getValue())}
                    </span>
                ),
            }),
            coluna.accessor("instancia", {
                header: "Nó / instância",
                size: 130,
                cell: (info) => {
                    // Sem dado de cluster (token de org), a coluna não tem o que mostrar.
                    if (instancias === null) {
                        return (
                            <span className="poph-orgs__celula--atenuada" title="A resposta não traz o nó desta organização">
                                —
                            </span>
                        );
                    }
                    // Instância sem id configurado (instalação única): a org é do nó local.
                    const id = info.getValue() ?? "";
                    const nodo = id === "" ? instanciaLocal : (mapaDeInstancias.get(id) ?? null);
                    const rotulo = rotuloDoNo(nodo);
                    return (
                        <div className="poph-orgs__identidade">
                            <span className="poph-mono">{id || "nó local"}</span>
                            {rotulo !== "" && <span className="poph-mono-mudo">{rotulo}</span>}
                        </div>
                    );
                },
            }),
            coluna.accessor("agents_active_count", {
                header: "Agentes ativos",
                size: 110,
                cell: (info) => {
                    const total = info.getValue();
                    if (typeof total !== "number") {
                        return (
                            <span
                                className="poph-orgs__celula--atenuada"
                                title="A instância ainda não informa a contagem de agentes ativos"
                            >
                                —
                            </span>
                        );
                    }
                    const org = info.row.original;
                    return (
                        <span className={org.status === "active" ? undefined : "poph-orgs__celula--atenuada"}>
                            {total === 1 ? "1 agente" : `${total} agentes`}
                        </span>
                    );
                },
            }),
            coluna.display({
                id: "retention",
                header: "Retenção",
                size: 90,
                cell: (info) => {
                    const org = info.row.original;
                    return (
                        <span className={org.status === "active" ? undefined : "poph-orgs__celula--atenuada"}>
                            {retencao(org)}
                        </span>
                    );
                },
            }),
            coluna.accessor("updated_at", {
                header: "Atualizada",
                size: 120,
                cell: (info) => {
                    const org = info.row.original;
                    return (
                        <div className="poph-orgs__identidade">
                            <span className={org.status === "active" ? undefined : "poph-orgs__celula--atenuada"}>
                                {dataCurta(info.getValue())}
                            </span>
                            <span className="poph-mono-mudo">v{org.version}</span>
                        </div>
                    );
                },
            }),
            coluna.display({
                id: "actions",
                header: "Ações",
                size: 170,
                cell: (info) => {
                    const org = info.row.original;
                    const aberta = selecionada === org.id;
                    // Inativa com gestão: a ação da linha é reativar; o acesso fica no menu.
                    const reativarNaLinha = podeGerenciar && org.status === "inactive";
                    return (
                        <div className="poph-orgs__acoes" onClick={(evento) => evento.stopPropagation()}>
                            {reativarNaLinha ? (
                                <SofaButton
                                    variant="secondary"
                                    size="sm"
                                    loading={reativando === org.id}
                                    disabled={ehDeOutroNo(org)}
                                    onClick={() => void reativar(org)}
                                >
                                    Reativar
                                </SofaButton>
                            ) : (
                                <SofaButton
                                    variant={aberta ? "primary" : "subtle"}
                                    size="sm"
                                    disabled={ehDeOutroNo(org)}
                                    onClick={() => acessarOrg(org)}
                                >
                                    Acessar →
                                </SofaButton>
                            )}
                            {podeGerenciar && (
                                <SofaMenu
                                    label={`Ações de ${org.name}`}
                                    align="end"
                                    trigger={
                                        <SofaIconButton label={`Ações de ${org.name}`} size={28}>
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
        [navegar, podeGerenciar, selecionada, reativando, instancias, instanciaLocal, mapaDeInstancias, ehDeOutroNo, tentarNavegar],
    );

    const filtrando = aba !== "todas" || filtro.trim() !== "";
    /** A gaveta mostra sempre a organização da seleção atual, relida da lista. */
    const orgSelecionada = selecionada === null ? null : (orgs.find((org) => org.id === selecionada) ?? null);

    const partesDaTelemetria: string[] = [];
    if (instancias) {
        const conectados = instancias.filter((i) => i.situacao === "local" || i.situacao === "ok").length;
        const inacessiveis = instancias.filter((i) => i.situacao === "inacessivel").length;
        partesDaTelemetria.push(`${conectados} ${conectados === 1 ? "nó conectado" : "nós conectados"}`);
        if (inacessiveis > 0) {
            partesDaTelemetria.push(`${inacessiveis} ${inacessiveis === 1 ? "nó inacessível" : "nós inacessíveis"}`);
        }
    }
    if (telemetria?.situacao === "sem_resposta") partesDaTelemetria.push("sem resposta do nó local");
    if (telemetria?.situacao === "degradada") partesDaTelemetria.push("banco do nó local com problema");
    if (telemetria?.uptimeSegundos !== undefined) {
        partesDaTelemetria.push(`Uptime ${formatarUptime(telemetria.uptimeSegundos)}`);
    }
    if (telemetria?.memoriaBytes !== undefined) {
        partesDaTelemetria.push(`RAM ${formatarMemoria(telemetria.memoriaBytes)}`);
    }
    const tomDaTelemetria =
        telemetria?.situacao === "sem_resposta"
            ? "falha"
            : telemetria?.situacao === "degradada" || (instancias?.some((i) => i.situacao === "inacessivel") ?? false)
              ? "aviso"
              : "ok";

    return (
        <div className={orgSelecionada ? "poph-orgs poph-com-gaveta" : "poph-orgs"}>
            <div className="poph-pagina poph-orgs__painel">
                <div className="poph-pagina__topo">
                    <div>
                        <div className="poph-orgs__titulo-linha">
                            <h1 className="poph-pagina__titulo">Organizações</h1>
                            <span className="poph-orgs__contagem">{orgs.length}</span>
                        </div>
                        <p className="poph-pagina__descricao">
                            Cada organização é um tenant isolado, com dados e limites próprios.
                        </p>
                    </div>
                    <div className="poph-orgs__acoes-topo">
                        <SofaSearchField
                            label="Filtrar organizações por nome ou slug"
                            value={filtro}
                            onValueChange={setFiltro}
                            placeholder="Filtrar por nome ou slug…"
                            size="md"
                            clearLabel="Limpar filtro"
                            className="poph-orgs__busca"
                        />
                        <SofaIconButton label="Atualizar" size={32} loading={carregando} onClick={() => void carregar()}>
                            <IconRefresh aria-hidden="true" />
                        </SofaIconButton>
                        {podeGerenciar && (
                            <SofaButton variant="primary" icon={<IconPlus aria-hidden="true" />} onClick={abrirCriacao}>
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

                <div className="poph-orgs__barra">
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
                    {partesDaTelemetria.length > 0 && (
                        <p className="poph-orgs__telemetria">
                            <span
                                className={`poph-orgs__ponto poph-orgs__ponto--${tomDaTelemetria}`}
                                aria-hidden="true"
                            />
                            {partesDaTelemetria.join(" · ")}
                        </p>
                    )}
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

                <div className="poph-orgs__cards">
                    <section className="poph-orgs__card" aria-labelledby="poph-card-matrix">
                        <div className="poph-orgs__card-cabeca">
                            <h2 id="poph-card-matrix" className="poph-orgs__card-titulo">
                                Espelho Matrix Synapse
                            </h2>
                            <span
                                className="poph-orgs__selo"
                                title="A lista do cluster não traz o estado do Matrix; ele é consultado por organização"
                            >
                                indisponível
                            </span>
                        </div>
                        <p className="poph-orgs__card-texto">
                            O estado do Matrix é de cada organização. Abra uma organização para ver a conexão dela.
                        </p>
                    </section>
                    <section className="poph-orgs__card" aria-labelledby="poph-card-isolamento">
                        <div className="poph-orgs__card-cabeca">
                            <h2 id="poph-card-isolamento" className="poph-orgs__card-titulo">
                                Isolamento Multi-Tenant
                            </h2>
                        </div>
                        <p className="poph-orgs__card-texto">
                            Cada organização tem agentes, fichas, catálogo e tokens próprios. Um token de organização
                            só enxerga a sua.
                        </p>
                    </section>
                    <section className="poph-orgs__card" aria-labelledby="poph-card-cli">
                        <div className="poph-orgs__card-cabeca">
                            <h2 id="poph-card-cli" className="poph-orgs__card-titulo">
                                Comandos CLI Úteis
                            </h2>
                            <span className="poph-orgs__selo">poph</span>
                        </div>
                        <ul className="poph-orgs__comandos">
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
                    no={noDaOrg(orgSelecionada)}
                    podeGerenciar={podeGerenciar}
                    remota={ehDeOutroNo(orgSelecionada)}
                    aoFechar={fecharInspetor}
                    aoAcessar={acessarOrg}
                    aoEditar={abrirEdicao}
                    aoExportar={(org) => void exportar(org)}
                    aoExcluir={(org) => setParaExcluir(org)}
                />
            )}
        </div>
    );
}

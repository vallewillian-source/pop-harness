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
    request,
    type ListInstance,
    type ClusterOrgList,
    type Org,
    type OrgInList,
} from "../api/client";
import { updateOrg, versionConflict, deleteOrg } from "../api/orgs";
import { fetchMetrics, fetchHealth } from "../api/operation";
import { downloadFile, exportOrg, backupFailure } from "../api/backup";
import { DialogoExclusao } from "../componentes/DialogoExclusao";
import { useGuardaDeAlteracoes } from "../componentes/GuardaAlteracoes";
import { InspetorOrg, type NoDaOrg } from "../componentes/InspetorOrg";
import { ModalOrg } from "../componentes/ModalOrg";
import { useSession } from "../session/Session";
import { translate, useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import { formatDate } from "../i18n/format";
import "../estilos/orgs.css";

const coluna = createColumnHelper<OrgInList>();

/** Abas de filtro da lista (PRD §5.3). "Arquivadas" só aparece quando existe alguma. */
type Aba = "todas" | "ativas" | "inativas" | "arquivadas";

const ABAS: Aba[] = ["todas", "ativas", "inativas", "arquivadas"];

const ROTULO_DA_ABA: Record<Aba, TranslationKey> = {
    todas: "orgs.tabAll",
    ativas: "orgs.tabActive",
    inativas: "orgs.tabInactive",
    arquivadas: "orgs.tabArchived",
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
            return translate("orgs.statusActive");
        case "inactive":
            return translate("orgs.statusInactive");
        case "archived":
            return translate("orgs.statusArchived");
        default:
            return situacao;
    }
}

function dataCurta(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    return formatDate(ms);
}

/** Retenção vem da configuração da org, que é JSON livre. */
function retencao(org: Org): string {
    const config = org.config as Record<string, unknown> | undefined;
    const valor = config?.["retention_days"] ?? config?.["retencao_dias"];
    if (typeof valor === "number") {
        return valor === 0
            ? translate("orgs.retentionNever")
            : translate("orgs.retentionDays", { days: valor });
    }
    return translate("orgs.retentionDefault");
}

/** Frase do nó na coluna "Nó / instância", a partir do estado que a listagem devolve. */
function rotuloDoNo(instancia: ListInstance | null | undefined): string {
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
    const [saudeLida, metricasLidas] = await Promise.allSettled([fetchHealth(), fetchMetrics()]);
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
    const { translate: t, locale } = useI18n();
    const { sessao, recarregarOrgs } = useSession();
    const navegar = useNavigate();
    const { tentarNavegar } = useGuardaDeAlteracoes();
    const [orgs, setOrgs] = useState<OrgInList[]>(sessao?.orgs ?? []);
    /** Estado das instâncias do cluster. `null` quando a resposta não traz (token de org). */
    const [instancias, setInstancias] = useState<ListInstance[] | null>(null);
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
        (org: OrgInList): boolean =>
            instanciaLocal !== null && !!org.instancia && org.instancia !== instanciaLocal.id,
        [instanciaLocal],
    );

    /**
     * Nó da organização para o inspetor, com a mesma leitura da coluna "Nó / instância":
     * `null` quando a resposta não traz o cluster (token de organização), `id` vazio na
     * instalação de nó único.
     */
    const noDaOrg = useCallback(
        (org: OrgInList): NoDaOrg | null => {
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
            const lista = await request<ClusterOrgList>("/admin/v1/orgs?cluster=true");
            setOrgs(lista.items);
            setInstancias(lista.instancias ?? null);
            // A lista também alimenta o seletor da casca.
            await recarregarOrgs();
        } catch (falha) {
            setErro(falha instanceof Error ? falha.message : t("orgs.orgLoadFailed"));
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
    function selecionarLinha(org: OrgInList) {
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
    function acessarOrg(org: OrgInList) {
        if (ehDeOutroNo(org)) {
            setAviso({
                tom: "warn",
                titulo: t("orgs.noticeRemoteTitle"),
                texto: t("orgs.noticeRemoteText", {
                    name: org.name,
                    node: org.instancia ?? "",
                }),
            });
            return;
        }
        tentarNavegar(`/orgs/${org.id}`);
    }

    /** Exportar: baixa o arquivo da organização (sem conversas), o mesmo que a tela de operação baixa. */
    async function exportar(org: OrgInList) {
        setAviso(null);
        try {
            downloadFile(await exportOrg(org.id, false));
        } catch (falha) {
            const detalhe = backupFailure(falha, "exportar");
            setAviso({ tom: "danger", titulo: detalhe.titulo, texto: detalhe.texto });
        }
    }

    /**
     * Reativar uma organização inativa: `PATCH` com `If-Match` da versão lida.
     *
     * O `412` é tratado como o `ModalOrg` trata: nada se perde, a lista é relida e o operador vê
     * o conflito na hora, em vez de um erro genérico.
     */
    async function reativar(org: OrgInList) {
        setReativando(org.id);
        setAviso(null);
        try {
            await updateOrg(org.id, org.version, { status: "active" });
            await carregar();
        } catch (falha) {
            const disputa = versionConflict(falha);
            if (disputa) {
                setAviso({
                    tom: "warn",
                    titulo: t("orgs.formConflictTitle"),
                    texto:
                        typeof disputa.atual === "number"
                            ? t("orgs.noticeConflictWithVersion", {
                                  name: org.name,
                                  version: disputa.atual,
                              })
                            : t("orgs.noticeConflictPlain", { name: org.name }),
                });
                await carregar();
                return;
            }
            setAviso({
                tom: "danger",
                titulo: t("orgs.noticeReactivateTitle"),
                texto:
                    falha instanceof Error
                        ? falha.message
                        : t("orgs.noticeReactivateUnexpected"),
            });
        } finally {
            setReativando(null);
        }
    }

    function itensDoMenu(org: OrgInList): SofaMenuItemSpec[] {
        const remota = ehDeOutroNo(org);
        const itens: SofaMenuItemSpec[] = [
            {
                id: "detalhes",
                label: t("orgs.actionDetails"),
                icon: <IconEye size={14} aria-hidden="true" />,
                onSelect: () => setSelecionada(org.id),
            },
            {
                id: "editar",
                label: t("orgs.actionEdit"),
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => abrirEdicao(org),
            },
            {
                id: "exportar",
                label: t("orgs.actionExport"),
                icon: <IconDownload size={14} aria-hidden="true" />,
                onSelect: () => void exportar(org),
            },
        ];
        if (org.status !== "active") {
            // Inativa: a linha mostra "Reativar", então o acesso à organização fica aqui.
            itens.push({
                id: "acessar",
                label: t("orgs.actionAccessOrg"),
                icon: <IconPlay size={14} aria-hidden="true" />,
                disabled: remota,
                onSelect: () => acessarOrg(org),
            });
            itens.push({
                id: "reativar",
                label: t("orgs.actionReactivate"),
                icon: <IconPlay size={14} aria-hidden="true" />,
                disabled: remota,
                onSelect: () => void reativar(org),
            });
        }
        itens.push({
            id: "excluir",
            label: t("orgs.actionDelete"),
            icon: <IconTrash size={14} aria-hidden="true" />,
            danger: true,
            onSelect: () => setParaExcluir(org),
        });
        return itens;
    }

    const colunas = useMemo(
        () => [
            coluna.accessor("name", {
                header: t("orgs.columnName"),
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
                header: t("orgs.columnStatus"),
                size: 100,
                cell: (info) => (
                    <span className={`poph-status ${tomDaSituacao(info.getValue())}`}>
                        <span className="poph-status__ponto" aria-hidden="true" />
                        {rotuloDaSituacao(info.getValue())}
                    </span>
                ),
            }),
            coluna.accessor("instancia", {
                header: t("orgs.columnNode"),
                size: 130,
                cell: (info) => {
                    // Sem dado de cluster (token de org), a coluna não tem o que mostrar.
                    if (instancias === null) {
                        return (
                            <span
                                className="poph-orgs__celula--atenuada"
                                title={t("orgs.cellNodeUnknown")}
                            >
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
                            <span className="poph-mono">{id || t("orgs.nodeLocal")}</span>
                            {rotulo !== "" && <span className="poph-mono-mudo">{rotulo}</span>}
                        </div>
                    );
                },
            }),
            coluna.accessor("agents_active_count", {
                header: t("orgs.columnAgents"),
                size: 110,
                cell: (info) => {
                    const total = info.getValue();
                    if (typeof total !== "number") {
                        return (
                            <span
                                className="poph-orgs__celula--atenuada"
                                title={t("orgs.cellAgentsUnknown")}
                            >
                                —
                            </span>
                        );
                    }
                    const org = info.row.original;
                    return (
                        <span className={org.status === "active" ? undefined : "poph-orgs__celula--atenuada"}>
                            {total === 1
                                ? t("orgs.agentsCountOne")
                                : t("orgs.agentsCountMany", { count: total })}
                        </span>
                    );
                },
            }),
            coluna.display({
                id: "retention",
                header: t("orgs.columnRetention"),
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
                header: t("orgs.columnUpdated"),
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
                header: t("orgs.columnActions"),
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
                                    {t("orgs.actionReactivate")}
                                </SofaButton>
                            ) : (
                                <SofaButton
                                    variant={aberta ? "primary" : "subtle"}
                                    size="sm"
                                    disabled={ehDeOutroNo(org)}
                                    onClick={() => acessarOrg(org)}
                                >
                                    {t("orgs.actionAccess")}
                                </SofaButton>
                            )}
                            {podeGerenciar && (
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
        [navegar, podeGerenciar, selecionada, reativando, instancias, instanciaLocal, mapaDeInstancias, ehDeOutroNo, tentarNavegar, locale],
    );

    const filtrando = aba !== "todas" || filtro.trim() !== "";
    /** A gaveta mostra sempre a organização da seleção atual, relida da lista. */
    const orgSelecionada = selecionada === null ? null : (orgs.find((org) => org.id === selecionada) ?? null);

    const partesDaTelemetria: string[] = [];
    if (instancias) {
        const conectados = instancias.filter((i) => i.situacao === "local" || i.situacao === "ok").length;
        const inacessiveis = instancias.filter((i) => i.situacao === "inacessivel").length;
        partesDaTelemetria.push(
            t(conectados === 1 ? "orgs.telemetryNodesOne" : "orgs.telemetryNodesMany", {
                count: conectados,
            }),
        );
        if (inacessiveis > 0) {
            partesDaTelemetria.push(
                t(
                    inacessiveis === 1
                        ? "orgs.telemetryNodesOfflineOne"
                        : "orgs.telemetryNodesOfflineMany",
                    { count: inacessiveis },
                ),
            );
        }
    }
    if (telemetria?.situacao === "sem_resposta") {
        partesDaTelemetria.push(t("orgs.telemetryNoResponse"));
    }
    if (telemetria?.situacao === "degradada") {
        partesDaTelemetria.push(t("orgs.telemetryDegraded"));
    }
    if (telemetria?.uptimeSegundos !== undefined) {
        partesDaTelemetria.push(
            t("orgs.telemetryUptime", { value: formatarUptime(telemetria.uptimeSegundos) }),
        );
    }
    if (telemetria?.memoriaBytes !== undefined) {
        partesDaTelemetria.push(
            t("orgs.telemetryMemory", { value: formatarMemoria(telemetria.memoriaBytes) }),
        );
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
                            <h1 className="poph-pagina__titulo">{t("orgs.listTitle")}</h1>
                            <span className="poph-orgs__contagem">{orgs.length}</span>
                        </div>
                        <p className="poph-pagina__descricao">{t("orgs.tenantDescription")}</p>
                    </div>
                    <div className="poph-orgs__acoes-topo">
                        <SofaSearchField
                            label={t("orgs.listFilterLabel")}
                            value={filtro}
                            onValueChange={setFiltro}
                            placeholder={t("orgs.listFilterPlaceholder")}
                            size="md"
                            clearLabel={t("orgs.listFilterClear")}
                            className="poph-orgs__busca"
                        />
                        <SofaIconButton
                            label={t("orgs.listRefresh")}
                            size={32}
                            loading={carregando}
                            onClick={() => void carregar()}
                        >
                            <IconRefresh aria-hidden="true" />
                        </SofaIconButton>
                        {podeGerenciar && (
                            <SofaButton variant="primary" icon={<IconPlus aria-hidden="true" />} onClick={abrirCriacao}>
                                {t("orgs.listNew")}
                            </SofaButton>
                        )}
                    </div>
                </div>

                {aviso && (
                    <SofaBanner
                        tone={aviso.tom}
                        title={aviso.titulo}
                        dismissLabel={t("orgs.listDismissNotice")}
                        onDismiss={() => setAviso(null)}
                    >
                        {aviso.texto}
                    </SofaBanner>
                )}

                <div className="poph-orgs__barra">
                    <div
                        className="poph-orgs__abas"
                        role="group"
                        aria-label={t("orgs.tabsAriaLabel")}
                    >
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
                                    {t(ROTULO_DA_ABA[candidata])}
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
                        title={t("orgs.listLoadErrorTitle")}
                        description={t("orgs.listLoadErrorDescription")}
                        details={erro}
                        retry={{ label: t("orgs.failureTryAgain"), onClick: () => void carregar() }}
                    />
                ) : carregando && orgs.length === 0 ? (
                    <div
                        className="poph-orgs__esqueleto"
                        aria-busy="true"
                        aria-label={t("orgs.listLoading")}
                    >
                        {[0, 1, 2, 3, 4].map((linha) => (
                            <div key={linha} className="poph-orgs__esqueleto-linha" />
                        ))}
                    </div>
                ) : (
                    <SofaTable
                        className="poph-orgs__tabela"
                        label={t("orgs.listTitle")}
                        columns={colunas}
                        data={visiveis}
                        getRowId={(org) => org.id}
                        enableSelection
                        selectedIds={selecionada === null ? [] : [selecionada]}
                        onRowClick={selecionarLinha}
                        isLoading={carregando}
                        totalCount={visiveis.length}
                        footerMeta={() =>
                            orgs.length === 1
                                ? t("orgs.listFooterOne", {
                                      shown: visiveis.length,
                                      total: orgs.length,
                                  })
                                : t("orgs.listFooterMany", {
                                      shown: visiveis.length,
                                      total: orgs.length,
                                  })
                        }
                        footer={
                            <span className="poph-orgs__dica">{t("orgs.listKeyboardHint")}</span>
                        }
                        empty={
                            filtrando ? (
                                <SofaEmptyState
                                    title={t("orgs.listEmptyFilteredTitle")}
                                    description={t("orgs.listEmptyFilteredDescription")}
                                    action={
                                        <SofaButton
                                            variant="secondary"
                                            onClick={() => {
                                                setAba("todas");
                                                setFiltro("");
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
                                        podeGerenciar
                                            ? t("orgs.listEmptyManagerDescription")
                                            : t("orgs.listEmptyOrgTokenDescription")
                                    }
                                    action={
                                        podeGerenciar ? (
                                            <SofaButton
                                                variant="primary"
                                                icon={<IconPlus aria-hidden="true" />}
                                                onClick={abrirCriacao}
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
                        <div className="poph-orgs__card-cabeca">
                            <h2 id="poph-card-matrix" className="poph-orgs__card-titulo">
                                {t("orgs.cardMatrixTitle")}
                            </h2>
                            <span
                                className="poph-orgs__selo"
                                title={t("orgs.cardMatrixBadgeTitle")}
                            >
                                {t("orgs.inspectorUnavailable")}
                            </span>
                        </div>
                        <p className="poph-orgs__card-texto">{t("orgs.cardMatrixText")}</p>
                    </section>
                    <section className="poph-orgs__card" aria-labelledby="poph-card-isolamento">
                        <div className="poph-orgs__card-cabeca">
                            <h2 id="poph-card-isolamento" className="poph-orgs__card-titulo">
                                {t("orgs.cardTenantTitle")}
                            </h2>
                        </div>
                        <p className="poph-orgs__card-texto">{t("orgs.cardTenantText")}</p>
                    </section>
                    <section className="poph-orgs__card" aria-labelledby="poph-card-cli">
                        <div className="poph-orgs__card-cabeca">
                            <h2 id="poph-card-cli" className="poph-orgs__card-titulo">
                                {t("orgs.cardCliTitle")}
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
                        titulo={t("orgs.deleteOrgTitle")}
                        descricao={t("orgs.deleteOrgDescription", { name: paraExcluir.name })}
                        recurso={paraExcluir.slug}
                        rotuloRecurso={t("orgs.deleteOrgResourceLabel")}
                        rotuloConfirmar={t("orgs.deleteOrgConfirm")}
                        permiteForcar
                        rotuloForcar={t("orgs.deleteOrgForceLabel")}
                        consequencia={
                            <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                                {t("orgs.deleteOrgConsequence")}
                            </p>
                        }
                        aoConfirmar={(forcar) => deleteOrg(paraExcluir.id, forcar)}
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

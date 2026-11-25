/**
 * Inspetor lateral da organização no desenho da prancha 01b do FigCraft (CF-05).
 *
 * Gaveta de 380 px à direita, sem modal: abre quando a linha da lista é selecionada, fecha com
 * `Esc` e com o "✕", e devolve o foco à linha (o `Orgs` guarda de onde ele veio).
 *
 * Estrutura da prancha: cabeçalho com o tile do prédio, título, slug + versão de concorrência;
 * badges de situação, nó e id; caixa do comando `poph chat`; grid de propriedades; bloco dos
 * agentes vinculados; rodapé fixo com as ações.
 *
 * **Duas** consultas ao abrir, uma por recurso e independentes — o resto da gaveta aparece na
 * hora: `GET …/agents` (bloco dos agentes) e `GET …/matrix` (estado real do Espelho Matrix). Cada
 * uma tem prazo próprio: `carregando` sempre termina em `pronto` ou `erro`, mesmo que a resposta
 * nunca chegue, e uma falha não derruba a outra nem a gaveta.
 *
 * O que o desenho mostra e a API **não** entrega fica de fora, em vez de inventado: latência do
 * agente, canal/espaço do Matrix dentro do estado (a rota de estado não traz o espaço) e o
 * endereço do nó. O comando `poph chat --org <slug> <agente>` foi conferido no binário (`--org` é
 * opção global e `chat` exige o agente como argumento posicional).
 */

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
import { listarAgentes, type Agente } from "../api/agentes";
import type { Org } from "../api/cliente";
import { lerEstadoMatrix, type EstadoMatrix } from "../api/matrix";
import { translate, useI18n } from "../i18n";
import { formatDate, formatTime } from "../i18n/format";
import "../estilos/inspetor.css";

/**
 * Nó onde a organização mora, já resolvido pela lista (`?cluster=true`). `id` vazio é a
 * instalação de nó único; `situacao` é `local`, `ok` (nó vizinho que respondeu) ou `inacessivel`.
 */
export type NoDaOrg = { id: string; situacao: string };

export type InspetorOrgProps = {
    /** Organização selecionada na lista; o inspetor mostra sempre esta. */
    org: Org;
    /** Nó da organização; `null` quando a resposta não traz o estado do cluster (token de org). */
    no?: NoDaOrg | null;
    /** `false` com token de organização: o rodapé mostra só "Acessar Organização →". */
    podeGerenciar?: boolean;
    /** Organização em outro nó: acesso e edição ficam desabilitados, como na lista. */
    remota?: boolean;
    /** Fecha a gaveta (o dono da tela devolve o foco à linha). */
    aoFechar: () => void;
    /** Abre o painel da organização; passa pela guarda de alterações não salvas. */
    aoAcessar: (org: Org) => void;
    /** Abre o formulário de edição da organização (`ModalOrg`). */
    aoEditar?: (org: Org) => void;
    /** Exporta a organização sem conversas (mesmo caminho da lista). */
    aoExportar?: (org: Org) => void;
    /** Abre o diálogo de exclusão com confirmação por slug. */
    aoExcluir?: (org: Org) => void;
};

/**
 * Prazo de uma leitura da gaveta. A API local responde em milissegundos; se ela não responder, a
 * gaveta diz que não conseguiu em vez de ficar em "carregando…" para sempre.
 */
const PRAZO_DA_LEITURA_MS = 12_000;

/** Quantos agentes aparecem como card; o resto fica no link "Ver todos". */
const AGENTES_NO_BLOCO = 8;

type Recurso<T> =
    | { estado: "carregando" }
    | { estado: "pronto"; dados: T }
    | { estado: "erro"; mensagem: string };

/**
 * Lê uma vez por `chave` (a org aberta) e garante desfecho: `pronto`, `erro` ou o prazo estourado.
 * Resultado que chega depois do prazo, ou depois de trocar de organização, é descartado.
 */
function useLeitura<T>(chave: string, ler: () => Promise<T>): Recurso<T> {
    const [recurso, setRecurso] = useState<Recurso<T>>({ estado: "carregando" });
    // A leitura mais recente, sem entrar nas dependências do efeito (as dependências são a chave).
    const lerAgora = useRef(ler);
    lerAgora.current = ler;

    useEffect(() => {
        let encerrado = false;
        setRecurso({ estado: "carregando" });
        const prazo = setTimeout(() => {
            if (encerrado) return;
            encerrado = true;
            setRecurso({ estado: "erro", mensagem: translate("orgs.inspectorBacklogTimeout") });
        }, PRAZO_DA_LEITURA_MS);
        lerAgora
            .current()
            .then((dados) => {
                if (encerrado) return;
                encerrado = true;
                clearTimeout(prazo);
                setRecurso({ estado: "pronto", dados });
            })
            .catch((falha: unknown) => {
                if (encerrado) return;
                encerrado = true;
                clearTimeout(prazo);
                setRecurso({ estado: "erro", mensagem: mensagemDoErro(falha) });
            });
        return () => {
            encerrado = true;
            clearTimeout(prazo);
        };
    }, [chave]);

    return recurso;
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

function tomDaSituacao(situacao: string): string {
    return situacao === "active" ? "poph-status--ativo" : "poph-status--inativo";
}

/** Situação do agente em caixa de frase, para o card ("Ativo", "Inativo"). */
function rotuloDoAgente(situacao: string): string {
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

/** Retenção vem da configuração da org, que é JSON livre (mesma regra da lista). */
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

/** Data e hora completas em pt-BR, como no desenho ("23/11/2026 às 12:08"). */
function dataEHora(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    // A data e a hora saem dos formatadores do locale ativo; o separador vem do catálogo.
    return translate("orgs.dateTimeAt", { date: formatDate(ms), time: formatTime(ms) });
}

/** Falha de consulta em texto curto, para o bloco que falhou. */
function mensagemDoErro(falha: unknown): string {
    if (falha instanceof Error && falha.message) return falha.message;
    return translate("orgs.inspectorQueryFailed");
}

/** Nó em texto curto para o chip e para a propriedade "Instância alocada". */
function textoDoNo(no: NoDaOrg | null | undefined): { chip: string; valor: string } | null {
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

/** Estado do Espelho Matrix em valor e tooltip, sem inventar: cada ramo é um campo da rota. */
function espelhoMatrix(estado: EstadoMatrix): { valor: string; detalhe: string } {
    const pendentes = estado.backlog?.messages ?? 0;
    const idade = estado.backlog?.oldest_seconds ?? 0;
    const partes: string[] = [];
    if (pendentes > 0) {
        partes.push(
            translate(pendentes === 1 ? "orgs.matrixBacklogOne" : "orgs.matrixBacklogMany", {
                count: pendentes,
            }),
        );
        if (idade > 0) partes.push(translate("orgs.matrixOldest", { seconds: idade }));
    }
    if (estado.pending_failures > 0) {
        partes.push(
            translate(
                estado.pending_failures === 1 ? "orgs.matrixFailureOne" : "orgs.matrixFailureMany",
                { count: estado.pending_failures },
            ),
        );
    }
    if (estado.last_failure) {
        partes.push(translate("orgs.matrixLastFailure", { reason: estado.last_failure }));
    }
    const detalhe =
        partes.length > 0 ? partes.join(" · ") : translate("orgs.matrixNothingPending");

    if (estado.connected === null || estado.connected === undefined) {
        return {
            valor: translate("orgs.matrixNotConfigured"),
            detalhe: translate("orgs.matrixNotConfiguredDetail"),
        };
    }
    if (!estado.enabled) {
        return {
            valor: translate("orgs.matrixDisabled"),
            detalhe: translate("orgs.matrixDisabledDetail", { detail: detalhe }),
        };
    }
    if (estado.connected) {
        return { valor: translate("orgs.matrixConnected"), detalhe };
    }
    return {
        valor: translate("orgs.matrixDisconnected"),
        detalhe: translate("orgs.matrixDisconnectedDetail", { detail: detalhe }),
    };
}

/** Texto do agente no card: modelo (ou o padrão do servidor) e situação. */
function metaDoAgente(agente: Agente): string {
    const modelo = agente.model?.trim()
        ? agente.model.trim()
        : translate("orgs.agentModelDefault");
    return `${modelo} · ${rotuloDoAgente(agente.status)}`;
}

export function InspetorOrg({
    org,
    no = null,
    podeGerenciar = true,
    remota = false,
    aoFechar,
    aoAcessar,
    aoEditar,
    aoExportar,
    aoExcluir,
}: InspetorOrgProps) {
    const { translate: t } = useI18n();
    const gaveta = useRef<HTMLElement | null>(null);
    const [copiado, setCopiado] = useState(false);
    /** Falha do clipboard, quando não há como copiar (degradação honesta). */
    const [falhaAoCopiar, setFalhaAoCopiar] = useState<string | null>(null);
    const relogioDoCopiado = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Uma consulta por recurso, independentes: a falha de uma não apaga a outra.
    const agentes = useLeitura(`agentes:${org.id}`, useCallback(() => listarAgentes(org.id), [org.id]));
    const matrix = useLeitura(`matrix:${org.id}`, useCallback(() => lerEstadoMatrix(org.id), [org.id]));

    // O foco entra na gaveta quando ela abre (a lista devolve ele para a linha ao fechar).
    useEffect(() => {
        gaveta.current?.focus();
    }, []);

    const fechar = useCallback(() => aoFechar(), [aoFechar]);

    // `Esc` fecha a gaveta, de onde o foco estiver. Menus e diálogos abertos consomem o `Esc`
    // primeiro (o deles fecha antes); se um deles já tratou a tecla, aqui não se faz nada.
    useEffect(() => {
        const aoTeclar = (evento: KeyboardEvent) => {
            if (evento.key !== "Escape" || evento.defaultPrevented) return;
            const alvo = evento.target as HTMLElement | null;
            if (alvo?.closest?.('[role="menu"], [role="dialog"], [role="listbox"]')) return;
            evento.preventDefault();
            fechar();
        };
        window.addEventListener("keydown", aoTeclar);
        return () => window.removeEventListener("keydown", aoTeclar);
    }, [fechar]);

    // A seleção mudou: o "Copiado" pertence à organização anterior.
    useEffect(() => {
        setCopiado(false);
        setFalhaAoCopiar(null);
    }, [org.id]);

    useEffect(
        () => () => {
            if (relogioDoCopiado.current !== null) clearTimeout(relogioDoCopiado.current);
        },
        [],
    );

    const listaDeAgentes = agentes.estado === "pronto" ? agentes.dados : [];
    const primeiroAtivo = listaDeAgentes.find((agente) => agente.status === "active") ?? null;
    const comando = `poph chat --org ${org.slug} ${primeiroAtivo?.slug ?? "<agente>"}`;
    const textoNo = textoDoNo(no);
    const idCurto = org.id.length > 8 ? `${org.id.slice(0, 8)}…` : org.id;
    const agentesNoBloco = listaDeAgentes.slice(0, AGENTES_NO_BLOCO);

    async function copiar() {
        setFalhaAoCopiar(null);
        try {
            if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
                throw new Error("clipboard indisponível");
            }
            await navigator.clipboard.writeText(comando);
        } catch {
            setCopiado(false);
            setFalhaAoCopiar(t("orgs.inspectorCopyFailed"));
            return;
        }
        setCopiado(true);
        if (relogioDoCopiado.current !== null) clearTimeout(relogioDoCopiado.current);
        relogioDoCopiado.current = setTimeout(() => setCopiado(false), 4000);
    }

    return (
        <aside
            ref={gaveta}
            className="poph-gaveta poph-inspetor"
            role="complementary"
            aria-label={t("orgs.inspectorAriaLabel", { name: org.name })}
            tabIndex={-1}
        >
            <header className="poph-inspetor__topo">
                <div className="poph-inspetor__topo-linha">
                    <span className="poph-inspetor__tile" aria-hidden="true">
                        <IconBuilding size={14} aria-hidden="true" />
                    </span>
                    <h2 className="poph-inspetor__titulo">{org.name}</h2>
                    <SofaIconButton label={t("orgs.inspectorClose")} size={24} onClick={fechar}>
                        <IconClose aria-hidden="true" />
                    </SofaIconButton>
                </div>
                <p className="poph-inspetor__subtitulo">
                    {t("orgs.inspectorSubtitle", { slug: org.slug, version: org.version })}
                </p>
            </header>

            <div className="poph-inspetor__corpo">
                <div className="poph-inspetor__badges">
                    <span className={`poph-status ${tomDaSituacao(org.status)}`}>
                        <span className="poph-status__ponto" aria-hidden="true" />
                        {rotuloDaSituacao(org.status)}
                    </span>
                    {textoNo && (
                        <span
                            className="poph-inspetor__chip"
                            title={textoNo.chip === textoNo.valor ? undefined : textoNo.valor}
                        >
                            {textoNo.chip}
                        </span>
                    )}
                    <span className="poph-inspetor__chip poph-inspetor__chip--mono" title={org.id}>
                        ID: {idCurto}
                    </span>
                </div>

                <section className="poph-inspetor__secao" aria-label={t("orgs.inspectorCommandAria")}>
                    <div className="poph-terminal">
                        <code className="poph-terminal__comando">{comando}</code>
                        <SofaIconButton
                            label={t("orgs.inspectorCopyCommand")}
                            size={24}
                            className="poph-terminal__copiar"
                            onClick={() => void copiar()}
                        >
                            <IconCopy size={13} aria-hidden="true" />
                        </SofaIconButton>
                    </div>
                    <p
                        className={[
                            "poph-inspetor__aviso",
                            copiado || falhaAoCopiar ? "poph-inspetor__aviso--visivel" : "",
                            falhaAoCopiar ? "poph-inspetor__aviso--erro" : "",
                        ]
                            .filter(Boolean)
                            .join(" ")}
                        role="status"
                        aria-live="polite"
                    >
                        {copiado ? (
                            <>
                                <IconCheck aria-hidden="true" /> {t("orgs.inspectorCopied")}
                            </>
                        ) : (
                            falhaAoCopiar ?? ""
                        )}
                    </p>
                </section>

                <section
                    className="poph-inspetor__secao poph-inspetor__secao--propriedades"
                    aria-label={t("orgs.inspectorPropertiesAria")}
                >
                    <dl className="poph-propriedades">
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">{t("orgs.inspectorRetention")}</dt>
                            <dd className="poph-propriedade__valor">{retencao(org)}</dd>
                        </div>
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">{t("orgs.inspectorMatrix")}</dt>
                            <dd className="poph-propriedade__valor">
                                {matrix.estado === "carregando" && (
                                    <span className="poph-inspetor__recurso">
                                        {t("orgs.inspectorLoading")}
                                    </span>
                                )}
                                {matrix.estado === "erro" && (
                                    <span
                                        className="poph-inspetor__recurso poph-inspetor__recurso--erro"
                                        title={matrix.mensagem}
                                    >
                                        {t("orgs.inspectorUnavailable")}
                                    </span>
                                )}
                                {matrix.estado === "pronto" &&
                                    (() => {
                                        const espelho = espelhoMatrix(matrix.dados);
                                        return (
                                            <span className="poph-inspetor__com-dica" title={espelho.detalhe}>
                                                {espelho.valor}
                                            </span>
                                        );
                                    })()}
                            </dd>
                        </div>
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">{t("orgs.inspectorInstance")}</dt>
                            <dd className="poph-propriedade__valor">
                                {textoNo ? (
                                    <span className="poph-propriedade__valor--mono">{textoNo.valor}</span>
                                ) : (
                                    <span
                                        className="poph-inspetor__recurso"
                                        title={t("orgs.inspectorInstanceUnknown")}
                                    >
                                        —
                                    </span>
                                )}
                            </dd>
                        </div>
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">{t("orgs.inspectorUpdatedAt")}</dt>
                            <dd className="poph-propriedade__valor">{dataEHora(org.updated_at)}</dd>
                        </div>
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">{t("orgs.inspectorCreatedAt")}</dt>
                            <dd className="poph-propriedade__valor">{dataEHora(org.created_at)}</dd>
                        </div>
                    </dl>
                </section>

                <section className="poph-inspetor__secao" aria-labelledby="poph-inspetor-agentes">
                    <h3 id="poph-inspetor-agentes" className="poph-inspetor__secao-titulo">
                        {t("orgs.inspectorAgentsTitle")}
                        {agentes.estado === "pronto" && listaDeAgentes.length > 0 && (
                            <span className="poph-inspetor__contagem"> ({listaDeAgentes.length})</span>
                        )}
                    </h3>
                    {agentes.estado === "carregando" && (
                        <p className="poph-inspetor__recurso">{t("orgs.inspectorAgentsLoading")}</p>
                    )}
                    {agentes.estado === "erro" && (
                        <p
                            className="poph-inspetor__recurso poph-inspetor__recurso--erro"
                            title={agentes.mensagem}
                        >
                            {t("orgs.inspectorAgentsError")}
                        </p>
                    )}
                    {agentes.estado === "pronto" && listaDeAgentes.length === 0 && (
                        <p className="poph-inspetor__recurso">{t("orgs.inspectorAgentsEmpty")}</p>
                    )}
                    {agentes.estado === "pronto" && listaDeAgentes.length > 0 && (
                        <>
                            <ul className="poph-inspetor__agentes">
                                {agentesNoBloco.map((agente) => {
                                    const conteudo = (
                                        <>
                                            <span className="poph-inspetor__agente-tile" aria-hidden="true">
                                                <IconBot size={14} aria-hidden="true" />
                                            </span>
                                            <span className="poph-inspetor__agente-texto">
                                                <span className="poph-inspetor__agente-nome">{agente.name}</span>
                                                <span className="poph-inspetor__agente-meta">
                                                    {metaDoAgente(agente)}
                                                </span>
                                            </span>
                                            <IconChevronRight size={14} aria-hidden="true" />
                                        </>
                                    );
                                    return (
                                        <li key={agente.id}>
                                            {remota ? (
                                                <div
                                                    className="poph-inspetor__agente poph-inspetor__agente--estatico"
                                                    title={t("orgs.inspectorAgentRemote")}
                                                >
                                                    {conteudo}
                                                </div>
                                            ) : (
                                                <Link
                                                    className="poph-inspetor__agente"
                                                    to={`/orgs/${org.id}/agents/${agente.id}`}
                                                >
                                                    {conteudo}
                                                </Link>
                                            )}
                                        </li>
                                    );
                                })}
                            </ul>
                            {listaDeAgentes.length > agentesNoBloco.length && (
                                <Link className="poph-inspetor__ver-todos" to={`/orgs/${org.id}/agents`}>
                                    {t("orgs.inspectorSeeAll", { count: listaDeAgentes.length })}
                                </Link>
                            )}
                        </>
                    )}
                </section>
            </div>

            <footer className="poph-inspetor__rodape">
                <SofaButton
                    variant="primary"
                    fullWidth
                    disabled={remota}
                    title={remota ? t("orgs.inspectorRemoteTitle") : undefined}
                    onClick={() => aoAcessar(org)}
                >
                    {t("orgs.inspectorAccess")}
                </SofaButton>
                {podeGerenciar && (
                    <div className="poph-inspetor__rodape-secundarias">
                        <SofaButton
                            variant="secondary"
                            size="sm"
                            disabled={remota || aoEditar === undefined}
                            onClick={() => aoEditar?.(org)}
                        >
                            {t("orgs.inspectorEdit")}
                        </SofaButton>
                        <SofaButton
                            variant="secondary"
                            size="sm"
                            disabled={aoExportar === undefined}
                            onClick={() => aoExportar?.(org)}
                        >
                            {t("orgs.inspectorExport")}
                        </SofaButton>
                        <SofaIconButton
                            label={t("orgs.inspectorDeleteLabel", { name: org.name })}
                            variant="danger"
                            size={32}
                            disabled={remota || aoExcluir === undefined}
                            onClick={() => aoExcluir?.(org)}
                        >
                            <IconTrash size={14} aria-hidden="true" />
                        </SofaIconButton>
                    </div>
                )}
            </footer>
        </aside>
    );
}

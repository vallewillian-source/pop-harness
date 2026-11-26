/**
 * Módulo Matrix da organização (MX-25, rodada 1 de 2): o espelho das conversas no Element, visto e
 * operado pelo Console, na forma do desenho aprovado (MX-24, pranchas D1 a D6).
 *
 * Da cima para baixo:
 * - **Cabeçalho:** Atualizar e ligar/desligar o módulo (com confirmação e `If-Match`).
 * - **Avisos de estado:** módulo desligado (sem backfill) ou Synapse fora (espelhamento parado).
 * - **Faixa de indicadores:** módulo na org, servidor de chat, atraso do cursor, falhas isoladas e
 *   última falha. O corte do backfill aparece na célula do módulo.
 * - **Espaços por departamento:** tabela com todo departamento ativo. Sem espaço, a linha mostra
 *   "Criar auto" e "Ligar existente" (este abre um diálogo). Desligar pede confirmação.
 *
 * Nenhum dado de conversa ou de cliente aparece aqui: a tela mostra só o que a API devolve.
 * `401` encerra a sessão pelo cliente (igual às outras telas); `403` e demais erros viram texto.
 */

import { useCallback, useEffect, useId, useRef, useState, type FormEvent } from "react";
import { useParams } from "react-router-dom";
import {
    IconRefresh,
    SofaBanner,
    SofaButton,
    SofaConfirmDialog,
    SofaEmptyState,
    SofaErrorState,
    SofaField,
    SofaInput,
    SofaModal,
    SofaSkeleton,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { translate, useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import { formatDateTime, formatNumber } from "../i18n/format";
import { listStructure, type Departamento } from "../api/structure";
import {
    updateMatrixModule,
    unlinkMatrixSpace,
    readMatrixState,
    linkMatrixSpace,
    listMatrixSpaces,
    stateVersion,
    type MatrixState,
    type Espaco,
} from "../api/matrix";
import { formatarTempoNoAr, mensagemDeFalha } from "./Operacao";
import "../estilos/matrix.css";

/** Mensagem da tela quando o `If-Match` não bate: o estado mudou entre a leitura e a gravação. */
export function conflictMessage(): string {
    return translate("matrix.conflict");
}

type TextoFonte =
    | { key: TranslationKey }
    | { erro: unknown; what: TranslationKey; espaco?: true };

function textoDoAviso(fonte: TextoFonte): string {
    if ("key" in fonte) return translate(fonte.key);
    const what = translate(fonte.what);
    return fonte.espaco ? mensagemDoEspaco(fonte.erro, what) : mensagemDeFalha(fonte.erro, what);
}

type Carga<T> = { carregando: boolean; dados: T | null; erro: TextoFonte | null };
type Aviso = { tom: "ok" | "erro"; fonte: TextoFonte } | null;

/** Linha da tabela: um departamento (ativo, ou arquivado com espaço ainda gravado). */
export type LinhaDeEspaco = {
    id: string;
    nome: string;
    espaco: Espaco | null;
    arquivado: boolean;
};

const cargaInicial = <T,>(): Carga<T> => ({ carregando: true, dados: null, erro: null });

/** Conexão com o servidor de chat. `null` = módulo não configurado nesta instância. */
export function situacaoDaConexao(conectado: boolean | null | undefined): { valor: string; detalhe?: string } {
    if (conectado === true) return { valor: translate("matrix.connectionConnected") };
    if (conectado === false) {
        return {
            valor: translate("matrix.connectionDisconnected"),
            detalhe: translate("matrix.connectionDisconnectedDetail"),
        };
    }
    return {
        valor: translate("matrix.connectionUnknown"),
        detalhe: translate("matrix.connectionUnknownDetail"),
    };
}

/** Atraso do cursor: quantas mensagens faltam espelhar, em abreviação de painel (`0 msgs atrás`). */
export function valorDoAtraso(mensagens: number): string {
    return translate(mensagens === 1 ? "matrix.backlogMessageOne" : "matrix.backlogMessageMany", {
        count: formatNumber(mensagens),
    });
}

/** Falhas isoladas (mensagens que o módulo não conseguiu espelhar). */
export function valorDasFalhas(quantidade: number): string {
    return translate(quantidade === 1 ? "matrix.failuresPendingOne" : "matrix.failuresPendingMany", {
        count: formatNumber(quantidade),
    });
}

/** Origem do espaço, com os rótulos do desenho: criado pelo módulo ou vinculado ao Element. */
export function textoDaOrigem(origem: string): string {
    if (origem === "auto") return translate("matrix.originAuto");
    if (origem === "linked") return translate("matrix.originLinked");
    return origem;
}

/**
 * Linhas da tabela: todo departamento ativo (com ou sem espaço) e, depois, os espaços de
 * departamentos que já não estão ativos (aparecem marcados como arquivados, para não sumirem).
 */
export function montarLinhas(departamentos: Departamento[] | null, espacos: Espaco[]): LinhaDeEspaco[] {
    const ativos = (departamentos ?? []).filter((departamento) => departamento.status === "active");
    const linhas: LinhaDeEspaco[] = ativos.map((departamento) => ({
        id: departamento.id,
        nome: departamento.name,
        espaco: espacos.find((espaco) => espaco.department_id === departamento.id) ?? null,
        arquivado: false,
    }));
    for (const espaco of espacos) {
        if (ativos.some((departamento) => departamento.id === espaco.department_id)) continue;
        linhas.push({
            id: espaco.department_id,
            nome: espaco.department_name ?? translate("matrix.departmentUnnamed"),
            espaco,
            arquivado: departamentos !== null,
        });
    }
    return linhas;
}

/**
 * Frase do erro de uma ação sobre espaço. `400` e `409` trazem a instrução do que fazer no Element
 * (convidar o bot, dar poder), então a mensagem do servidor segue como ela veio; o resto passa pela
 * tradução comum das telas.
 */
export function mensagemDoEspaco(erro: unknown, oQue: string): string {
    if (erro instanceof ApiError && (erro.status === 400 || erro.status === 409) && erro.message) {
        return erro.message;
    }
    return mensagemDeFalha(erro, oQue);
}

function dataDoCorte(ms: number): string {
    return formatDateTime(new Date(ms));
}

function Indicador({
    rotulo,
    valor,
    detalhe,
    tom,
}: {
    rotulo: string;
    valor: string;
    detalhe?: string;
    tom?: "ok" | "alerta" | "erro";
}) {
    return (
        <div className="poph-matrix__indicador">
            <dt className="poph-matrix__indicador-rotulo">{rotulo}</dt>
            <dd className="poph-matrix__indicador-corpo">
                <span
                    className={
                        tom ? `poph-matrix__indicador-valor poph-matrix__indicador-valor--${tom}` : "poph-matrix__indicador-valor"
                    }
                >
                    {valor}
                </span>
                {detalhe ? <span className="poph-matrix__indicador-detalhe">{detalhe}</span> : null}
            </dd>
        </div>
    );
}

export function Matrix() {
    const { orgId = "" } = useParams();
    const { translate: t } = useI18n();

    const [modulo, setModulo] = useState<Carga<MatrixState>>(cargaInicial);
    const [espacos, setEspacos] = useState<Carga<Espaco[]>>(cargaInicial);
    const [departamentos, setDepartamentos] = useState<Carga<Departamento[]>>(cargaInicial);

    const [enviandoModulo, setEnviandoModulo] = useState(false);
    const [avisoModulo, setAvisoModulo] = useState<Aviso>(null);
    const [confirmandoDesligarModulo, setConfirmandoDesligarModulo] = useState(false);

    const [criandoPara, setCriandoPara] = useState<string | null>(null);
    const [avisoEspaco, setAvisoEspaco] = useState<Aviso>(null);
    const [espacoParaDesligar, setEspacoParaDesligar] = useState<Espaco | null>(null);
    const [desligandoEspaco, setDesligandoEspaco] = useState(false);

    /** Departamento do diálogo "Ligar existente"; `null` = diálogo fechado. */
    const [ligandoA, setLigandoA] = useState<{ id: string; nome: string } | null>(null);
    const [espacoDigitado, setEspacoDigitado] = useState("");
    const [enviandoLigacao, setEnviandoLigacao] = useState(false);
    const [erroDoDialogo, setErroDoDialogo] = useState<TextoFonte | null>(null);

    const vivo = useRef(true);
    const emCurso = useRef(false);
    const idModulo = useId();
    const idEspacos = useId();
    const idFormLigacao = useId();
    const idEspacoExistente = useId();

    useEffect(() => {
        vivo.current = true;
        return () => {
            vivo.current = false;
        };
    }, []);

    const carregar = useCallback(async () => {
        if (emCurso.current) return;
        emCurso.current = true;
        setModulo((atual) => ({ ...atual, carregando: true }));
        setEspacos((atual) => ({ ...atual, carregando: true }));
        setDepartamentos((atual) => ({ ...atual, carregando: true }));

        const [resModulo, resEspacos, resDepartamentos] = await Promise.allSettled([
            readMatrixState(orgId),
            listMatrixSpaces(orgId),
            listStructure("departamento", orgId),
        ]);
        emCurso.current = false;
        if (!vivo.current) return;

        if (resModulo.status === "fulfilled") {
            setModulo({ carregando: false, dados: resModulo.value, erro: null });
        } else {
            setModulo((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: { erro: resModulo.reason, what: "operation.whatMatrixModule" },
            }));
        }

        if (resEspacos.status === "fulfilled") {
            setEspacos({ carregando: false, dados: resEspacos.value, erro: null });
        } else {
            setEspacos((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: { erro: resEspacos.reason, what: "operation.whatSpaces" },
            }));
        }

        if (resDepartamentos.status === "fulfilled") {
            setDepartamentos({
                carregando: false,
                dados: resDepartamentos.value as Departamento[],
                erro: null,
            });
        } else {
            setDepartamentos((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: { erro: resDepartamentos.reason, what: "operation.whatDepartments" },
            }));
        }
    }, [orgId]);

    useEffect(() => {
        void carregar();
    }, [carregar]);

    /** Relê só a lista de espaços (depois de criar ou desligar), sem trocar a tela para carregando. */
    const recarregarEspacos = useCallback(async () => {
        try {
            const lista = await listMatrixSpaces(orgId);
            if (vivo.current) setEspacos({ carregando: false, dados: lista, erro: null });
        } catch (erro) {
            if (vivo.current) {
                setEspacos((atual) => ({
                    ...atual,
                    erro: { erro, what: "operation.whatSpaces" },
                }));
            }
        }
    }, [orgId]);

    async function alternarModulo(ligado: boolean) {
        const atual = modulo.dados;
        if (!atual || enviandoModulo) return;
        setEnviandoModulo(true);
        setAvisoModulo(null);
        try {
            const novo = await updateMatrixModule(orgId, ligado, stateVersion(atual));
            if (!vivo.current) return;
            setModulo({ carregando: false, dados: novo, erro: null });
            setAvisoModulo({
                tom: "ok",
                fonte: {
                    key: ligado ? "matrix.moduleEnabledNotice" : "matrix.moduleDisabledNotice",
                },
            });
        } catch (erro) {
            if (!vivo.current) return;
            if (erro instanceof ApiError && erro.status === 412) {
                setAvisoModulo({ tom: "erro", fonte: { key: "matrix.conflict" } });
                void carregar();
            } else {
                setAvisoModulo({
                    tom: "erro",
                    fonte: { erro, what: "operation.whatMatrixModule" },
                });
            }
        } finally {
            if (vivo.current) setEnviandoModulo(false);
        }
    }

    async function criarEspaco(linha: LinhaDeEspaco) {
        if (criandoPara !== null) return;
        setCriandoPara(linha.id);
        setAvisoEspaco(null);
        try {
            await linkMatrixSpace(orgId, { department_id: linha.id, space: null });
            if (!vivo.current) return;
            setAvisoEspaco({ tom: "ok", fonte: { key: "matrix.spaceCreated" } });
            await recarregarEspacos();
        } catch (erro) {
            if (vivo.current) {
                setAvisoEspaco({
                    tom: "erro",
                    fonte: { erro, what: "operation.whatDepartment", espaco: true },
                });
            }
        } finally {
            if (vivo.current) setCriandoPara(null);
        }
    }

    function abrirLigacao(linha: LinhaDeEspaco) {
        setLigandoA({ id: linha.id, nome: linha.nome });
        setEspacoDigitado("");
        setErroDoDialogo(null);
    }

    function fecharLigacao() {
        if (enviandoLigacao) return;
        setLigandoA(null);
        setEspacoDigitado("");
        setErroDoDialogo(null);
    }

    async function ligarExistente(evento: FormEvent) {
        evento.preventDefault();
        const espaco = espacoDigitado.trim();
        if (!ligandoA || !espaco || enviandoLigacao) return;
        setEnviandoLigacao(true);
        setErroDoDialogo(null);
        try {
            await linkMatrixSpace(orgId, { department_id: ligandoA.id, space: espaco });
            if (!vivo.current) return;
            setLigandoA(null);
            setEspacoDigitado("");
            setAvisoEspaco({ tom: "ok", fonte: { key: "matrix.spaceLinked" } });
            await recarregarEspacos();
        } catch (erro) {
            if (vivo.current) {
                setErroDoDialogo({ erro, what: "operation.whatDepartment", espaco: true });
            }
        } finally {
            if (vivo.current) setEnviandoLigacao(false);
        }
    }

    async function desligarEspaco(espaco: Espaco) {
        if (desligandoEspaco) return;
        setDesligandoEspaco(true);
        setAvisoEspaco(null);
        try {
            await unlinkMatrixSpace(orgId, espaco.department_id);
            if (!vivo.current) return;
            setEspacoParaDesligar(null);
            setAvisoEspaco({
                tom: "ok",
                fonte: { key: "matrix.spaceDisconnected" },
            });
            await recarregarEspacos();
        } catch (erro) {
            if (!vivo.current) return;
            setEspacoParaDesligar(null);
            setAvisoEspaco({
                tom: "erro",
                fonte: { erro, what: "operation.whatSpace", espaco: true },
            });
        } finally {
            if (vivo.current) setDesligandoEspaco(false);
        }
    }

    const m = modulo.dados;
    const lista = espacos.dados;
    const deptos = departamentos.dados;
    const linhas = lista ? montarLinhas(deptos, lista) : [];
    const carregandoAlgum = modulo.carregando || espacos.carregando || departamentos.carregando;
    const nomeDoEspaco = (espaco: Espaco) => espaco.department_name ?? t("matrix.departmentUnnamed");
    const conexao = situacaoDaConexao(m?.connected);
    const espelhando = m?.enabled === true;

    return (
        <div className="poph-pagina poph-matrix">
            <div className="poph-pagina__topo">
                <div>
                    <h1 className="poph-pagina__titulo">{t("matrix.pageTitle")}</h1>
                    <p className="poph-pagina__descricao">{t("matrix.pageDescription")}</p>
                </div>
                <div className="poph-pagina__acoes">
                    <SofaButton
                        variant="subtle"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void carregar()}
                        disabled={carregandoAlgum}
                    >
                        {carregandoAlgum ? t("matrix.refreshing") : t("matrix.refresh")}
                    </SofaButton>
                    {enviandoModulo ? (
                        <span className="poph-matrix__status" role="status">{t("matrix.saving")}</span>
                    ) : null}
                    {m ? (
                        espelhando ? (
                            <SofaButton
                                variant="danger"
                                disabled={enviandoModulo}
                                onClick={() => setConfirmandoDesligarModulo(true)}
                            >
                                {t("matrix.disableModule")}
                            </SofaButton>
                        ) : (
                            <SofaButton
                                variant="primary"
                                loading={enviandoModulo}
                                disabled={enviandoModulo}
                                onClick={() => void alternarModulo(true)}
                            >
                                {t("matrix.enableModule")}
                            </SofaButton>
                        )
                    ) : null}
                </div>
            </div>

            <section className="poph-secao poph-matrix__resumo" aria-labelledby={idModulo}>
                <h2 id={idModulo} className="poph-matrix__oculto">{t("matrix.moduleSectionLabel")}</h2>
                {modulo.carregando && !m ? (
                    <dl className="poph-matrix__indicadores" aria-busy="true">
                        {[
                            t("matrix.indicatorModule"),
                            t("matrix.indicatorSynapse"),
                            t("matrix.indicatorBacklog"),
                            t("matrix.indicatorFailures"),
                            t("matrix.indicatorLastFailure"),
                        ].map(
                            (rotulo) => (
                                <div className="poph-matrix__indicador" key={rotulo}>
                                    <dt className="poph-matrix__indicador-rotulo">{rotulo}</dt>
                                    <dd className="poph-matrix__indicador-corpo">
                                        <SofaSkeleton lines={2} />
                                    </dd>
                                </div>
                            ),
                        )}
                    </dl>
                ) : modulo.erro && !m ? (
                    <SofaErrorState
                        title={t("matrix.moduleLoadFailed")}
                        description={textoDoAviso(modulo.erro)}
                        retry={{ label: t("operation.retry"), onClick: () => void carregar() }}
                    />
                ) : m ? (
                    <>
                        {modulo.erro ? (
                            <p className="poph-matrix__aviso" role="alert">
                                {t("matrix.lastUpdateFailed", { message: textoDoAviso(modulo.erro) })}
                            </p>
                        ) : null}

                        {!m.enabled ? (
                            <SofaBanner tone="warn" title={t("matrix.disabledTitle")}>
                                {t("matrix.disabledText")}
                            </SofaBanner>
                        ) : null}

                        {m.enabled && m.connected === false ? (
                            <SofaBanner
                                tone="danger"
                                title={t("matrix.synapseTitle")}
                                action={
                                    <SofaButton variant="subtle" onClick={() => void carregar()}>
                                        {t("matrix.reconnect")}
                                    </SofaButton>
                                }
                            >
                                {t("matrix.synapseText")}
                            </SofaBanner>
                        ) : null}

                        {avisoModulo ? (
                            <SofaBanner tone={avisoModulo.tom === "ok" ? "success" : "danger"}>
                                {textoDoAviso(avisoModulo.fonte)}
                            </SofaBanner>
                        ) : null}

                        <dl className="poph-matrix__indicadores">
                            <Indicador
                                rotulo={t("matrix.indicatorModule")}
                                valor={m.enabled ? t("matrix.valueEnabled") : t("matrix.valueDisabled")}
                                tom={m.enabled ? "ok" : undefined}
                                detalhe={
                                    m.enabled
                                        ? m.enabled_at
                                            ? t("matrix.detailActiveSince", {
                                                  date: dataDoCorte(m.enabled_at),
                                              })
                                            : t("matrix.detailActive")
                                        : m.enabled_at
                                          ? t("matrix.detailPausedSince", {
                                                date: dataDoCorte(m.enabled_at),
                                            })
                                          : t("matrix.detailPausedNever")
                                }
                            />
                            <Indicador
                                rotulo={t("matrix.indicatorSynapse")}
                                valor={conexao.valor}
                                tom={m.connected === true ? "ok" : m.connected === false ? "erro" : undefined}
                                detalhe={conexao.detalhe}
                            />
                            <Indicador
                                rotulo={t("matrix.indicatorBacklog")}
                                valor={espelhando ? valorDoAtraso(m.backlog.messages) : "—"}
                                detalhe={
                                    !espelhando
                                        ? t("matrix.backlogPaused")
                                        : m.backlog.messages > 0
                                          ? t("matrix.backlogOldest", {
                                                duration: formatarTempoNoAr(m.backlog.oldest_seconds),
                                            })
                                          : undefined
                                }
                                tom={espelhando && m.backlog.messages > 0 ? "alerta" : undefined}
                            />
                            <Indicador
                                rotulo={t("matrix.indicatorFailures")}
                                valor={valorDasFalhas(m.pending_failures)}
                                tom={m.pending_failures > 0 ? "erro" : undefined}
                            />
                            <Indicador
                                rotulo={t("matrix.indicatorLastFailure")}
                                valor={m.last_failure ?? t("matrix.noRecentFailure")}
                                tom={m.last_failure ? "erro" : undefined}
                            />
                        </dl>
                    </>
                ) : null}
            </section>

            <section className="poph-secao" aria-labelledby={idEspacos}>
                <h2 id={idEspacos} className="poph-secao__titulo">{t("matrix.sectionSpaces")}</h2>
                <p className="poph-matrix__ajuda">
                    {t("matrix.spacesHelpBefore")}
                    <code>!</code>
                    {t("matrix.spacesHelpBetween")}
                    <code>#</code>
                    {t("matrix.spacesHelpAfter")}
                </p>

                {espacos.carregando && !lista ? (
                    <div
                        className="poph-tabela poph-matrix__tabela"
                        aria-busy="true"
                        aria-label={t("matrix.loadingSpacesLabel")}
                    >
                        <div className="poph-tabela__cabecalho" role="presentation">
                            <span>{t("matrix.columnDepartment")}</span>
                            <span>{t("matrix.columnSpace")}</span>
                            <span>{t("matrix.columnOrigin")}</span>
                            <span>{t("matrix.columnStatus")}</span>
                            <span className="poph-tabela__celula--acoes">{t("matrix.columnActions")}</span>
                        </div>
                        {[0, 1, 2].map((indice) => (
                            <div className="poph-tabela__linha" role="presentation" key={indice}>
                                <SofaSkeleton lines={2} />
                                <SofaSkeleton lines={2} />
                                <SofaSkeleton lines={1} />
                                <SofaSkeleton lines={1} />
                                <SofaSkeleton lines={1} />
                            </div>
                        ))}
                    </div>
                ) : espacos.erro && !lista ? (
                    <SofaErrorState
                        title={t("matrix.spacesLoadFailed")}
                        description={textoDoAviso(espacos.erro)}
                        retry={{ label: t("operation.retry"), onClick: () => void carregar() }}
                    />
                ) : lista ? (
                    <>
                        {espacos.erro ? (
                            <p className="poph-matrix__aviso" role="alert">
                                {t("matrix.lastUpdateFailed", { message: textoDoAviso(espacos.erro) })}
                            </p>
                        ) : null}

                        {departamentos.erro && !deptos ? (
                            <p className="poph-matrix__aviso" role="alert">
                                {t("matrix.departmentsLoadFailed", {
                                    message: textoDoAviso(departamentos.erro),
                                })}
                            </p>
                        ) : null}

                        {avisoEspaco ? (
                            <SofaBanner tone={avisoEspaco.tom === "ok" ? "success" : "danger"}>
                                {textoDoAviso(avisoEspaco.fonte)}
                            </SofaBanner>
                        ) : null}

                        {departamentos.carregando && !deptos ? (
                            <SofaSkeleton lines={3} />
                        ) : linhas.length === 0 ? (
                            <SofaEmptyState
                                title={t("matrix.noActiveDepartment")}
                                description={t("matrix.noActiveDepartmentText")}
                            />
                        ) : (
                            <div
                                className="poph-tabela poph-matrix__tabela"
                                role="table"
                                aria-label={t("matrix.spacesTableLabel")}
                            >
                                <div className="poph-tabela__cabecalho" role="row">
                                    <span role="columnheader">{t("matrix.columnDepartment")}</span>
                                    <span role="columnheader">{t("matrix.columnSpace")}</span>
                                    <span role="columnheader">{t("matrix.columnOrigin")}</span>
                                    <span role="columnheader">{t("matrix.columnStatus")}</span>
                                    <span role="columnheader" className="poph-tabela__celula--acoes">
                                        {t("matrix.columnActions")}
                                    </span>
                                </div>
                                {linhas.map((linha) => {
                                    const { espaco } = linha;
                                    return (
                                        <div className="poph-tabela__linha" role="row" key={linha.id}>
                                            <span role="cell" className="poph-tabela__celula poph-matrix__departamento">
                                                <strong>{linha.nome}</strong>
                                                <span className="poph-matrix__pequeno">{linha.id}</span>
                                            </span>
                                            <span role="cell" className="poph-tabela__celula">
                                                {espaco ? (
                                                    <>
                                                        <span className="poph-matrix__alias">{espaco.alias}</span>
                                                        <span className="poph-matrix__pequeno">{espaco.space_room_id}</span>
                                                    </>
                                                ) : (
                                                    <>
                                                        <span className="poph-matrix__vazio">
                                                            {t("matrix.noSpaceLinked")}
                                                        </span>
                                                        <span className="poph-matrix__pequeno">
                                                            {t("matrix.noSpaceLinkHint")}
                                                        </span>
                                                    </>
                                                )}
                                            </span>
                                            <span role="cell" className="poph-tabela__celula">
                                                {espaco ? (
                                                    <span
                                                        className={
                                                            espaco.origin === "auto"
                                                                ? "poph-matrix__origem poph-matrix__origem--auto"
                                                                : "poph-matrix__origem poph-matrix__origem--vinculado"
                                                        }
                                                    >
                                                        {textoDaOrigem(espaco.origin)}
                                                    </span>
                                                ) : (
                                                    <span className="poph-matrix__origem poph-matrix__origem--nenhuma">
                                                        {t("matrix.originNotConfigured")}
                                                    </span>
                                                )}
                                            </span>
                                            <span role="cell" className="poph-tabela__celula">
                                                {espaco && !linha.arquivado ? (
                                                    <span className="poph-status poph-status--ativo">
                                                        <span className="poph-status__ponto" aria-hidden="true" />
                                                        {t("matrix.statusActive")}
                                                    </span>
                                                ) : espaco ? (
                                                    <span className="poph-status poph-status--inativo">
                                                        <span className="poph-status__ponto" aria-hidden="true" />
                                                        {t("matrix.statusArchivedDepartment")}
                                                    </span>
                                                ) : (
                                                    <span className="poph-status poph-status--inativo">
                                                        <span className="poph-status__ponto" aria-hidden="true" />
                                                        {t("matrix.statusNoSpace")}
                                                    </span>
                                                )}
                                            </span>
                                            <span role="cell" className="poph-tabela__celula poph-tabela__celula--acoes">
                                                {espaco ? (
                                                    <SofaButton
                                                        variant="subtle"
                                                        aria-label={t("matrix.disconnectAria", {
                                                            name: linha.nome,
                                                        })}
                                                        disabled={desligandoEspaco}
                                                        onClick={() => setEspacoParaDesligar(espaco)}
                                                    >
                                                        {t("matrix.disconnect")}
                                                    </SofaButton>
                                                ) : (
                                                    <>
                                                        <SofaButton
                                                            variant="primary"
                                                            size="sm"
                                                            aria-label={t("matrix.createAutoAria", {
                                                                name: linha.nome,
                                                            })}
                                                            loading={criandoPara === linha.id}
                                                            disabled={criandoPara !== null}
                                                            onClick={() => void criarEspaco(linha)}
                                                        >
                                                            {t("matrix.createAuto")}
                                                        </SofaButton>
                                                        <SofaButton
                                                            variant="secondary"
                                                            size="sm"
                                                            aria-label={t("matrix.linkExistingAria", {
                                                                name: linha.nome,
                                                            })}
                                                            disabled={criandoPara !== null}
                                                            onClick={() => abrirLigacao(linha)}
                                                        >
                                                            {t("matrix.linkExisting")}
                                                        </SofaButton>
                                                    </>
                                                )}
                                            </span>
                                        </div>
                                    );
                                })}
                            </div>
                        )}

                        <SofaBanner tone="info" title={t("matrix.operationRuleTitle")}>
                            {t("matrix.operationRuleText")}
                        </SofaBanner>
                    </>
                ) : null}
            </section>

            <SofaModal
                open={ligandoA !== null}
                onOpenChange={(aberto) => {
                    if (!aberto) fecharLigacao();
                }}
                title={t("matrix.linkModalTitle")}
                description={t("matrix.linkModalDescription")}
                closeLabel={t("matrix.linkModalClose")}
                footer={
                    <>
                        <SofaButton variant="secondary" disabled={enviandoLigacao} onClick={fecharLigacao}>
                            {t("matrix.cancel")}
                        </SofaButton>
                        <SofaButton
                            type="submit"
                            form={idFormLigacao}
                            variant="primary"
                            loading={enviandoLigacao}
                            disabled={enviandoLigacao || espacoDigitado.trim() === ""}
                        >
                            {t("matrix.linkSpaceAction")}
                        </SofaButton>
                    </>
                }
            >
                <form id={idFormLigacao} className="poph-matrix__form" onSubmit={(evento) => void ligarExistente(evento)} noValidate>
                    <div className="poph-matrix__departamento-escolhido">
                        <span className="poph-matrix__pequeno">{t("matrix.selectedDepartment")}</span>
                        <strong>{ligandoA ? `${ligandoA.nome} (${ligandoA.id})` : ""}</strong>
                    </div>
                    {erroDoDialogo ? (
                        <SofaBanner tone="danger">{textoDoAviso(erroDoDialogo)}</SofaBanner>
                    ) : null}
                    <SofaField
                        label={t("matrix.spaceFieldLabel")}
                        hint={t("matrix.spaceFieldHint")}
                        htmlFor={idEspacoExistente}
                    >
                        {(controle) => (
                            <SofaInput
                                id={controle.id}
                                aria-describedby={controle.describedBy}
                                invalid={controle.invalid}
                                required={controle.required}
                                value={espacoDigitado}
                                autoComplete="off"
                                autoFocus
                                disabled={enviandoLigacao}
                                placeholder={t("matrix.spaceFieldPlaceholder")}
                                onChange={(evento) => setEspacoDigitado(evento.target.value)}
                            />
                        )}
                    </SofaField>
                </form>
            </SofaModal>

            <SofaConfirmDialog
                open={confirmandoDesligarModulo}
                onOpenChange={(aberto) => {
                    if (!aberto && !enviandoModulo) setConfirmandoDesligarModulo(false);
                }}
                title={t("matrix.confirmDisableTitle")}
                description={t("matrix.confirmDisableText")}
                confirmLabel={t("matrix.disableModule")}
                cancelLabel={t("matrix.cancel")}
                closeLabel={t("matrix.confirmDisableClose")}
                tone="danger"
                loading={enviandoModulo}
                onConfirm={() => {
                    setConfirmandoDesligarModulo(false);
                    void alternarModulo(false);
                }}
            />

            <SofaConfirmDialog
                open={espacoParaDesligar !== null}
                onOpenChange={(aberto) => {
                    if (!aberto && !desligandoEspaco) setEspacoParaDesligar(null);
                }}
                title={
                    espacoParaDesligar
                        ? t("matrix.confirmDisconnectTitle", {
                              department: nomeDoEspaco(espacoParaDesligar),
                          })
                        : t("matrix.confirmDisconnectTitleFallback")
                }
                description={t("matrix.confirmDisconnectText")}
                confirmLabel={t("matrix.confirmDisconnectAction")}
                cancelLabel={t("matrix.cancel")}
                closeLabel={t("matrix.confirmDisconnectClose")}
                tone="danger"
                loading={desligandoEspaco}
                onConfirm={() => {
                    if (espacoParaDesligar) void desligarEspaco(espacoParaDesligar);
                }}
            />
        </div>
    );
}

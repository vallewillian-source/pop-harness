/**
 * Painel lateral do "por quê" de um turno do chat de teste (HR-47, UX_SPEC §3.7).
 *
 * Abre sobre a conversa (`SofaDrawer`: foco preso, `Esc` fecha, foco volta ao botão que abriu) e
 * mostra o que o turno gravou: decisões do Jev por gate, trechos e produtos com a fonte, ferramentas
 * chamadas, tempos por etapa, tokens e custo. Quando um dado não foi gravado, a tela diz isso em
 * vez de mostrar um número.
 *
 * Os dados vêm da leitura da conversa que a página já tem; a página recarrega essa leitura quando o
 * painel abre. Erro de leitura aparece no painel com "Tentar de novo" e não apaga o que já estava
 * na tela.
 */

import { SofaBanner, SofaButton, SofaDrawer, SofaStatusIndicator } from "@sofacoding/sofa-ui";
import {
    countTools,
    turnDuration,
    formatConfidence,
    formatToolContent,
    formatCostMicros,
    formatToolDuration,
    formatMargin,
    formatMs,
    formatPriceCents,
    formatTokens,
    readToolCalls,
    readTurnTools,
    readTurnJev,
    readTurnRetrieved,
    readTurnTimings,
    originLabel,
    turnStateLabel,
    endReasonLabel,
    refusalReasonLabel,
    gateTitle,
    type ToolCall,
    type GateDecision,
    type TurnJev,
    type TurnRetrieved,
    type TurnTimings,
} from "../api/turnWhy";
import type { TestTurn } from "../api/testChat";
import { useI18n } from "../i18n";
import "../estilos/painel-porque.css";

type Props = {
    aberto: boolean;
    /** Posição do turno na conversa (1 é o primeiro). */
    numero: number | null;
    /** O turno, como a leitura da conversa o trouxe. `null`: ele não está na leitura atual. */
    turno: TestTurn | null;
    /** A leitura está sendo refeita. */
    atualizando: boolean;
    /** Motivo do erro da última leitura, quando ela falhou. */
    erroDeLeitura: string | null;
    aoFechar: () => void;
    aoTentarDeNovo: () => void;
};

export function PainelPorQue({
    aberto,
    numero,
    turno,
    atualizando,
    erroDeLeitura,
    aoFechar,
    aoTentarDeNovo,
}: Props) {
    const { translate: t } = useI18n();
    const title = numero !== null ? t("testChat.panelTitle", { number: numero }) : t("testChat.panelTitleNoNumber");
    return (
        <SofaDrawer
            open={aberto}
            onOpenChange={(proximo) => {
                if (!proximo) aoFechar();
            }}
            title={title}
            description={t("testChat.panelDescription")}
            closeLabel={t("testChat.panelClose")}
            size="lg"
        >
            <div className="poph-porque">
                {erroDeLeitura && (
                    <SofaBanner tone="danger" title={t("testChat.panelRefreshError")}>
                        <p>{erroDeLeitura}</p>
                        <SofaButton variant="secondary" onClick={aoTentarDeNovo} disabled={atualizando}>
                            {t("testChat.retry")}
                        </SofaButton>
                    </SofaBanner>
                )}
                {atualizando && (
                    <p className="poph-porque__atualizando" role="status">
                        {t("testChat.panelRefreshing")}
                    </p>
                )}
                {turno === null ? (
                    <p className="poph-porque__vazio">{t("testChat.panelEmpty")}</p>
                ) : (
                    <ConteudoDoTurno turno={turno} numero={numero} />
                )}
            </div>
        </SofaDrawer>
    );
}

function ConteudoDoTurno({ turno, numero }: { turno: TestTurn; numero: number | null }) {
    const { translate: t } = useI18n();
    const jev = readTurnJev(turno.jev);
    const recuperados = readTurnRetrieved(turno.retrieved);
    const ferramentas = readTurnTools(turno.tools);
    const chamadas = readToolCalls(turno.tool_calls);
    const tempos = readTurnTimings(turno.timings);
    const duracao = turnDuration(turno.started_at, turno.finished_at, tempos);

    return (
        <>
            <CabecalhoDoTurno numero={numero} turno={turno} duracao={duracao} />

            {turno.error && (
                <section className="poph-porque__secao" aria-labelledby="porque-erro">
                    <h3 id="porque-erro" className="poph-porque__titulo">
                        {t("testChat.panelErrorSection")}
                    </h3>
                    <p className="poph-porque__erro">{turno.error}</p>
                </section>
            )}

            <SecaoJev jev={jev} />
            <SecaoRecuperados recuperados={recuperados} />
            <SecaoFerramentas chamadas={chamadas} nomesAntigos={ferramentas} />
            <SecaoTempos tempos={tempos} />
        </>
    );
}

function CabecalhoDoTurno({
    numero,
    turno,
    duracao,
}: {
    numero: number | null;
    turno: TestTurn;
    duracao: number | null;
}) {
    const { translate: t, formatDateTime } = useI18n();
    const entrada = turno.input_tokens ?? null;
    const saida = turno.output_tokens ?? null;
    // Zero nos dois lados é o padrão de quem não chegou a usar o modelo (ou não gravou o uso): não é
    // uma medição, então não entra como "0 tokens".
    const medido = (entrada ?? 0) + (saida ?? 0) > 0;
    const total = medido ? (entrada ?? 0) + (saida ?? 0) : null;
    return (
        <section className="poph-porque__secao" aria-labelledby="porque-cabecalho">
            <h3 id="porque-cabecalho" className="poph-porque__titulo">
                {t("testChat.panelSummary")}
            </h3>
            <dl className="poph-porque__resumo">
                <div>
                    <dt>{t("testChat.fieldTurn")}</dt>
                    <dd>{numero !== null ? t("testChat.turnNumber", { number: numero }) : t("testChat.notRecorded")}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldState")}</dt>
                    <dd>{turnStateLabel(turno.state)}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldStart")}</dt>
                    <dd>
                        <time dateTime={new Date(turno.started_at).toISOString()}>{formatDateTime(turno.started_at)}</time>
                    </dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldDuration")}</dt>
                    <dd>
                        {duracao !== null
                            ? formatMs(duracao)
                            : turno.state === "running"
                              ? t("testChat.durationRunning")
                              : t("testChat.durationNotRecorded")}
                    </dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldModelTokens")}</dt>
                    <dd>
                        {total === null
                            ? t("testChat.notRecorded")
                            : t("testChat.tokensBreakdown", {
                                  total: formatTokens(total),
                                  input: formatTokens(entrada),
                                  output: formatTokens(saida),
                              })}
                    </dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldCost")}</dt>
                    <dd>{formatCostMicros(turno.cost_micros ?? null)}</dd>
                </div>
            </dl>
        </section>
    );
}

function SecaoJev({ jev }: { jev: TurnJev | null }) {
    const { translate: t } = useI18n();
    return (
        <section className="poph-porque__secao" aria-labelledby="porque-jev">
            <h3 id="porque-jev" className="poph-porque__titulo">
                {t("testChat.sectionJev")}
            </h3>
            {jev === null ? (
                <p className="poph-porque__vazio">{t("testChat.jevEmpty")}</p>
            ) : (
                <>
                    {jev.chamada && (
                        <dl className="poph-porque__linhas">
                            <div>
                                <dt>{t("testChat.fieldJevCall")}</dt>
                                <dd>
                                    {jev.chamada.perguntas === null
                                        ? t("testChat.notRecorded")
                                        : t("testChat.jevCallSummary", {
                                              questions: jev.chamada.perguntas,
                                              cached: jev.chamada.decididasPorCache ?? 0,
                                          })}
                                </dd>
                            </div>
                            <div>
                                <dt>{t("testChat.fieldBatchTime")}</dt>
                                <dd>{formatMs(jev.chamada.ms)}</dd>
                            </div>
                            <div>
                                <dt>{t("testChat.fieldJevTokens")}</dt>
                                <dd>
                                    {jev.chamada.total !== null || jev.chamada.tokens !== null
                                        ? `${formatTokens(jev.chamada.total ?? jev.chamada.tokens)}${
                                              jev.chamada.entrada !== null || jev.chamada.saida !== null
                                                  ? ` ${t("testChat.tokensBreakdownOnly", {
                                                        input: formatTokens(jev.chamada.entrada),
                                                        output: formatTokens(jev.chamada.saida),
                                                    })}`
                                                  : ""
                                          }`
                                        : t("testChat.jevTokensNotInformed")}
                                </dd>
                            </div>
                            {jev.chamada.falha && (
                                <div>
                                    <dt>{t("testChat.fieldCallFailure")}</dt>
                                    <dd className="poph-porque__erro">{jev.chamada.falha}</dd>
                                </div>
                            )}
                        </dl>
                    )}

                    {jev.decisoes.length === 0 && <p className="poph-porque__vazio">{t("testChat.jevNoDecisions")}</p>}
                    {jev.decisoes.map((decisao, indice) => (
                        <CartaoDoGate key={`${decisao.gate}-${indice}`} decisao={decisao} />
                    ))}

                    {jev.revisao && (
                        <article className="poph-porque__cartao" aria-labelledby="porque-g6">
                            <h4 id="porque-g6" className="poph-porque__subtitulo">
                                {t("testChat.g6Title")}
                            </h4>
                            <dl className="poph-porque__linhas">
                                <div>
                                    <dt>{t("testChat.fieldG6Choice")}</dt>
                                    <dd>{jev.revisao.escolha ?? t("testChat.notRecorded")}</dd>
                                </div>
                                <div>
                                    <dt>{t("testChat.fieldConfidence")}</dt>
                                    <dd>{formatConfidence(jev.revisao.confianca)}</dd>
                                </div>
                                <div>
                                    <dt>{t("testChat.fieldRewrote")}</dt>
                                    <dd>{jev.revisao.regenerou ? t("testChat.yes") : t("testChat.no")}</dd>
                                </div>
                                {jev.revisao.segundaEscolha && (
                                    <div>
                                        <dt>{t("testChat.fieldSecondChoice")}</dt>
                                        <dd>{jev.revisao.segundaEscolha}</dd>
                                    </div>
                                )}
                                <div>
                                    <dt>{t("testChat.fieldSafeResponse")}</dt>
                                    <dd>{jev.revisao.respostaSegura ? t("testChat.yes") : t("testChat.no")}</dd>
                                </div>
                            </dl>
                        </article>
                    )}

                    <p className="poph-porque__nao-gravado">{t("testChat.jevNotRecorded")}</p>
                </>
            )}
        </section>
    );
}

function CartaoDoGate({ decisao }: { decisao: GateDecision }) {
    const { translate: t } = useI18n();
    const id = `porque-gate-${decisao.gate}-${decisao.escolha ?? "sem"}`;
    return (
        <article className="poph-porque__cartao" aria-labelledby={id}>
            <h4 id={id} className="poph-porque__subtitulo">
                {gateTitle(decisao.gate)}
            </h4>
            <dl className="poph-porque__linhas">
                <div>
                    <dt>{t("testChat.fieldChosen")}</dt>
                    <dd>{decisao.escolha ?? t("testChat.none")}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldJevChoice")}</dt>
                    <dd>{decisao.escolhaDoJev ?? t("testChat.notRecorded")}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldOrigin")}</dt>
                    <dd>{originLabel(decisao.origem)}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldConfidence")}</dt>
                    <dd>{formatConfidence(decisao.confianca)}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldMargin")}</dt>
                    <dd>{formatMargin(decisao.margem)}</dd>
                </div>
                {decisao.motivo && (
                    <div>
                        <dt>{t("testChat.fieldReason")}</dt>
                        <dd>{decisao.motivo}</dd>
                    </div>
                )}
                <div>
                    <dt>{t("testChat.fieldBatchTime")}</dt>
                    <dd>{formatMs(decisao.ms)}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldBatchTokens")}</dt>
                    <dd>{formatTokens(decisao.tokens)}</dd>
                </div>
            </dl>
        </article>
    );
}

function SecaoRecuperados({ recuperados }: { recuperados: TurnRetrieved }) {
    const { translate: t, formatNumber } = useI18n();
    const vazio = recuperados.trechos.length === 0 && recuperados.produtos.length === 0;
    return (
        <section className="poph-porque__secao" aria-labelledby="porque-recuperados">
            <h3 id="porque-recuperados" className="poph-porque__titulo">
                {t("testChat.sectionRetrieved")}
            </h3>
            {vazio ? (
                <p className="poph-porque__vazio">{t("testChat.retrievedEmpty")}</p>
            ) : (
                <>
                    {recuperados.trechos.length > 0 && (
                        <>
                            <h4 className="poph-porque__subtitulo">{t("testChat.subheadingKnowledge")}</h4>
                            <ul className="poph-porque__lista">
                                {recuperados.trechos.map((trecho, indice) => (
                                    <li key={`${trecho.item ?? "trecho"}-${indice}`} className="poph-porque__item">
                                        <strong>{trecho.titulo}</strong>
                                        <span className="poph-porque__meta">
                                            {t("testChat.snippetMeta", {
                                                group: trecho.grupo || t("testChat.notRecorded"),
                                                similarity: formatConfidence(trecho.similaridade),
                                                relevance:
                                                    trecho.relevancia === null
                                                        ? t("testChat.notRecordedF")
                                                        : formatNumber(trecho.relevancia, {
                                                              minimumFractionDigits: 4,
                                                              maximumFractionDigits: 4,
                                                          }),
                                            })}
                                        </span>
                                        {trecho.texto && (
                                            <details>
                                                <summary>{t("testChat.showSnippetText")}</summary>
                                                <p className="poph-porque__trecho">{trecho.texto}</p>
                                            </details>
                                        )}
                                    </li>
                                ))}
                            </ul>
                        </>
                    )}
                    {recuperados.produtos.length > 0 && (
                        <>
                            <h4 className="poph-porque__subtitulo">{t("testChat.subheadingProducts")}</h4>
                            <ul className="poph-porque__lista">
                                {recuperados.produtos.map((produto, indice) => {
                                    const preco =
                                        produto.precoTexto ??
                                        formatPriceCents(produto.precoCentavos) ??
                                        t("testChat.priceNotRecorded");
                                    return (
                                        <li key={`${produto.produto ?? "produto"}-${indice}`} className="poph-porque__item">
                                            <strong>{produto.nome}</strong>
                                            <span className="poph-porque__meta">
                                                {t("testChat.productPriceSeen", { price: preco })}
                                                {produto.unidade ? ` · ${produto.unidade}` : ""}
                                            </span>
                                            {produto.condicoes && (
                                                <span className="poph-porque__meta">
                                                    {t("testChat.productConditions", { conditions: produto.condicoes })}
                                                </span>
                                            )}
                                        </li>
                                    );
                                })}
                            </ul>
                        </>
                    )}
                </>
            )}
            <p className="poph-porque__nao-gravado">{t("testChat.retrievedNotRecorded")}</p>
        </section>
    );
}

function SecaoFerramentas({
    chamadas,
    nomesAntigos,
}: {
    chamadas: ToolCall[];
    nomesAntigos: string[];
}) {
    const { translate: t } = useI18n();
    // Turno gravado antes da HR-47b: tem `tools` (só os nomes) e nenhum detalhe por chamada.
    const soNomes = chamadas.length === 0 && nomesAntigos.length > 0;
    return (
        <section className="poph-porque__secao" aria-labelledby="porque-ferramentas">
            <h3 id="porque-ferramentas" className="poph-porque__titulo">
                {t("testChat.sectionTools")}
            </h3>
            {chamadas.length > 0 && (
                <ol className="poph-porque__lista" aria-label={t("testChat.toolCallsLabel")}>
                    {chamadas.map((chamada, indice) => (
                        <ChamadaDaFerramenta key={`${chamada.rodada ?? "r"}-${chamada.ordem ?? indice}`} chamada={chamada} />
                    ))}
                </ol>
            )}
            {soNomes && (
                <>
                    <p className="poph-porque__nota">{t("testChat.legacyToolNote")}</p>
                    <ul className="poph-porque__lista">
                        {countTools(nomesAntigos).map((item) => (
                            <li key={item.nome} className="poph-porque__item">
                                <strong>{item.nome}</strong>
                                <span className="poph-porque__meta">
                                    {item.vezes === 1
                                        ? t("testChat.toolCalledOnce")
                                        : t("testChat.toolCalledMany", { count: item.vezes })}
                                </span>
                            </li>
                        ))}
                    </ul>
                </>
            )}
            {chamadas.length === 0 && nomesAntigos.length === 0 && (
                <p className="poph-porque__vazio">{t("testChat.toolsEmpty")}</p>
            )}
            <p className="poph-porque__nao-gravado">{t("testChat.toolsNotRecorded")}</p>
        </section>
    );
}

/** Uma chamada: executada e recusada têm estado com rótulo e cor diferentes, e a recusa mostra o motivo. */
function ChamadaDaFerramenta({ chamada }: { chamada: ToolCall }) {
    const { translate: t, formatNumber } = useI18n();
    const recusada = chamada.estado === "recusada";
    const stateLabel =
        chamada.estado === "executada"
            ? t("testChat.toolStateExecuted")
            : recusada
              ? t("testChat.toolStateRejected")
              : t("testChat.toolStateUnrecorded");
    const tom = chamada.estado === "executada" ? "success" : recusada ? "attention" : "muted";
    const rodada =
        chamada.rodada === null
            ? t("testChat.roundUnrecorded")
            : t("testChat.roundNumber", { number: formatNumber(chamada.rodada + 1) });
    const ordem = chamada.ordem === null ? "" : t("testChat.callOrder", { number: formatNumber(chamada.ordem) });
    return (
        <li className={`poph-porque__item poph-porque__chamada${recusada ? " poph-porque__chamada--recusada" : ""}`}>
            <div className="poph-porque__cabecalho-chamada">
                <strong>{chamada.nome}</strong>
                <SofaStatusIndicator tone={tom}>{stateLabel}</SofaStatusIndicator>
            </div>
            <span className="poph-porque__meta">
                {rodada}
                {ordem}
            </span>
            {recusada && (
                <p className="poph-porque__motivo">
                    {t("testChat.rejectedReason", { reason: refusalReasonLabel(chamada.motivo) })}
                </p>
            )}
            <dl className="poph-porque__linhas">
                <div>
                    <dt>{t("testChat.fieldDuration")}</dt>
                    <dd>{formatToolDuration(chamada.duracaoMs)}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldArguments")}</dt>
                    <dd>
                        {chamada.argumentos === null ? (
                            t("testChat.notRecordedPlural")
                        ) : (
                            <pre
                                className="poph-porque__texto"
                                tabIndex={0}
                                aria-label={t("testChat.argumentsLabel", { name: chamada.nome })}
                            >
                                {formatToolContent(chamada.argumentos)}
                            </pre>
                        )}
                    </dd>
                </div>
                <div>
                    <dt>{recusada ? t("testChat.fieldErrorToModel") : t("testChat.fieldResult")}</dt>
                    <dd>
                        {chamada.resultado === null ? (
                            t("testChat.notRecorded")
                        ) : (
                            <pre
                                className="poph-porque__texto"
                                tabIndex={0}
                                aria-label={t("testChat.resultLabel", { name: chamada.nome })}
                            >
                                {formatToolContent(chamada.resultado)}
                            </pre>
                        )}
                    </dd>
                </div>
            </dl>
        </li>
    );
}

function SecaoTempos({ tempos }: { tempos: TurnTimings | null }) {
    const { translate: t } = useI18n();
    if (tempos === null) {
        return (
            <section className="poph-porque__secao" aria-labelledby="porque-tempos">
                <h3 id="porque-tempos" className="poph-porque__titulo">
                    {t("testChat.sectionTimes")}
                </h3>
                <p className="poph-porque__vazio">{t("testChat.timesEmpty")}</p>
            </section>
        );
    }
    const total = tempos.totalMs;
    const etapas: Array<{ rotulo: string; ms: number | null }> = [
        { rotulo: t("testChat.timeQueue"), ms: tempos.filaMs },
        { rotulo: t("testChat.timeAssembly"), ms: tempos.montagemMs },
        { rotulo: t("testChat.timeFirstText"), ms: tempos.primeiroTextoMs },
        { rotulo: t("testChat.timeTotal"), ms: total },
    ];
    return (
        <section className="poph-porque__secao" aria-labelledby="porque-tempos">
            <h3 id="porque-tempos" className="poph-porque__titulo">
                {t("testChat.sectionTimes")}
            </h3>
            <ul className="poph-porque__lista">
                {etapas.map((etapa) => (
                    <li key={etapa.rotulo} className="poph-porque__item poph-porque__tempo">
                        <span>{etapa.rotulo}</span>
                        <span className="poph-porque__meta">{formatMs(etapa.ms)}</span>
                        {etapa.ms !== null && total !== null && total > 0 && (
                            <progress
                                className="poph-porque__barra"
                                max={total}
                                value={Math.min(etapa.ms, total)}
                                aria-label={t("testChat.timeBarLabel", {
                                    label: etapa.rotulo,
                                    value: formatMs(etapa.ms),
                                    total: formatMs(total),
                                })}
                            />
                        )}
                    </li>
                ))}
            </ul>
            <dl className="poph-porque__linhas">
                <div>
                    <dt>{t("testChat.fieldToolRounds")}</dt>
                    <dd>{tempos.rodadasFerramenta ?? t("testChat.notRecorded")}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldModel")}</dt>
                    <dd>{tempos.modelo ?? t("testChat.notRecorded")}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldEndReason")}</dt>
                    <dd>{endReasonLabel(tempos.motivoFim)}</dd>
                </div>
            </dl>
            <p className="poph-porque__nao-gravado">{t("testChat.timesNotRecorded")}</p>
        </section>
    );
}

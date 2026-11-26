
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
import "../styles/why-panel.css";

type Props = {
    opened: boolean;
    numero: number | null;
    turno: TestTurn | null;
    updating: boolean;
    errorRead: string | null;
    onClose: () => void;
    onTryNew: () => void;
};

export function PanelWhy({
    opened: isOpen,
    numero,
    turno,
    updating,
    errorRead,
    onClose,
    onTryNew,
}: Props) {
    const { translate: t } = useI18n();
    const title = numero !== null ? t("testChat.panelTitle", { number: numero }) : t("testChat.panelTitleNoNumber");
    return (
        <SofaDrawer
            open={isOpen}
            onOpenChange={(next) => {
                if (!next) onClose();
            }}
            title={title}
            description={t("testChat.panelDescription")}
            closeLabel={t("testChat.panelClose")}
            size="lg"
        >
            <div className="poph-why">
                {errorRead && (
                    <SofaBanner tone="danger" title={t("testChat.panelRefreshError")}>
                        <p>{errorRead}</p>
                        <SofaButton variant="secondary" onClick={onTryNew} disabled={updating}>
                            {t("testChat.retry")}
                        </SofaButton>
                    </SofaBanner>
                )}
                {updating && (
                    <p className="poph-why__updating" role="status">
                        {t("testChat.panelRefreshing")}
                    </p>
                )}
                {turno === null ? (
                    <p className="poph-why__empty">{t("testChat.panelEmpty")}</p>
                ) : (
                    <ContentTurn turno={turno} numero={numero} />
                )}
            </div>
        </SofaDrawer>
    );
}

function ContentTurn({ turno, numero }: { turno: TestTurn; numero: number | null }) {
    const { translate: t } = useI18n();
    const jev = readTurnJev(turno.jev);
    const recovered = readTurnRetrieved(turno.retrieved);
    const tools = readTurnTools(turno.tools);
    const calls = readToolCalls(turno.tool_calls);
    const times = readTurnTimings(turno.timings);
    const duration = turnDuration(turno.started_at, turno.finished_at, times);

    return (
        <>
            <HeaderTurn numero={numero} turno={turno} duration={duration} />

            {turno.error && (
                <section className="poph-why__section" aria-labelledby="porque-erro">
                    <h3 id="porque-erro" className="poph-why__title">
                        {t("testChat.panelErrorSection")}
                    </h3>
                    <p className="poph-why__error">{turno.error}</p>
                </section>
            )}

            <SectionJev jev={jev} />
            <SectionRecovered recovered={recovered} />
            <SectionTools calls={calls} namesOld={tools} />
            <SectionTimes timings={times} />
        </>
    );
}

function HeaderTurn({
    numero,
    turno,
    duration,
}: {
    numero: number | null;
    turno: TestTurn;
    duration: number | null;
}) {
    const { translate: t, formatDateTime } = useI18n();
    const entry = turno.input_tokens ?? null;
    const saida = turno.output_tokens ?? null;
    const measured = (entry ?? 0) + (saida ?? 0) > 0;
    const total = measured ? (entry ?? 0) + (saida ?? 0) : null;
    return (
        <section className="poph-why__section" aria-labelledby="porque-cabecalho">
            <h3 id="porque-cabecalho" className="poph-why__title">
                {t("testChat.panelSummary")}
            </h3>
            <dl className="poph-why__summary">
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
                        {duration !== null
                            ? formatMs(duration)
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
                                  input: formatTokens(entry),
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

function SectionJev({ jev }: { jev: TurnJev | null }) {
    const { translate: t } = useI18n();
    return (
        <section className="poph-why__section" aria-labelledby="porque-jev">
            <h3 id="porque-jev" className="poph-why__title">
                {t("testChat.sectionJev")}
            </h3>
            {jev === null ? (
                <p className="poph-why__empty">{t("testChat.jevEmpty")}</p>
            ) : (
                <>
                    {jev.call && (
                        <dl className="poph-why__lines">
                            <div>
                                <dt>{t("testChat.fieldJevCall")}</dt>
                                <dd>
                                    {jev.call.questions === null
                                        ? t("testChat.notRecorded")
                                        : t("testChat.jevCallSummary", {
                                              questions: jev.call.questions,
                                              cached: jev.call.decidedByCache ?? 0,
                                          })}
                                </dd>
                            </div>
                            <div>
                                <dt>{t("testChat.fieldBatchTime")}</dt>
                                <dd>{formatMs(jev.call.ms)}</dd>
                            </div>
                            <div>
                                <dt>{t("testChat.fieldJevTokens")}</dt>
                                <dd>
                                    {jev.call.total !== null || jev.call.tokens !== null
                                        ? `${formatTokens(jev.call.total ?? jev.call.tokens)}${
                                              jev.call.input !== null || jev.call.saida !== null
                                                  ? ` ${t("testChat.tokensBreakdownOnly", {
                                                        input: formatTokens(jev.call.input),
                                                        output: formatTokens(jev.call.saida),
                                                    })}`
                                                  : ""
                                          }`
                                        : t("testChat.jevTokensNotInformed")}
                                </dd>
                            </div>
                            {jev.call.failure && (
                                <div>
                                    <dt>{t("testChat.fieldCallFailure")}</dt>
                                    <dd className="poph-why__error">{jev.call.failure}</dd>
                                </div>
                            )}
                        </dl>
                    )}

                    {jev.decisions.length === 0 && <p className="poph-why__empty">{t("testChat.jevNoDecisions")}</p>}
                    {jev.decisions.map((decisao, index) => (
                        <CardGate key={`${decisao.gate}-${index}`} decisao={decisao} />
                    ))}

                    {jev.review && (
                        <article className="poph-why__card" aria-labelledby="porque-g6">
                            <h4 id="porque-g6" className="poph-why__subtitle">
                                {t("testChat.g6Title")}
                            </h4>
                            <dl className="poph-why__lines">
                                <div>
                                    <dt>{t("testChat.fieldG6Choice")}</dt>
                                    <dd>{jev.review.choice ?? t("testChat.notRecorded")}</dd>
                                </div>
                                <div>
                                    <dt>{t("testChat.fieldConfidence")}</dt>
                                    <dd>{formatConfidence(jev.review.confidence)}</dd>
                                </div>
                                <div>
                                    <dt>{t("testChat.fieldRewrote")}</dt>
                                    <dd>{jev.review.regenerou ? t("testChat.yes") : t("testChat.no")}</dd>
                                </div>
                                {jev.review.secondChoice && (
                                    <div>
                                        <dt>{t("testChat.fieldSecondChoice")}</dt>
                                        <dd>{jev.review.secondChoice}</dd>
                                    </div>
                                )}
                                <div>
                                    <dt>{t("testChat.fieldSafeResponse")}</dt>
                                    <dd>{jev.review.responseSafe ? t("testChat.yes") : t("testChat.no")}</dd>
                                </div>
                            </dl>
                        </article>
                    )}

                    <p className="poph-why__not-stored">{t("testChat.jevNotRecorded")}</p>
                </>
            )}
        </section>
    );
}

function CardGate({ decisao }: { decisao: GateDecision }) {
    const { translate: t } = useI18n();
    const id = `porque-gate-${decisao.gate}-${decisao.choice ?? "sem"}`;
    return (
        <article className="poph-why__card" aria-labelledby={id}>
            <h4 id={id} className="poph-why__subtitle">
                {gateTitle(decisao.gate)}
            </h4>
            <dl className="poph-why__lines">
                <div>
                    <dt>{t("testChat.fieldChosen")}</dt>
                    <dd>{decisao.choice ?? t("testChat.none")}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldJevChoice")}</dt>
                    <dd>{decisao.jevChoice ?? t("testChat.notRecorded")}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldOrigin")}</dt>
                    <dd>{originLabel(decisao.origin)}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldConfidence")}</dt>
                    <dd>{formatConfidence(decisao.confidence)}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldMargin")}</dt>
                    <dd>{formatMargin(decisao.margin)}</dd>
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

function SectionRecovered({ recovered }: { recovered: TurnRetrieved }) {
    const { translate: t, formatNumber } = useI18n();
    const vazio = recovered.excerpts.length === 0 && recovered.products.length === 0;
    return (
        <section className="poph-why__section" aria-labelledby="porque-recuperados">
            <h3 id="porque-recuperados" className="poph-why__title">
                {t("testChat.sectionRetrieved")}
            </h3>
            {vazio ? (
                <p className="poph-why__empty">{t("testChat.retrievedEmpty")}</p>
            ) : (
                <>
                    {recovered.excerpts.length > 0 && (
                        <>
                            <h4 className="poph-why__subtitle">{t("testChat.subheadingKnowledge")}</h4>
                            <ul className="poph-why__list">
                                {recovered.excerpts.map((passage, index) => (
                                    <li key={`${passage.item ?? "trecho"}-${index}`} className="poph-why__item">
                                        <strong>{passage.title}</strong>
                                        <span className="poph-why__meta">
                                            {t("testChat.snippetMeta", {
                                                group: passage.group || t("testChat.notRecorded"),
                                                similarity: formatConfidence(passage.similarity),
                                                relevance:
                                                    passage.relevance === null
                                                        ? t("testChat.notRecordedF")
                                                        : formatNumber(passage.relevance, {
                                                              minimumFractionDigits: 4,
                                                              maximumFractionDigits: 4,
                                                          }),
                                            })}
                                        </span>
                                        {passage.texto && (
                                            <details>
                                                <summary>{t("testChat.showSnippetText")}</summary>
                                                <p className="poph-why__passage">{passage.texto}</p>
                                            </details>
                                        )}
                                    </li>
                                ))}
                            </ul>
                        </>
                    )}
                    {recovered.products.length > 0 && (
                        <>
                            <h4 className="poph-why__subtitle">{t("testChat.subheadingProducts")}</h4>
                            <ul className="poph-why__list">
                                {recovered.products.map((product, index) => {
                                    const price =
                                        product.priceText ??
                                        formatPriceCents(product.priceCents) ??
                                        t("testChat.priceNotRecorded");
                                    return (
                                        <li key={`${product.product ?? "produto"}-${index}`} className="poph-why__item">
                                            <strong>{product.nome}</strong>
                                            <span className="poph-why__meta">
                                                {t("testChat.productPriceSeen", { price })}
                                                {product.unit ? ` · ${product.unit}` : ""}
                                            </span>
                                            {product.conditions && (
                                                <span className="poph-why__meta">
                                                    {t("testChat.productConditions", { conditions: product.conditions })}
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
            <p className="poph-why__not-stored">{t("testChat.retrievedNotRecorded")}</p>
        </section>
    );
}

function SectionTools({
    calls,
    namesOld,
}: {
    calls: ToolCall[];
    namesOld: string[];
}) {
    const { translate: t } = useI18n();
    const onlyNames = calls.length === 0 && namesOld.length > 0;
    return (
        <section className="poph-why__section" aria-labelledby="porque-ferramentas">
            <h3 id="porque-ferramentas" className="poph-why__title">
                {t("testChat.sectionTools")}
            </h3>
            {calls.length > 0 && (
                <ol className="poph-why__list" aria-label={t("testChat.toolCallsLabel")}>
                    {calls.map((call, index) => (
                        <CallTool key={`${call.round ?? "r"}-${call.ordem ?? index}`} call={call} />
                    ))}
                </ol>
            )}
            {onlyNames && (
                <>
                    <p className="poph-why__note">{t("testChat.legacyToolNote")}</p>
                    <ul className="poph-why__list">
                        {countTools(namesOld).map((item) => (
                            <li key={item.nome} className="poph-why__item">
                                <strong>{item.nome}</strong>
                                <span className="poph-why__meta">
                                    {item.times === 1
                                        ? t("testChat.toolCalledOnce")
                                        : t("testChat.toolCalledMany", { count: item.times })}
                                </span>
                            </li>
                        ))}
                    </ul>
                </>
            )}
            {calls.length === 0 && namesOld.length === 0 && (
                <p className="poph-why__empty">{t("testChat.toolsEmpty")}</p>
            )}
            <p className="poph-why__not-stored">{t("testChat.toolsNotRecorded")}</p>
        </section>
    );
}

function CallTool({ call }: { call: ToolCall }) {
    const { translate: t, formatNumber } = useI18n();
    const refused = call.estado === "recusada";
    const stateLabel =
        call.estado === "executada"
            ? t("testChat.toolStateExecuted")
            : refused
              ? t("testChat.toolStateRejected")
              : t("testChat.toolStateUnrecorded");
    const tom = call.estado === "executada" ? "success" : refused ? "attention" : "muted";
    const round =
        call.round === null
            ? t("testChat.roundUnrecorded")
            : t("testChat.roundNumber", { number: formatNumber(call.round + 1) });
    const ordem = call.ordem === null ? "" : t("testChat.callOrder", { number: formatNumber(call.ordem) });
    return (
        <li className={`poph-why__item poph-why__call${refused ? " poph-why__call--refused" : ""}`}>
            <div className="poph-why__header-call">
                <strong>{call.nome}</strong>
                <SofaStatusIndicator tone={tom}>{stateLabel}</SofaStatusIndicator>
            </div>
            <span className="poph-why__meta">
                {round}
                {ordem}
            </span>
            {refused && (
                <p className="poph-why__reason">
                    {t("testChat.rejectedReason", { reason: refusalReasonLabel(call.motivo) })}
                </p>
            )}
            <dl className="poph-why__lines">
                <div>
                    <dt>{t("testChat.fieldDuration")}</dt>
                    <dd>{formatToolDuration(call.durationMs)}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldArguments")}</dt>
                    <dd>
                        {call.arguments === null ? (
                            t("testChat.notRecordedPlural")
                        ) : (
                            <pre
                                className="poph-why__text"
                                tabIndex={0}
                                aria-label={t("testChat.argumentsLabel", { name: call.nome })}
                            >
                                {formatToolContent(call.arguments)}
                            </pre>
                        )}
                    </dd>
                </div>
                <div>
                    <dt>{refused ? t("testChat.fieldErrorToModel") : t("testChat.fieldResult")}</dt>
                    <dd>
                        {call.resultado === null ? (
                            t("testChat.notRecorded")
                        ) : (
                            <pre
                                className="poph-why__text"
                                tabIndex={0}
                                aria-label={t("testChat.resultLabel", { name: call.nome })}
                            >
                                {formatToolContent(call.resultado)}
                            </pre>
                        )}
                    </dd>
                </div>
            </dl>
        </li>
    );
}

function SectionTimes({ timings: times }: { timings: TurnTimings | null }) {
    const { translate: t } = useI18n();
    if (times === null) {
        return (
            <section className="poph-why__section" aria-labelledby="porque-tempos">
                <h3 id="porque-tempos" className="poph-why__title">
                    {t("testChat.sectionTimes")}
                </h3>
                <p className="poph-why__empty">{t("testChat.timesEmpty")}</p>
            </section>
        );
    }
    const total = times.totalMs;
    const steps: Array<{ label: string; ms: number | null }> = [
        { label: t("testChat.timeQueue"), ms: times.queueMs },
        { label: t("testChat.timeAssembly"), ms: times.buildMs },
        { label: t("testChat.timeFirstText"), ms: times.firstTextMs },
        { label: t("testChat.timeTotal"), ms: total },
    ];
    return (
        <section className="poph-why__section" aria-labelledby="porque-tempos">
            <h3 id="porque-tempos" className="poph-why__title">
                {t("testChat.sectionTimes")}
            </h3>
            <ul className="poph-why__list">
                {steps.map((etapa) => (
                    <li key={etapa.label} className="poph-why__item poph-why__time">
                        <span>{etapa.label}</span>
                        <span className="poph-why__meta">{formatMs(etapa.ms)}</span>
                        {etapa.ms !== null && total !== null && total > 0 && (
                            <progress
                                className="poph-why__bar"
                                max={total}
                                value={Math.min(etapa.ms, total)}
                                aria-label={t("testChat.timeBarLabel", {
                                    label: etapa.label,
                                    value: formatMs(etapa.ms),
                                    total: formatMs(total),
                                })}
                            />
                        )}
                    </li>
                ))}
            </ul>
            <dl className="poph-why__lines">
                <div>
                    <dt>{t("testChat.fieldToolRounds")}</dt>
                    <dd>{times.roundsTool ?? t("testChat.notRecorded")}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldModel")}</dt>
                    <dd>{times.modelo ?? t("testChat.notRecorded")}</dd>
                </div>
                <div>
                    <dt>{t("testChat.fieldEndReason")}</dt>
                    <dd>{endReasonLabel(times.reasonEnd)}</dd>
                </div>
            </dl>
            <p className="poph-why__not-stored">{t("testChat.timesNotRecorded")}</p>
        </section>
    );
}

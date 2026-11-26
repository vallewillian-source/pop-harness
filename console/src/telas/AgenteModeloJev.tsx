/**
 * Aba Modelo e Jev do agente (UX_SPEC §3.5, HR-45): provedor e modelo, parâmetros de geração, busca
 * na base (RAG), a janela de agrupamento, os limites de ferramenta e os oito portões do Jev.
 *
 * O que a tela grava vai para o mesmo `PATCH` do agente que as outras abas usam; o efeito vale no
 * **próximo turno**, porque o turno confere a versão do agente antes de cada mensagem (ver
 * `ConfigQuente::carregar_verificando`). Detalhes de contrato e limites em `api/modeloJev.ts` e em
 * docs/CONSOLE.md (seção HR-45).
 *
 * Gravação com `If-Match` da versão do agente. Um `412` relê o agente, adota a versão atual e
 * **mantém o que foi digitado**; o próximo salvamento manda só o que o operador mudou em relação ao
 * que ele viu.
 *
 * Re-layout (RL-12): os cartões viraram seções planas (`SecaoDoAgente`, título + linha de 1 px) e o
 * cabeçalho antigo deu lugar ao comum das abas (`CabecalhoDoAgente`, RL-09). A barra de salvar e a
 * de alterações não salvas ficam no pé da página, como antes. Nenhum campo, validação, `412` ou
 * consulta mudou.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconRefresh,
    SofaBanner,
    SofaButton,
    SofaErrorState,
    SofaField,
    type SofaFieldControlProps,
    SofaInput,
    SofaSelect,
    SofaSkeleton,
    SofaSwitch,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { getAgent, type Agente } from "../api/agents";
import { getOrg } from "../api/orgs";
import {
    PRESERVED_KEYS,
    type ModelField,
    GATE_DESCRIPTION,
    type GateState,
    GATES,
    type IdGate,
    LIMIT_CONTEXT_MIN,
    LIMIT_CUT_MAX,
    LIMIT_CUT_MIN,
    LIMIT_TOOLS_MAX,
    LIMIT_WINDOW_MS,
    LIMIT_ROUNDS_MAX,
    LIMIT_TEMPERATURE_MAX,
    LIMIT_TEMPERATURE_MIN,
    LIMIT_TOP_K_MAX,
    LIMIT_TOP_K_MIN,
    type TextModel,
    FACTORY_DEFAULTS,
    type JevValues,
    gateEffect,
    agentJev,
    agentModel,
    jevChange,
    modelChanges,
    saveModelJev,
    validateModel,
} from "../api/modelJev";
import { TabsAgent, HeaderAgent, SectionAgent } from "../components/TabsAgent";
import { useGuardChanges } from "../components/GuardChanges";
import { translate, useI18n } from "../i18n";
import "../styles/model-jev.css";

function ligacoes(controle: SofaFieldControlProps) {
    const { describedBy, invalid, required, id } = controle;
    return { id, "aria-describedby": describedBy, invalid, required };
}

/** Mesmo conteúdo de dois modelos (texto digitado, com espaços das pontas ignorados). */
function mesmoModelo(a: TextModel, b: TextModel): boolean {
    return (Object.keys(a) as ModelField[]).every((campo) => a[campo].trim() === b[campo].trim());
}

function mesmoJev(a: JevValues, b: JevValues): boolean {
    return a.jevDesligado === b.jevDesligado && GATES.every((gate) => a.gates[gate] === b.gates[gate]);
}

/** Texto curto do que o gate faz neste agente agora, com a origem do valor. */
function situacaoDoGate(valores: JevValues, gate: IdGate): string {
    const ligado = gateEffect(valores, gate);
    if (valores.jevDesligado) return translate("modelJev.gateStatusDisabledBySwitch");
    if (valores.gates[gate] === "herdar") {
        return translate("modelJev.gateStatusInherit", {
            state: ligado ? translate("modelJev.gateStateOnLower") : translate("modelJev.gateStateOffLower"),
        });
    }
    return ligado
        ? translate("modelJev.gateStatusActiveInAgent")
        : translate("modelJev.gateStatusInactiveInAgent");
}

/** Chaves do `jev_json` que estão definidas no agente (para a lista de preservadas). */
function chavesDefinidas(jev: Record<string, unknown> | undefined): Set<string> {
    return new Set(Object.keys(jev ?? {}));
}

export function AgentModelJev() {
    const { translate: t } = useI18n();
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardChanges();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [versao, setVersao] = useState<number>(0);
    const [baseModelo, setBaseModelo] = useState<TextModel | null>(null);
    const [rascunhoModelo, setRascunhoModelo] = useState<TextModel | null>(null);
    const [baseJev, setBaseJev] = useState<JevValues | null>(null);
    const [rascunhoJev, setRascunhoJev] = useState<JevValues | null>(null);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [erros, setErros] = useState<Partial<Record<ModelField, string>>>({});
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [conflito, setConflito] = useState<string | null>(null);
    const [aviso, setAviso] = useState<string | null>(null);
    const [salvando, setSalvando] = useState(false);

    const aplicarAgente = useCallback((dados: Agente) => {
        setAgente(dados);
        setVersao(dados.version);
        const modelo = agentModel(dados);
        const jev = agentJev(dados.jev);
        setBaseModelo(modelo);
        setRascunhoModelo(modelo);
        setBaseJev(jev);
        setRascunhoJev(jev);
    }, []);

    const carregar = useCallback(async () => {
        setErroDeCarga(null);
        try {
            aplicarAgente(await getAgent(orgId, agentId));
        } catch (falha) {
            setErroDeCarga(
                falha instanceof Error ? falha.message : t("modelJev.loadAgentFallbackError"),
            );
        }
    }, [orgId, agentId, aplicarAgente, t]);

    useEffect(() => {
        void carregar();
        getOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
    }, [carregar, orgId]);

    const sujo =
        baseModelo !== null &&
        rascunhoModelo !== null &&
        baseJev !== null &&
        rascunhoJev !== null &&
        (!mesmoModelo(baseModelo, rascunhoModelo) || !mesmoJev(baseJev, rascunhoJev));

    /**
     * Depois de um 412: relê o agente (nova versão e `jev_json` atual). O que o operador digitou e a
     * base que ele viu **não mudam**, então o próximo salvamento continua mandando só a diferença.
     */
    const releraAgente = useCallback(async (): Promise<number | undefined> => {
        try {
            const atual = await getAgent(orgId, agentId);
            setAgente(atual);
            setVersao(atual.version);
            return atual.version;
        } catch {
            return undefined;
        }
    }, [orgId, agentId]);

    const salvar = useCallback(async (): Promise<boolean> => {
        if (!agente || !baseModelo || !rascunhoModelo || !baseJev || !rascunhoJev) return true;
        const novosErros = validateModel(rascunhoModelo, baseModelo);
        setErros(novosErros);
        setErroGeral(null);
        setConflito(null);
        setAviso(null);
        if (Object.keys(novosErros).length > 0) return false;

        const mudancaModelo = modelChanges(baseModelo, rascunhoModelo);
        const jevJson = jevChange(agente.jev, baseJev, rascunhoJev);
        const mudanca = jevJson === undefined ? mudancaModelo : { ...mudancaModelo, jev_json: jevJson };
        if (Object.keys(mudanca).length === 0) {
            setAviso(t("modelJev.nothingToSave"));
            return true;
        }

        setSalvando(true);
        try {
            const gravado = await saveModelJev(orgId, agentId, versao, mudanca);
            aplicarAgente(gravado);
            setAviso(t("modelJev.savedNotice", { version: gravado.version }));
            return true;
        } catch (falha) {
            if (falha instanceof ApiError && falha.status === 412) {
                const atual = await releraAgente();
                setConflito(
                    t("modelJev.conflictMessage", {
                        versionSuffix:
                            typeof atual === "number"
                                ? t("modelJev.conflictVersionSuffix", { version: atual })
                                : "",
                    }),
                );
                return false;
            }
            if (falha instanceof ApiError) {
                const detalhes = falha.detalhes as Record<string, unknown> | null | undefined;
                const campo = typeof detalhes?.["field"] === "string" ? detalhes["field"] : "";
                const motivo = typeof detalhes?.["reason"] === "string" ? detalhes["reason"] : null;
                const campos = baseModelo ? (Object.keys(baseModelo) as string[]) : [];
                if (motivo && campos.includes(campo)) {
                    setErros((atuais) => ({ ...atuais, [campo]: motivo }));
                    return false;
                }
                setErroGeral(
                    motivo
                        ? t("modelJev.apiErrorWithReason", {
                              message: falha.message,
                              reason: motivo,
                              code: falha.codigo,
                          })
                        : t("modelJev.apiErrorWithoutReason", {
                              message: falha.message,
                              code: falha.codigo,
                          }),
                );
                return false;
            }
            setErroGeral(t("modelJev.unexpectedSaveError"));
            return false;
        } finally {
            setSalvando(false);
        }
    }, [agente, baseModelo, rascunhoModelo, baseJev, rascunhoJev, versao, orgId, agentId, aplicarAgente, releraAgente, t]);

    // A guarda de alterações precisa da versão mais recente de `salvar`.
    useEffect(() => {
        if (!rascunhoModelo) return;
        guarda.registrar({ dirty: sujo, salvar: async () => salvar() });
    }, [guarda, sujo, salvar, rascunhoModelo]);

    useEffect(() => {
        return () => guarda.registrar(null);
    }, [guarda]);

    // Ctrl+S / Cmd+S grava quando há alteração pendente.
    useEffect(() => {
        function aoTeclar(evento: KeyboardEvent) {
            if ((evento.ctrlKey || evento.metaKey) && evento.key.toLowerCase() === "s") {
                evento.preventDefault();
                if (sujo && !salvando) void salvar();
            }
        }
        window.addEventListener("keydown", aoTeclar);
        return () => window.removeEventListener("keydown", aoTeclar);
    }, [sujo, salvando, salvar]);

    function alterarModelo(campo: ModelField, valor: string) {
        setRascunhoModelo((atual) => (atual ? { ...atual, [campo]: valor } : atual));
        setErros((atuais) => ({ ...atuais, [campo]: undefined }));
        setAviso(null);
    }

    function alterarGate(gate: IdGate, estado: GateState) {
        setRascunhoJev((atual) =>
            atual ? { ...atual, gates: { ...atual.gates, [gate]: estado } } : atual,
        );
        setAviso(null);
    }

    function alterarInterruptor(desligado: boolean) {
        setRascunhoJev((atual) => (atual ? { ...atual, jevDesligado: desligado } : atual));
        setAviso(null);
    }

    function descartar() {
        if (!baseModelo || !baseJev) return;
        setRascunhoModelo(baseModelo);
        setRascunhoJev(baseJev);
        setErros({});
        setErroGeral(null);
        setConflito(null);
        setAviso(t("modelJev.discardedNotice"));
    }

    const chavesNoAgente = useMemo(() => chavesDefinidas(agente?.jev), [agente]);

    if (!agente && erroDeCarga) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("modelJev.loadErrorTitle")}
                    description={t("modelJev.loadErrorDescription")}
                    details={erroDeCarga}
                    retry={{ label: t("modelJev.retry"), onClick: () => void carregar() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guarda.tentarNavegar(`/orgs/${orgId}/agents`)}
                        >
                            {t("modelJev.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente || !rascunhoModelo || !rascunhoJev) {
        return (
            <div className="poph-page" aria-busy="true">
                <div className="poph-page__top">
                    <div>
                        <SofaSkeleton width={260} height={26} />
                        <div className="poph-model-jev__skeleton-line">
                            <SofaSkeleton width={360} height={12} />
                        </div>
                    </div>
                </div>
                <SofaSkeleton lines={8} />
            </div>
        );
    }

    function campoTexto(
        campo: ModelField,
        rotulo: string,
        opcoes: { dica?: string; placeholder?: string; modo?: "decimal" | "numeric" } = {},
    ) {
        return (
            <SofaField
                label={rotulo}
                hint={opcoes.dica}
                error={erros[campo]}
                htmlFor={`poph-modelo-${campo}`}
            >
                {(controle) => (
                    <SofaInput
                        {...ligacoes(controle)}
                        value={rascunhoModelo![campo]}
                        placeholder={opcoes.placeholder}
                        inputMode={opcoes.modo}
                        autoComplete="off"
                        onChange={(evento) => alterarModelo(campo, evento.target.value)}
                    />
                )}
            </SofaField>
        );
    }

    return (
        <>
            <div className="poph-page poph-model-jev">
                <HeaderAgent
                    org={orgId}
                    nome={agente.name}
                    slug={agente.slug}
                    situacao={agente.status}
                    organizacao={nomeDaOrg}
                    descricao={t("modelJev.headerDescription")}
                />

                <TabsAgent org={orgId} agente={agentId} active="model-jev" nome={agente.name} model={agente.model ?? null} />

                {conflito && (
                    <SofaBanner tone="warn" title={t("modelJev.bannerOutdatedTitle")}>
                        {conflito}
                    </SofaBanner>
                )}
                {erroGeral && (
                    <SofaBanner tone="danger" title={t("modelJev.bannerSaveErrorTitle")}>
                        {erroGeral}
                    </SofaBanner>
                )}
                {aviso && !erroGeral && !conflito && (
                    <p className="poph-mono-muted" style={{ margin: 0 }} role="status">
                        {aviso}
                    </p>
                )}

                <form
                    id="poph-form-modelo-jev"
                    className="poph-model-jev__form"
                    onSubmit={(evento) => {
                        evento.preventDefault();
                        void salvar();
                    }}
                    noValidate
                >
                    <SectionAgent
                        titulo={t("modelJev.sectionModelTitle")}
                        descricao={t("modelJev.sectionModelDescription")}
                    >
                        <div className="poph-model-jev__grid">
                            {campoTexto("provider", t("modelJev.fieldProviderLabel"), {
                                dica: t("modelJev.fieldProviderHint"),
                            })}
                            {campoTexto("model", t("modelJev.fieldModelLabel"), {
                                dica: t("modelJev.fieldModelHint"),
                            })}
                            {campoTexto("temperature", t("modelJev.fieldTemperatureLabel"), {
                                modo: "decimal",
                                placeholder:
                                    agente.temperature == null
                                        ? t("modelJev.placeholderInheritsDefault")
                                        : undefined,
                                dica: t("modelJev.fieldTemperatureHint", {
                                    min: LIMIT_TEMPERATURE_MIN,
                                    max: LIMIT_TEMPERATURE_MAX,
                                }),
                            })}
                            {campoTexto("max_output_tokens", t("modelJev.fieldMaxOutputLabel"), {
                                modo: "numeric",
                                placeholder:
                                    agente.max_output_tokens == null
                                        ? t("modelJev.placeholderInheritsDefault")
                                        : undefined,
                                dica: t("modelJev.fieldMaxOutputHint"),
                            })}
                            {campoTexto("max_context_tokens", t("modelJev.fieldMaxContextLabel"), {
                                modo: "numeric",
                                dica: t("modelJev.fieldMaxContextHint", { min: LIMIT_CONTEXT_MIN }),
                            })}
                        </div>
                    </SectionAgent>

                    <SectionAgent
                        titulo={t("modelJev.sectionServiceTitle")}
                        descricao={t("modelJev.sectionServiceDescription")}
                    >
                        <div className="poph-model-jev__grid">
                            {campoTexto("grouping_window_ms", t("modelJev.fieldGroupingWindowLabel"), {
                                modo: "numeric",
                                dica: t("modelJev.fieldGroupingWindowHint", { max: LIMIT_WINDOW_MS }),
                            })}
                            {campoTexto("max_tool_rounds", t("modelJev.fieldMaxToolRoundsLabel"), {
                                modo: "numeric",
                                dica: t("modelJev.fieldMaxToolRoundsHint", { max: LIMIT_ROUNDS_MAX }),
                            })}
                            {campoTexto("max_tool_calls", t("modelJev.fieldMaxToolCallsLabel"), {
                                modo: "numeric",
                                dica: t("modelJev.fieldMaxToolCallsHint", { max: LIMIT_TOOLS_MAX }),
                            })}
                        </div>
                    </SectionAgent>

                    <SectionAgent
                        titulo={t("modelJev.sectionRagTitle")}
                        descricao={t("modelJev.sectionRagDescription")}
                    >
                        <div className="poph-model-jev__grid">
                            {campoTexto("rag_top_k", t("modelJev.fieldRagTopKLabel"), {
                                modo: "numeric",
                                dica: t("modelJev.fieldRagTopKHint", {
                                    min: LIMIT_TOP_K_MIN,
                                    max: LIMIT_TOP_K_MAX,
                                }),
                            })}
                            {campoTexto("rag_min_score", t("modelJev.fieldRagMinScoreLabel"), {
                                modo: "decimal",
                                dica: t("modelJev.fieldRagMinScoreHint", {
                                    min: LIMIT_CUT_MIN,
                                    max: LIMIT_CUT_MAX,
                                }),
                            })}
                        </div>
                    </SectionAgent>

                    <SectionAgent
                        titulo={t("modelJev.sectionJevTitle")}
                        descricao={t("modelJev.sectionJevDescription")}
                    >
                        <SofaSwitch
                            label={t("modelJev.switchDisableJevLabel")}
                            description={t("modelJev.switchDisableJevDescription")}
                            checked={rascunhoJev.jevDesligado}
                            onChange={(evento) => alterarInterruptor(evento.target.checked)}
                        />

                        <div className="poph-model-jev__grid">
                            {GATES.map((gate) => {
                                const descricao = GATE_DESCRIPTION[gate];
                                return (
                                    <SofaField
                                        key={gate}
                                        label={descricao.titulo}
                                        hint={t("modelJev.gateHintToday", {
                                            explanation: descricao.explicacao,
                                            status: situacaoDoGate(rascunhoJev, gate),
                                        })}
                                        htmlFor={`poph-gate-${gate}`}
                                    >
                                        {(controle) => (
                                            <SofaSelect
                                                {...ligacoes(controle)}
                                                value={rascunhoJev.gates[gate]}
                                                onChange={(evento) =>
                                                    alterarGate(gate, evento.target.value as GateState)
                                                }
                                            >
                                                <option value="herdar">
                                                    {t("modelJev.gateOptionInherit", {
                                                        defaultState: FACTORY_DEFAULTS[gate]
                                                            ? t("modelJev.gateStateOnLower")
                                                            : t("modelJev.gateStateOffLower"),
                                                    })}
                                                </option>
                                                <option value="ligado">{t("modelJev.gateOptionOn")}</option>
                                                <option value="desligado">{t("modelJev.gateOptionOff")}</option>
                                            </SofaSelect>
                                        )}
                                    </SofaField>
                                );
                            })}
                        </div>
                    </SectionAgent>

                    <SectionAgent
                        titulo={t("modelJev.sectionOtherKeysTitle")}
                        descricao={t("modelJev.sectionOtherKeysDescription")}
                    >
                        <ul className="poph-model-jev__keys">
                            {PRESERVED_KEYS.map((item) => (
                                <li key={item.chave}>
                                    <code>{item.chave}</code> ·{" "}
                                    {chavesNoAgente.has(item.chave)
                                        ? t("modelJev.keyDefined")
                                        : t("modelJev.keyUndefined")}
                                    <div className="poph-mono-muted">
                                        {item.explicacao}
                                        {item.ondeEdita
                                            ? t("modelJev.keyEditIn", { where: item.ondeEdita })
                                            : ""}
                                    </div>
                                </li>
                            ))}
                        </ul>
                    </SectionAgent>
                </form>

                <div className="poph-model-jev__bar">
                    <span
                        className={`poph-status ${sujo ? "poph-status--attention" : "poph-status--inactive"}`}
                    >
                        <span className="poph-status__point" aria-hidden="true" />
                        {sujo
                            ? t("modelJev.statusUnsaved")
                            : t("modelJev.statusSaved", { version: versao })}
                    </span>
                    <SofaButton variant="subtle" disabled={!sujo || salvando} onClick={descartar}>
                        {t("modelJev.discardChanges")}
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        icon={<IconRefresh aria-hidden="true" />}
                        disabled={!sujo}
                        loading={salvando}
                        onClick={() => void salvar()}
                    >
                        {t("modelJev.saveButton")}
                    </SofaButton>
                </div>
            </div>
        </>
    );
}

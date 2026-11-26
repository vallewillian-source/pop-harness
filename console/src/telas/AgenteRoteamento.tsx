/**
 * Aba Roteamento do agente (UX_SPEC §3.5, HR-43): para onde a conversa vai e quando passa a humano.
 *
 * Três blocos: departamento padrão (onde a conversa começa), transferência humana (liga/desliga e
 * o departamento de destino) e motivos de transferência (política que o Jev lê, até 15 de 300
 * caracteres). Cada item corresponde a um campo que o motor realmente lê — ver `api/roteamento.ts`.
 *
 * Gravação com `If-Match` da versão do agente. Um `412` relê o agente, passa a usar a versão atual e
 * **mantém o que foi digitado**: só o que o operador mudou é enviado no próximo salvamento, então
 * uma mudança feita por outra pessoa em outro campo não é desfeita por engano.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconPlus,
    IconRefresh,
    IconTrash,
    SofaBanner,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaField,
    type SofaFieldControlProps,
    SofaIconButton,
    SofaInput,
    SofaSelect,
    SofaSkeleton,
    SofaSwitch,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { getAgent, type Agente } from "../api/agents";
import { listStructure, type Departamento } from "../api/structure";
import { getOrg } from "../api/orgs";
import {
    LIMIT_REASON,
    LIMIT_REASONS,
    jevWithReasons,
    cleanReasons,
    agentRouting,
    saveRouting,
    validateRouting,
    type RoutingChange,
    type Routing,
} from "../api/routing";
import {
    TabsAgent,
    HeaderAgent,
    SectionAgent,
} from "../components/TabsAgent";
import { useGuardChanges } from "../components/GuardChanges";
import { useI18n } from "../i18n";
import "../styles/routing.css";

/** Compara dois roteamentos como a tela os grava (motivos sem espaços vazios). */
function mesmoRoteamento(a: Routing, b: Routing): boolean {
    return (
        a.departamentoPadrao === b.departamentoPadrao &&
        a.transferenciaHumana === b.transferenciaHumana &&
        a.departamentoHumano === b.departamentoHumano &&
        JSON.stringify(cleanReasons(a.motivos)) === JSON.stringify(cleanReasons(b.motivos))
    );
}

/**
 * Monta só o que mudou em relação ao último salvamento visto pelo operador. Motivos vão como
 * `jev_json` inteiro, com as outras chaves do agente (ver `jevComMotivos`).
 */
export function mudancaDoRoteamento(
    salvo: Routing,
    rascunho: Routing,
    jevAtual: Record<string, unknown> | undefined,
): RoutingChange {
    const mudanca: RoutingChange = {};
    if (rascunho.departamentoPadrao && rascunho.departamentoPadrao !== salvo.departamentoPadrao) {
        mudanca.department = rascunho.departamentoPadrao;
    }
    if (rascunho.transferenciaHumana !== salvo.transferenciaHumana) {
        mudanca.handoff = rascunho.transferenciaHumana;
    }
    if (rascunho.departamentoHumano && rascunho.departamentoHumano !== salvo.departamentoHumano) {
        mudanca.handoff_department = rascunho.departamentoHumano;
    }
    if (JSON.stringify(cleanReasons(rascunho.motivos)) !== JSON.stringify(cleanReasons(salvo.motivos))) {
        mudanca.jev_json = jevWithReasons(jevAtual, cleanReasons(rascunho.motivos));
    }
    return mudanca;
}

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function ligacoes(controle: SofaFieldControlProps) {
    const { describedBy, invalid, required, id } = controle;
    return { id, "aria-describedby": describedBy, invalid, required };
}

/** Campo da API → chave de erro do formulário. */
const CAMPOS_DA_API: Record<string, string> = {
    department: "departamentoPadrao",
    handoff: "transferenciaHumana",
    handoff_department: "departamentoHumano",
    jev_json: "motivos",
};

export function AgentRouting() {
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardChanges();
    const { translate: t } = useI18n();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [salvo, setSalvo] = useState<Routing | null>(null);
    const [rascunho, setRascunho] = useState<Routing | null>(null);
    const [versao, setVersao] = useState<number>(0);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [departamentos, setDepartamentos] = useState<Departamento[]>([]);
    const [carregandoDepartamentos, setCarregandoDepartamentos] = useState(true);
    const [erroDepartamentos, setErroDepartamentos] = useState<string | null>(null);

    const [erros, setErros] = useState<Record<string, string>>({});
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [conflito, setConflito] = useState<string | null>(null);
    const [aviso, setAviso] = useState<string | null>(null);
    const [salvando, setSalvando] = useState(false);

    const carregarDepartamentos = useCallback(async () => {
        setCarregandoDepartamentos(true);
        setErroDepartamentos(null);
        try {
            const lista = await listStructure("departamento", orgId);
            setDepartamentos(lista as Departamento[]);
        } catch (falha) {
            setErroDepartamentos(
                falha instanceof Error ? falha.message : t("routing.loadDepartmentsFailed"),
            );
            setDepartamentos([]);
        } finally {
            setCarregandoDepartamentos(false);
        }
    }, [orgId, t]);

    const carregar = useCallback(async () => {
        setErroDeCarga(null);
        try {
            const dados = await getAgent(orgId, agentId);
            const roteamento = agentRouting(dados);
            setAgente(dados);
            setVersao(dados.version);
            setSalvo(roteamento);
            setRascunho(roteamento);
        } catch (falha) {
            setErroDeCarga(falha instanceof Error ? falha.message : t("routing.loadAgentFailed"));
            return;
        }
        void carregarDepartamentos();
    }, [orgId, agentId, carregarDepartamentos]);

    useEffect(() => {
        void carregar();
        getOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
    }, [carregar, orgId]);

    const sujo = salvo !== null && rascunho !== null && !mesmoRoteamento(salvo, rascunho);

    /** Lê o agente de novo depois de um 412: nova versão e jev atual; o rascunho não é tocado. */
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
        if (!rascunho || !salvo || !agente) return true;
        const novosErros = validateRouting(rascunho);
        setErros(novosErros);
        setErroGeral(null);
        setConflito(null);
        setAviso(null);
        if (Object.keys(novosErros).length > 0) return false;

        const mudanca = mudancaDoRoteamento(salvo, rascunho, agente.jev);
        if (Object.keys(mudanca).length === 0) {
            setAviso(t("routing.nothingToSave"));
            return true;
        }

        setSalvando(true);
        try {
            const gravado = await saveRouting(orgId, agentId, versao, mudanca);
            setAgente(gravado);
            setVersao(gravado.version);
            const novo = agentRouting(gravado);
            setSalvo(novo);
            setRascunho(novo);
            setAviso(t("routing.savedVersion", { version: gravado.version }));
            return true;
        } catch (falha) {
            if (falha instanceof ApiError && falha.status === 412) {
                const atual = await releraAgente();
                setConflito(
                    typeof atual === "number"
                        ? t("routing.conflictMessageVersion", { version: atual })
                        : t("routing.conflictMessage"),
                );
                return false;
            }
            if (falha instanceof ApiError) {
                const detalhes = falha.detalhes as Record<string, unknown> | null | undefined;
                const campo = typeof detalhes?.["field"] === "string" ? detalhes["field"] : "";
                const motivo =
                    typeof detalhes?.["reason"] === "string" ? detalhes["reason"] : null;
                const chave = CAMPOS_DA_API[campo];
                if (chave && motivo) {
                    setErros((atuais) => ({ ...atuais, [chave]: motivo }));
                    return false;
                }
                setErroGeral(
                    motivo
                        ? t("routing.errorWithDetails", {
                              message: falha.message,
                              reason: motivo,
                              code: falha.codigo,
                          })
                        : t("routing.errorWithCode", {
                              message: falha.message,
                              code: falha.codigo,
                          }),
                );
                return false;
            }
            setErroGeral(t("routing.unexpectedSaveError"));
            return false;
        } finally {
            setSalvando(false);
        }
    }, [rascunho, salvo, agente, versao, orgId, agentId, releraAgente, t]);

    // A guarda de alterações precisa da versão mais recente de `salvar`.
    useEffect(() => {
        if (!rascunho) return;
        guarda.registrar({ dirty: sujo, salvar: async () => salvar() });
    }, [guarda, sujo, salvar, rascunho]);

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

    const opcoesDeDepartamento = useMemo(() => {
        const opcoes = departamentos.map((departamento) => ({
            valor: departamento.id,
            rotulo:
                departamento.status === "active"
                    ? departamento.name
                    : t("routing.departmentInactive", { name: departamento.name }),
        }));
        // Departamento guardado no agente que não existe mais na lista (apagado ou de outra org).
        for (const id of [rascunho?.departamentoPadrao, rascunho?.departamentoHumano]) {
            if (id && !opcoes.some((opcao) => opcao.valor === id)) {
                opcoes.push({ valor: id, rotulo: t("routing.departmentNotFound") });
            }
        }
        return opcoes;
    }, [departamentos, rascunho?.departamentoPadrao, rascunho?.departamentoHumano, t]);

    function alterar(parcial: Partial<Routing>) {
        setRascunho((atual) => (atual ? { ...atual, ...parcial } : atual));
        setAviso(null);
    }

    function alterarMotivo(indice: number, texto: string) {
        if (!rascunho) return;
        const motivos = rascunho.motivos.map((motivo, posicao) =>
            posicao === indice ? texto : motivo,
        );
        alterar({ motivos });
    }

    function removerMotivo(indice: number) {
        if (!rascunho) return;
        alterar({ motivos: rascunho.motivos.filter((_, posicao) => posicao !== indice) });
    }

    function adicionarMotivo() {
        if (!rascunho) return;
        alterar({ motivos: [...rascunho.motivos, ""] });
    }

    function descartar() {
        if (!salvo) return;
        setRascunho(salvo);
        setErros({});
        setErroGeral(null);
        setConflito(null);
        setAviso(t("routing.discarded"));
    }

    if (!agente && erroDeCarga) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("routing.loadErrorTitle")}
                    description={t("routing.loadErrorDescription")}
                    details={erroDeCarga}
                    retry={{ label: t("routing.retry"), onClick: () => void carregar() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guarda.tentarNavegar(`/orgs/${orgId}/agents`)}
                        >
                            {t("routing.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente || !rascunho) {
        return (
            <div className="poph-page" aria-busy="true">
                <SofaSkeleton lines={6} />
            </div>
        );
    }

    const semDepartamentos = !carregandoDepartamentos && !erroDepartamentos && departamentos.length === 0;
    const motivosCheios = rascunho.motivos.length >= LIMIT_REASONS;

    return (
        <div className="poph-page">
            <HeaderAgent
                org={orgId}
                nome={agente.name}
                slug={agente.slug}
                situacao={agente.status}
                organizacao={nomeDaOrg}
                descricao={t("routing.headerDescription")}
            />

            <TabsAgent org={orgId} agente={agentId} active="routing" nome={agente.name} model={agente.model ?? null} />

            {conflito && (
                <SofaBanner tone="warn" title={t("routing.conflictTitle")}>
                    {conflito}
                </SofaBanner>
            )}
            {erroGeral && (
                <SofaBanner tone="danger" title={t("routing.saveErrorTitle")}>
                    {erroGeral}
                </SofaBanner>
            )}
            {aviso && !erroGeral && !conflito && (
                <p className="poph-mono-muted" style={{ margin: 0 }} role="status">
                    {aviso}
                </p>
            )}

            {erroDepartamentos && (
                <SofaBanner tone="danger" title={t("routing.departmentsErrorTitle")}>
                    {erroDepartamentos}.{" "}
                    <SofaButton variant="subtle" size="sm" onClick={() => void carregarDepartamentos()}>
                        {t("routing.retry")}
                    </SofaButton>
                </SofaBanner>
            )}

            <SectionAgent
                titulo={t("routing.sectionDepartments")}
                descricao={t("routing.sectionDepartmentsDescription")}
            >
                {semDepartamentos ? (
                    <SofaEmptyState
                        title={t("routing.emptyDepartmentsTitle")}
                        description={t("routing.emptyDepartmentsDescription")}
                        action={
                            <SofaButton
                                variant="primary"
                                icon={<IconPlus aria-hidden="true" />}
                                onClick={() => guarda.tentarNavegar(`/orgs/${orgId}/structure`)}
                            >
                                {t("routing.goToStructure")}
                            </SofaButton>
                        }
                    />
                ) : carregandoDepartamentos && departamentos.length === 0 ? (
                    <SofaSkeleton lines={3} />
                ) : (
                    <form
                        id="poph-form-roteamento"
                        className="poph-stack"
                        style={{ gap: "var(--pop-spacing-lg)" }}
                        onSubmit={(evento) => {
                            evento.preventDefault();
                            void salvar();
                        }}
                        noValidate
                    >
                        <SofaField
                            label={t("routing.defaultDepartmentLabel")}
                            hint={t("routing.defaultDepartmentHint")}
                            error={erros["departamentoPadrao"]}
                            htmlFor="poph-roteamento-padrao"
                        >
                            {(controle) => (
                                <SofaSelect
                                    {...ligacoes(controle)}
                                    value={rascunho.departamentoPadrao ?? ""}
                                    onChange={(evento) =>
                                        alterar({ departamentoPadrao: evento.target.value || null })
                                    }
                                >
                                    {rascunho.departamentoPadrao === null && (
                                        <option value="">{t("routing.chooseDepartment")}</option>
                                    )}
                                    {opcoesDeDepartamento.map((opcao) => (
                                        <option key={opcao.valor} value={opcao.valor}>
                                            {opcao.rotulo}
                                        </option>
                                    ))}
                                </SofaSelect>
                            )}
                        </SofaField>

                        <SofaSwitch
                            label={t("routing.handoffLabel")}
                            description={t("routing.handoffDescription")}
                            checked={rascunho.transferenciaHumana}
                            onChange={(evento) =>
                                alterar({ transferenciaHumana: evento.target.checked })
                            }
                        />

                        <SofaField
                            label={t("routing.humanDepartmentLabel")}
                            hint={t("routing.humanDepartmentHint")}
                            error={erros["departamentoHumano"]}
                            htmlFor="poph-roteamento-humano"
                        >
                            {(controle) => (
                                <SofaSelect
                                    {...ligacoes(controle)}
                                    value={rascunho.departamentoHumano ?? ""}
                                    onChange={(evento) =>
                                        alterar({ departamentoHumano: evento.target.value || null })
                                    }
                                >
                                    {rascunho.departamentoHumano === null && (
                                        <option value="">{t("routing.chooseDepartment")}</option>
                                    )}
                                    {opcoesDeDepartamento.map((opcao) => (
                                        <option key={opcao.valor} value={opcao.valor}>
                                            {opcao.rotulo}
                                        </option>
                                    ))}
                                </SofaSelect>
                            )}
                        </SofaField>
                    </form>
                )}
            </SectionAgent>

            <SectionAgent
                titulo={t("routing.sectionReasons")}
                descricao={t("routing.sectionReasonsDescription", {
                    limit: LIMIT_REASONS,
                    maxLength: LIMIT_REASON,
                })}
                acoes={
                    <SofaButton
                        variant="secondary"
                        size="sm"
                        icon={<IconPlus aria-hidden="true" />}
                        disabled={motivosCheios}
                        onClick={adicionarMotivo}
                    >
                        {t("routing.addReason")}
                    </SofaButton>
                }
            >
                {rascunho.motivos.length === 0 ? (
                    <p className="poph-mono-muted poph-routing__empty">{t("routing.noReasons")}</p>
                ) : (
                    <div className="poph-routing__reasons">
                        {rascunho.motivos.map((motivo, indice) => (
                            <div key={indice} className="poph-routing__reason">
                                <div className="poph-routing__reason-field">
                                    <SofaField
                                        label={t("routing.reasonLabel", { number: indice + 1 })}
                                        hint={`${motivo.trim().length}/${LIMIT_REASON}`}
                                        error={erros[`motivo-${indice}`]}
                                        htmlFor={`poph-motivo-${indice}`}
                                    >
                                        {(controle) => (
                                            <SofaInput
                                                {...ligacoes(controle)}
                                                value={motivo}
                                                autoComplete="off"
                                                onChange={(evento) =>
                                                    alterarMotivo(indice, evento.target.value)
                                                }
                                            />
                                        )}
                                    </SofaField>
                                </div>
                                <SofaIconButton
                                    label={t("routing.removeReasonLabel", { number: indice + 1 })}
                                    onClick={() => removerMotivo(indice)}
                                >
                                    <IconTrash aria-hidden="true" />
                                </SofaIconButton>
                            </div>
                        ))}
                    </div>
                )}
                {erros["motivos"] && (
                    <p role="alert" className="poph-routing__error">
                        {erros["motivos"]}
                    </p>
                )}
            </SectionAgent>

            <div className="poph-line-title poph-routing__bar">
                <span
                    className={`poph-status ${sujo ? "poph-status--attention" : "poph-status--inactive"}`}
                >
                    <span className="poph-status__point" aria-hidden="true" />
                    {sujo
                        ? t("routing.unsavedChanges")
                        : t("routing.savedStatus", { version: versao })}
                </span>
                <SofaButton variant="subtle" disabled={!sujo || salvando} onClick={descartar}>
                    {t("routing.discardChanges")}
                </SofaButton>
                <SofaButton
                    variant="primary"
                    icon={<IconRefresh aria-hidden="true" />}
                    disabled={!sujo}
                    loading={salvando}
                    onClick={() => void salvar()}
                >
                    {t("routing.saveRouting")}
                </SofaButton>
            </div>
        </div>
    );
}

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
import { ErroApi } from "../api/cliente";
import { obterAgente, type Agente } from "../api/agentes";
import { obterOrg } from "../api/orgs";
import {
    CHAVES_PRESERVADAS,
    type CampoModelo,
    DESCRICAO_GATE,
    type EstadoGate,
    GATES,
    type IdGate,
    LIMITE_CONTEXTO_MIN,
    LIMITE_CORTE_MAX,
    LIMITE_CORTE_MIN,
    LIMITE_FERRAMENTAS_MAX,
    LIMITE_JANELA_MS,
    LIMITE_RODADAS_MAX,
    LIMITE_TEMPERATURA_MAX,
    LIMITE_TEMPERATURA_MIN,
    LIMITE_TOP_K_MAX,
    LIMITE_TOP_K_MIN,
    type ModeloTexto,
    PADRAO_DE_FABRICA,
    type ValoresJev,
    efeitoDoGate,
    jevDoAgente,
    modeloDoAgente,
    mudancaDoJev,
    mudancasDoModelo,
    salvarModeloJev,
    validarModelo,
} from "../api/modeloJev";
import { AbasDoAgente, CabecalhoDoAgente, SecaoDoAgente } from "../componentes/AbasDoAgente";
import { useGuardaDeAlteracoes } from "../componentes/GuardaAlteracoes";
import "../estilos/modelo-jev.css";

function ligacoes(controle: SofaFieldControlProps) {
    const { describedBy, invalid, required, id } = controle;
    return { id, "aria-describedby": describedBy, invalid, required };
}

/** Mesmo conteúdo de dois modelos (texto digitado, com espaços das pontas ignorados). */
function mesmoModelo(a: ModeloTexto, b: ModeloTexto): boolean {
    return (Object.keys(a) as CampoModelo[]).every((campo) => a[campo].trim() === b[campo].trim());
}

function mesmoJev(a: ValoresJev, b: ValoresJev): boolean {
    return a.jevDesligado === b.jevDesligado && GATES.every((gate) => a.gates[gate] === b.gates[gate]);
}

/** Texto curto do que o gate faz neste agente agora, com a origem do valor. */
function situacaoDoGate(valores: ValoresJev, gate: IdGate): string {
    const ligado = efeitoDoGate(valores, gate);
    if (valores.jevDesligado) return "desligado pelo interruptor do Jev deste agente";
    if (valores.gates[gate] === "herdar") {
        return `${ligado ? "ligado" : "desligado"} (herda o padrão de fábrica)`;
    }
    return ligado ? "ligado neste agente" : "desligado neste agente";
}

/** Chaves do `jev_json` que estão definidas no agente (para a lista de preservadas). */
function chavesDefinidas(jev: Record<string, unknown> | undefined): Set<string> {
    return new Set(Object.keys(jev ?? {}));
}

export function AgenteModeloJev() {
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardaDeAlteracoes();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [versao, setVersao] = useState<number>(0);
    const [baseModelo, setBaseModelo] = useState<ModeloTexto | null>(null);
    const [rascunhoModelo, setRascunhoModelo] = useState<ModeloTexto | null>(null);
    const [baseJev, setBaseJev] = useState<ValoresJev | null>(null);
    const [rascunhoJev, setRascunhoJev] = useState<ValoresJev | null>(null);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [erros, setErros] = useState<Partial<Record<CampoModelo, string>>>({});
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [conflito, setConflito] = useState<string | null>(null);
    const [aviso, setAviso] = useState<string | null>(null);
    const [salvando, setSalvando] = useState(false);

    const aplicarAgente = useCallback((dados: Agente) => {
        setAgente(dados);
        setVersao(dados.version);
        const modelo = modeloDoAgente(dados);
        const jev = jevDoAgente(dados.jev);
        setBaseModelo(modelo);
        setRascunhoModelo(modelo);
        setBaseJev(jev);
        setRascunhoJev(jev);
    }, []);

    const carregar = useCallback(async () => {
        setErroDeCarga(null);
        try {
            aplicarAgente(await obterAgente(orgId, agentId));
        } catch (falha) {
            setErroDeCarga(falha instanceof Error ? falha.message : "falha ao carregar o agente");
        }
    }, [orgId, agentId, aplicarAgente]);

    useEffect(() => {
        void carregar();
        obterOrg(orgId)
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
            const atual = await obterAgente(orgId, agentId);
            setAgente(atual);
            setVersao(atual.version);
            return atual.version;
        } catch {
            return undefined;
        }
    }, [orgId, agentId]);

    const salvar = useCallback(async (): Promise<boolean> => {
        if (!agente || !baseModelo || !rascunhoModelo || !baseJev || !rascunhoJev) return true;
        const novosErros = validarModelo(rascunhoModelo, baseModelo);
        setErros(novosErros);
        setErroGeral(null);
        setConflito(null);
        setAviso(null);
        if (Object.keys(novosErros).length > 0) return false;

        const mudancaModelo = mudancasDoModelo(baseModelo, rascunhoModelo);
        const jevJson = mudancaDoJev(agente.jev, baseJev, rascunhoJev);
        const mudanca = jevJson === undefined ? mudancaModelo : { ...mudancaModelo, jev_json: jevJson };
        if (Object.keys(mudanca).length === 0) {
            setAviso("Nada para gravar: o modelo e o Jev já estão como estão salvos.");
            return true;
        }

        setSalvando(true);
        try {
            const gravado = await salvarModeloJev(orgId, agentId, versao, mudanca);
            aplicarAgente(gravado);
            setAviso(`Modelo e Jev gravados (versão ${gravado.version}). Vale a partir do próximo turno.`);
            return true;
        } catch (falha) {
            if (falha instanceof ErroApi && falha.status === 412) {
                const atual = await releraAgente();
                setConflito(
                    `Outra pessoa salvou este agente antes de você${
                        typeof atual === "number" ? ` (agora na versão ${atual})` : ""
                    }. O que você digitou foi mantido: revise e salve de novo.`,
                );
                return false;
            }
            if (falha instanceof ErroApi) {
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
                        ? `${falha.message}: ${motivo} (código ${falha.codigo})`
                        : `${falha.message} (código ${falha.codigo})`,
                );
                return false;
            }
            setErroGeral("falha inesperada ao salvar o modelo e o Jev");
            return false;
        } finally {
            setSalvando(false);
        }
    }, [agente, baseModelo, rascunhoModelo, baseJev, rascunhoJev, versao, orgId, agentId, aplicarAgente, releraAgente]);

    // A guarda de alterações precisa da versão mais recente de `salvar`.
    useEffect(() => {
        if (!rascunhoModelo) return;
        guarda.registrar({ sujo, salvar: async () => salvar() });
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

    function alterarModelo(campo: CampoModelo, valor: string) {
        setRascunhoModelo((atual) => (atual ? { ...atual, [campo]: valor } : atual));
        setErros((atuais) => ({ ...atuais, [campo]: undefined }));
        setAviso(null);
    }

    function alterarGate(gate: IdGate, estado: EstadoGate) {
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
        setAviso("Alterações descartadas: voltou ao que está salvo.");
    }

    const chavesNoAgente = useMemo(() => chavesDefinidas(agente?.jev), [agente]);

    if (!agente && erroDeCarga) {
        return (
            <div className="poph-pagina">
                <SofaErrorState
                    title="Não consegui carregar o modelo e o Jev do agente"
                    description="A instância não respondeu à leitura do agente. Tente de novo; se persistir, confira a saúde do servidor."
                    details={erroDeCarga}
                    retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guarda.tentarNavegar(`/orgs/${orgId}/agents`)}
                        >
                            Voltar para agentes
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente || !rascunhoModelo || !rascunhoJev) {
        return (
            <div className="poph-pagina" aria-busy="true">
                <div className="poph-pagina__topo">
                    <div>
                        <SofaSkeleton width={260} height={26} />
                        <div className="poph-modelo-jev__esqueleto-linha">
                            <SofaSkeleton width={360} height={12} />
                        </div>
                    </div>
                </div>
                <SofaSkeleton lines={8} />
            </div>
        );
    }

    function campoTexto(
        campo: CampoModelo,
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
            <div className="poph-pagina poph-modelo-jev">
                <CabecalhoDoAgente
                    org={orgId}
                    nome={agente.name}
                    slug={agente.slug}
                    situacao={agente.status}
                    organizacao={nomeDaOrg}
                    descricao="modelo, busca na base e portões de decisão do Jev"
                />

                <AbasDoAgente org={orgId} agente={agentId} ativa="model-jev" nome={agente.name} />

                {conflito && (
                    <SofaBanner tone="warn" title="Versão desatualizada">
                        {conflito}
                    </SofaBanner>
                )}
                {erroGeral && (
                    <SofaBanner tone="danger" title="Não foi possível salvar">
                        {erroGeral}
                    </SofaBanner>
                )}
                {aviso && !erroGeral && !conflito && (
                    <p className="poph-mono-mudo" style={{ margin: 0 }} role="status">
                        {aviso}
                    </p>
                )}

                <form
                    id="poph-form-modelo-jev"
                    className="poph-modelo-jev__formulario"
                    onSubmit={(evento) => {
                        evento.preventDefault();
                        void salvar();
                    }}
                    noValidate
                >
                    <SecaoDoAgente
                        titulo="Modelo de linguagem"
                        descricao="Quem escreve a resposta e com que limites. Temperatura e máximo de tokens de saída mostram “herda o padrão do servidor” quando o agente ainda não tem valor; depois de definidos, a tela não consegue voltar a esse padrão."
                    >
                        <div className="poph-modelo-jev__grade">
                            {campoTexto("provider", "Provedor", {
                                dica: "Na V1 o provedor é antigravity2api.",
                            })}
                            {campoTexto("model", "Modelo", {
                                dica: "Nome do modelo que escreve a resposta.",
                            })}
                            {campoTexto("temperature", "Temperatura", {
                                modo: "decimal",
                                placeholder:
                                    agente.temperature == null
                                        ? "herda o padrão do servidor"
                                        : undefined,
                                dica: `De ${LIMITE_TEMPERATURA_MIN} a ${LIMITE_TEMPERATURA_MAX}. Mais alto, respostas mais variadas.`,
                            })}
                            {campoTexto("max_output_tokens", "Máximo de tokens de saída", {
                                modo: "numeric",
                                placeholder:
                                    agente.max_output_tokens == null
                                        ? "herda o padrão do servidor"
                                        : undefined,
                                dica: "Tamanho máximo de cada resposta, em tokens.",
                            })}
                            {campoTexto("max_context_tokens", "Máximo de tokens de contexto", {
                                modo: "numeric",
                                dica: `Quanto da conversa e das instruções o turno monta para o modelo. Mínimo ${LIMITE_CONTEXTO_MIN}.`,
                            })}
                        </div>
                    </SecaoDoAgente>

                    <SecaoDoAgente
                        titulo="Atendimento e ferramentas"
                        descricao="Como o turno junta as mensagens do cliente e quantas ferramentas pode usar."
                    >
                        <div className="poph-modelo-jev__grade">
                            {campoTexto("grouping_window_ms", "Janela de agrupamento (ms)", {
                                modo: "numeric",
                                dica: `De 0 a ${LIMITE_JANELA_MS} ms. Mensagens que chegam dentro dessa janela viram uma resposta só.`,
                            })}
                            {campoTexto("max_tool_rounds", "Rodadas de ferramenta por turno", {
                                modo: "numeric",
                                dica: `Quantas vezes o modelo pode chamar ferramentas numa mesma resposta. De 0 a ${LIMITE_RODADAS_MAX}.`,
                            })}
                            {campoTexto("max_tool_calls", "Ferramentas por mensagem", {
                                modo: "numeric",
                                dica: `Limite de chamadas de ferramenta para cada mensagem do cliente. De 0 a ${LIMITE_FERRAMENTAS_MAX}.`,
                            })}
                        </div>
                    </SecaoDoAgente>

                    <SecaoDoAgente
                        titulo="Busca na base (RAG)"
                        descricao="Quantos trechos da base de conhecimento entram na resposta e o mínimo de relevância para aceitar um trecho."
                    >
                        <div className="poph-modelo-jev__grade">
                            {campoTexto("rag_top_k", "Trechos recuperados (top-k)", {
                                modo: "numeric",
                                dica: `De ${LIMITE_TOP_K_MIN} a ${LIMITE_TOP_K_MAX}.`,
                            })}
                            {campoTexto("rag_min_score", "Corte mínimo de relevância", {
                                modo: "decimal",
                                dica: `De ${LIMITE_CORTE_MIN} a ${LIMITE_CORTE_MAX}. Zero aceita qualquer trecho do top-k, mesmo sem relação com a pergunta.`,
                            })}
                        </div>
                    </SecaoDoAgente>

                    <SecaoDoAgente
                        titulo="Jev: portões de decisão"
                        descricao="Cada gate decide uma coisa antes da resposta. “Herdar” usa o padrão de fábrica do código (G3, G5 e G8 ligados; os demais desligados). A configuração do servidor pode ter mudado isso, e esta tela não consegue ler essa configuração."
                    >
                        <SofaSwitch
                            label="Desligar o Jev neste agente"
                            description="Quando ligado, nenhum gate roda neste agente, mesmo com os gates abaixo ligados."
                            checked={rascunhoJev.jevDesligado}
                            onChange={(evento) => alterarInterruptor(evento.target.checked)}
                        />

                        <div className="poph-modelo-jev__grade">
                            {GATES.map((gate) => {
                                const descricao = DESCRICAO_GATE[gate];
                                return (
                                    <SofaField
                                        key={gate}
                                        label={descricao.titulo}
                                        hint={`${descricao.explicacao} Hoje: ${situacaoDoGate(rascunhoJev, gate)}.`}
                                        htmlFor={`poph-gate-${gate}`}
                                    >
                                        {(controle) => (
                                            <SofaSelect
                                                {...ligacoes(controle)}
                                                value={rascunhoJev.gates[gate]}
                                                onChange={(evento) =>
                                                    alterarGate(gate, evento.target.value as EstadoGate)
                                                }
                                            >
                                                <option value="herdar">
                                                    Herdar (padrão de fábrica:{" "}
                                                    {PADRAO_DE_FABRICA[gate] ? "ligado" : "desligado"})
                                                </option>
                                                <option value="ligado">Ligado</option>
                                                <option value="desligado">Desligado</option>
                                            </SofaSelect>
                                        )}
                                    </SofaField>
                                );
                            })}
                        </div>
                    </SecaoDoAgente>

                    <SecaoDoAgente
                        titulo="Outras chaves do Jev (não editadas aqui)"
                        descricao="Estas chaves o motor lê, mas esta aba não as altera. Ao salvar, elas voltam exatamente como estão."
                    >
                        <ul className="poph-modelo-jev__chaves">
                            {CHAVES_PRESERVADAS.map((item) => (
                                <li key={item.chave}>
                                    <code>{item.chave}</code> ·{" "}
                                    {chavesNoAgente.has(item.chave) ? "definida" : "não definida"}
                                    <div className="poph-mono-mudo">
                                        {item.explicacao}
                                        {item.ondeEdita ? ` Edite em ${item.ondeEdita}.` : ""}
                                    </div>
                                </li>
                            ))}
                        </ul>
                    </SecaoDoAgente>
                </form>

                <div className="poph-modelo-jev__barra">
                    <span
                        className={`poph-status ${sujo ? "poph-status--atencao" : "poph-status--inativo"}`}
                    >
                        <span className="poph-status__ponto" aria-hidden="true" />
                        {sujo ? "Alterações não salvas" : `Salvo (versão ${versao})`}
                    </span>
                    <SofaButton variant="subtle" disabled={!sujo || salvando} onClick={descartar}>
                        Descartar alterações
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        icon={<IconRefresh aria-hidden="true" />}
                        disabled={!sujo}
                        loading={salvando}
                        onClick={() => void salvar()}
                    >
                        Salvar modelo e Jev
                    </SofaButton>
                </div>
            </div>
        </>
    );
}

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
import { ErroApi } from "../api/cliente";
import { obterAgente, type Agente } from "../api/agentes";
import { listarEstrutura, type Departamento } from "../api/estrutura";
import { obterOrg } from "../api/orgs";
import {
    LIMITE_MOTIVO,
    LIMITE_MOTIVOS,
    jevComMotivos,
    motivosLimpos,
    roteamentoDoAgente,
    salvarRoteamento,
    validarRoteamento,
    type MudancaRoteamento,
    type Roteamento,
} from "../api/roteamento";
import {
    AbasDoAgente,
    CabecalhoDoAgente,
    SecaoDoAgente,
} from "../componentes/AbasDoAgente";
import { useGuardaDeAlteracoes } from "../componentes/GuardaAlteracoes";
import "../estilos/roteamento.css";

/** Compara dois roteamentos como a tela os grava (motivos sem espaços vazios). */
function mesmoRoteamento(a: Roteamento, b: Roteamento): boolean {
    return (
        a.departamentoPadrao === b.departamentoPadrao &&
        a.transferenciaHumana === b.transferenciaHumana &&
        a.departamentoHumano === b.departamentoHumano &&
        JSON.stringify(motivosLimpos(a.motivos)) === JSON.stringify(motivosLimpos(b.motivos))
    );
}

/**
 * Monta só o que mudou em relação ao último salvamento visto pelo operador. Motivos vão como
 * `jev_json` inteiro, com as outras chaves do agente (ver `jevComMotivos`).
 */
export function mudancaDoRoteamento(
    salvo: Roteamento,
    rascunho: Roteamento,
    jevAtual: Record<string, unknown> | undefined,
): MudancaRoteamento {
    const mudanca: MudancaRoteamento = {};
    if (rascunho.departamentoPadrao && rascunho.departamentoPadrao !== salvo.departamentoPadrao) {
        mudanca.department = rascunho.departamentoPadrao;
    }
    if (rascunho.transferenciaHumana !== salvo.transferenciaHumana) {
        mudanca.handoff = rascunho.transferenciaHumana;
    }
    if (rascunho.departamentoHumano && rascunho.departamentoHumano !== salvo.departamentoHumano) {
        mudanca.handoff_department = rascunho.departamentoHumano;
    }
    if (JSON.stringify(motivosLimpos(rascunho.motivos)) !== JSON.stringify(motivosLimpos(salvo.motivos))) {
        mudanca.jev_json = jevComMotivos(jevAtual, motivosLimpos(rascunho.motivos));
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

export function AgenteRoteamento() {
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardaDeAlteracoes();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [salvo, setSalvo] = useState<Roteamento | null>(null);
    const [rascunho, setRascunho] = useState<Roteamento | null>(null);
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
            const lista = await listarEstrutura("departamento", orgId);
            setDepartamentos(lista as Departamento[]);
        } catch (falha) {
            setErroDepartamentos(
                falha instanceof Error ? falha.message : "falha ao carregar os departamentos",
            );
            setDepartamentos([]);
        } finally {
            setCarregandoDepartamentos(false);
        }
    }, [orgId]);

    const carregar = useCallback(async () => {
        setErroDeCarga(null);
        try {
            const dados = await obterAgente(orgId, agentId);
            const roteamento = roteamentoDoAgente(dados);
            setAgente(dados);
            setVersao(dados.version);
            setSalvo(roteamento);
            setRascunho(roteamento);
        } catch (falha) {
            setErroDeCarga(falha instanceof Error ? falha.message : "falha ao carregar o agente");
            return;
        }
        void carregarDepartamentos();
    }, [orgId, agentId, carregarDepartamentos]);

    useEffect(() => {
        void carregar();
        obterOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
    }, [carregar, orgId]);

    const sujo = salvo !== null && rascunho !== null && !mesmoRoteamento(salvo, rascunho);

    /** Lê o agente de novo depois de um 412: nova versão e jev atual; o rascunho não é tocado. */
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
        if (!rascunho || !salvo || !agente) return true;
        const novosErros = validarRoteamento(rascunho);
        setErros(novosErros);
        setErroGeral(null);
        setConflito(null);
        setAviso(null);
        if (Object.keys(novosErros).length > 0) return false;

        const mudanca = mudancaDoRoteamento(salvo, rascunho, agente.jev);
        if (Object.keys(mudanca).length === 0) {
            setAviso("Nada para gravar: o roteamento já está como está salvo.");
            return true;
        }

        setSalvando(true);
        try {
            const gravado = await salvarRoteamento(orgId, agentId, versao, mudanca);
            setAgente(gravado);
            setVersao(gravado.version);
            const novo = roteamentoDoAgente(gravado);
            setSalvo(novo);
            setRascunho(novo);
            setAviso(`Roteamento gravado (versão ${gravado.version}).`);
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
                const motivo =
                    typeof detalhes?.["reason"] === "string" ? detalhes["reason"] : null;
                const chave = CAMPOS_DA_API[campo];
                if (chave && motivo) {
                    setErros((atuais) => ({ ...atuais, [chave]: motivo }));
                    return false;
                }
                setErroGeral(
                    motivo
                        ? `${falha.message}: ${motivo} (código ${falha.codigo})`
                        : `${falha.message} (código ${falha.codigo})`,
                );
                return false;
            }
            setErroGeral("falha inesperada ao salvar o roteamento");
            return false;
        } finally {
            setSalvando(false);
        }
    }, [rascunho, salvo, agente, versao, orgId, agentId, releraAgente]);

    // A guarda de alterações precisa da versão mais recente de `salvar`.
    useEffect(() => {
        if (!rascunho) return;
        guarda.registrar({ sujo, salvar: async () => salvar() });
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
                    : `${departamento.name} (inativo)`,
        }));
        // Departamento guardado no agente que não existe mais na lista (apagado ou de outra org).
        for (const id of [rascunho?.departamentoPadrao, rascunho?.departamentoHumano]) {
            if (id && !opcoes.some((opcao) => opcao.valor === id)) {
                opcoes.push({ valor: id, rotulo: "departamento não encontrado na organização" });
            }
        }
        return opcoes;
    }, [departamentos, rascunho?.departamentoPadrao, rascunho?.departamentoHumano]);

    function alterar(parcial: Partial<Roteamento>) {
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
        setAviso("Alterações descartadas: voltou ao que está salvo.");
    }

    if (!agente && erroDeCarga) {
        return (
            <div className="poph-pagina">
                <SofaErrorState
                    title="Não consegui carregar o roteamento do agente"
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

    if (!agente || !rascunho) {
        return (
            <div className="poph-pagina" aria-busy="true">
                <SofaSkeleton lines={6} />
            </div>
        );
    }

    const semDepartamentos = !carregandoDepartamentos && !erroDepartamentos && departamentos.length === 0;
    const motivosCheios = rascunho.motivos.length >= LIMITE_MOTIVOS;

    return (
        <div className="poph-pagina">
            <CabecalhoDoAgente
                org={orgId}
                nome={agente.name}
                slug={agente.slug}
                situacao={agente.status}
                organizacao={nomeDaOrg}
                descricao="para onde a conversa vai e quando passa a humano"
            />

            <AbasDoAgente org={orgId} agente={agentId} ativa="routing" nome={agente.name} />

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

            {erroDepartamentos && (
                <SofaBanner tone="danger" title="Não consegui carregar os departamentos">
                    {erroDepartamentos}.{" "}
                    <SofaButton variant="subtle" size="sm" onClick={() => void carregarDepartamentos()}>
                        Tentar de novo
                    </SofaButton>
                </SofaBanner>
            )}

            <SecaoDoAgente
                titulo="Departamentos do agente"
                descricao="Os departamentos são da organização (criados em Estrutura). Aqui você escolhe quais o agente usa."
            >
                {semDepartamentos ? (
                    <SofaEmptyState
                        title="Nenhum departamento na organização"
                        description="Crie os departamentos em Estrutura antes de escolher o departamento padrão e o de transferência."
                        action={
                            <SofaButton
                                variant="primary"
                                icon={<IconPlus aria-hidden="true" />}
                                onClick={() => guarda.tentarNavegar(`/orgs/${orgId}/structure`)}
                            >
                                Ir para Estrutura
                            </SofaButton>
                        }
                    />
                ) : carregandoDepartamentos && departamentos.length === 0 ? (
                    <SofaSkeleton lines={3} />
                ) : (
                    <form
                        id="poph-form-roteamento"
                        className="poph-pilha"
                        style={{ gap: "var(--pop-spacing-lg)" }}
                        onSubmit={(evento) => {
                            evento.preventDefault();
                            void salvar();
                        }}
                        noValidate
                    >
                        <SofaField
                            label="Departamento padrão"
                            hint="Onde a conversa deste agente começa. Só dá para trocar; a API não aceita deixar sem departamento."
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
                                        <option value="">Escolha um departamento</option>
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
                            label="Transferir para atendente humano"
                            description="Liga a transferência: quando o motor decidir que o caso é de humano, a conversa vai para o departamento de destino abaixo."
                            checked={rascunho.transferenciaHumana}
                            onChange={(evento) =>
                                alterar({ transferenciaHumana: evento.target.checked })
                            }
                        />

                        <SofaField
                            label="Departamento humano de destino"
                            hint="Para onde a conversa vai quando há transferência. Obrigatório com a transferência ligada."
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
                                        <option value="">Escolha um departamento</option>
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
            </SecaoDoAgente>

            <SecaoDoAgente
                titulo="Motivos de transferência"
                descricao={`Situações em que a conversa deve passar a humano, em linguagem simples (ex.: “cliente pede segunda via”). Até ${LIMITE_MOTIVOS} motivos, com até ${LIMITE_MOTIVO} caracteres cada. Motivos vazios não são gravados.`}
                acoes={
                    <SofaButton
                        variant="secondary"
                        size="sm"
                        icon={<IconPlus aria-hidden="true" />}
                        disabled={motivosCheios}
                        onClick={adicionarMotivo}
                    >
                        Adicionar motivo
                    </SofaButton>
                }
            >
                {rascunho.motivos.length === 0 ? (
                    <p className="poph-mono-mudo poph-roteamento__vazio">
                        Nenhum motivo cadastrado. Sem motivos, o agente só transfere nos casos de risco que o próprio motor já trata.
                    </p>
                ) : (
                    <div className="poph-roteamento__motivos">
                        {rascunho.motivos.map((motivo, indice) => (
                            <div key={indice} className="poph-roteamento__motivo">
                                <div className="poph-roteamento__motivo-campo">
                                    <SofaField
                                        label={`Motivo ${indice + 1}`}
                                        hint={`${motivo.trim().length}/${LIMITE_MOTIVO}`}
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
                                    label={`Remover motivo ${indice + 1}`}
                                    onClick={() => removerMotivo(indice)}
                                >
                                    <IconTrash aria-hidden="true" />
                                </SofaIconButton>
                            </div>
                        ))}
                    </div>
                )}
                {erros["motivos"] && (
                    <p role="alert" className="poph-roteamento__erro">
                        {erros["motivos"]}
                    </p>
                )}
            </SecaoDoAgente>

            <div className="poph-linha-titulo poph-roteamento__barra">
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
                    Salvar roteamento
                </SofaButton>
            </div>
        </div>
    );
}

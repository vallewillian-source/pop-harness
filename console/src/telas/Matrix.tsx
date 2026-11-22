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
import { ErroApi } from "../api/cliente";
import { listarEstrutura, type Departamento } from "../api/estrutura";
import {
    alterarModuloMatrix,
    desligarEspacoMatrix,
    lerEstadoMatrix,
    ligarEspacoMatrix,
    listarEspacosMatrix,
    versaoDoEstado,
    type EstadoMatrix,
    type Espaco,
} from "../api/matrix";
import { formatarTempoNoAr, mensagemDeFalha } from "./Operacao";
import "../estilos/matrix.css";

/** Mensagem da tela quando o `If-Match` não bate: o estado mudou entre a leitura e a gravação. */
export const MENSAGEM_DE_CONFLITO = "O estado mudou entre a leitura e a gravação, tente de novo.";

type Carga<T> = { carregando: boolean; dados: T | null; erro: string | null };
type Aviso = { tom: "ok" | "erro"; texto: string } | null;

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
    if (conectado === true) return { valor: "Conectado" };
    if (conectado === false) return { valor: "Desconectado", detalhe: "o módulo tenta de novo sozinho" };
    return { valor: "Desconhecido", detalhe: "não configurado nesta instância" };
}

/** Atraso do cursor: quantas mensagens faltam espelhar, em abreviação de painel (`0 msgs atrás`). */
export function valorDoAtraso(mensagens: number): string {
    if (mensagens <= 0) return "0 msgs atrás";
    return `${mensagens.toLocaleString("pt-BR")} ${mensagens === 1 ? "msg atrás" : "msgs atrás"}`;
}

/** Falhas isoladas (mensagens que o módulo não conseguiu espelhar). */
export function valorDasFalhas(quantidade: number): string {
    return `${quantidade.toLocaleString("pt-BR")} ${quantidade === 1 ? "pendente" : "pendentes"}`;
}

/** Origem do espaço, com os rótulos do desenho: criado pelo módulo ou vinculado ao Element. */
export function textoDaOrigem(origem: string): string {
    if (origem === "auto") return "Automático (módulo)";
    if (origem === "linked") return "Vinculado (Element)";
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
            nome: espaco.department_name ?? "departamento sem nome",
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
    if (erro instanceof ErroApi && (erro.status === 400 || erro.status === 409) && erro.message) {
        return erro.message;
    }
    return mensagemDeFalha(erro, oQue);
}

function dataDoCorte(ms: number): string {
    return new Date(ms).toLocaleString("pt-BR", { dateStyle: "short", timeStyle: "short" });
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

    const [modulo, setModulo] = useState<Carga<EstadoMatrix>>(cargaInicial);
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
    const [erroDoDialogo, setErroDoDialogo] = useState<string | null>(null);

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
            lerEstadoMatrix(orgId),
            listarEspacosMatrix(orgId),
            listarEstrutura("departamento", orgId),
        ]);
        emCurso.current = false;
        if (!vivo.current) return;

        if (resModulo.status === "fulfilled") {
            setModulo({ carregando: false, dados: resModulo.value, erro: null });
        } else {
            setModulo((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: mensagemDeFalha(resModulo.reason, "o módulo Matrix"),
            }));
        }

        if (resEspacos.status === "fulfilled") {
            setEspacos({ carregando: false, dados: resEspacos.value, erro: null });
        } else {
            setEspacos((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: mensagemDeFalha(resEspacos.reason, "os espaços"),
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
                erro: mensagemDeFalha(resDepartamentos.reason, "os departamentos"),
            }));
        }
    }, [orgId]);

    useEffect(() => {
        void carregar();
    }, [carregar]);

    /** Relê só a lista de espaços (depois de criar ou desligar), sem trocar a tela para carregando. */
    const recarregarEspacos = useCallback(async () => {
        try {
            const lista = await listarEspacosMatrix(orgId);
            if (vivo.current) setEspacos({ carregando: false, dados: lista, erro: null });
        } catch (erro) {
            if (vivo.current) {
                setEspacos((atual) => ({
                    ...atual,
                    erro: mensagemDeFalha(erro, "os espaços"),
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
            const novo = await alterarModuloMatrix(orgId, ligado, versaoDoEstado(atual));
            if (!vivo.current) return;
            setModulo({ carregando: false, dados: novo, erro: null });
            setAvisoModulo({
                tom: "ok",
                texto: ligado
                    ? "Módulo ligado. Só o que chegar daqui em diante é espelhado."
                    : "Módulo desligado. Nada foi apagado: as salas e os espaços continuam no Element.",
            });
        } catch (erro) {
            if (!vivo.current) return;
            if (erro instanceof ErroApi && erro.status === 412) {
                setAvisoModulo({ tom: "erro", texto: MENSAGEM_DE_CONFLITO });
                void carregar();
            } else {
                setAvisoModulo({ tom: "erro", texto: mensagemDeFalha(erro, "o módulo Matrix") });
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
            await ligarEspacoMatrix(orgId, { department_id: linha.id, space: null });
            if (!vivo.current) return;
            setAvisoEspaco({ tom: "ok", texto: "Espaço do departamento criado." });
            await recarregarEspacos();
        } catch (erro) {
            if (vivo.current) setAvisoEspaco({ tom: "erro", texto: mensagemDoEspaco(erro, "o departamento") });
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
            await ligarEspacoMatrix(orgId, { department_id: ligandoA.id, space: espaco });
            if (!vivo.current) return;
            setLigandoA(null);
            setEspacoDigitado("");
            setAvisoEspaco({ tom: "ok", texto: "Espaço existente ligado ao departamento." });
            await recarregarEspacos();
        } catch (erro) {
            if (vivo.current) setErroDoDialogo(mensagemDoEspaco(erro, "o departamento"));
        } finally {
            if (vivo.current) setEnviandoLigacao(false);
        }
    }

    async function desligarEspaco(espaco: Espaco) {
        if (desligandoEspaco) return;
        setDesligandoEspaco(true);
        setAvisoEspaco(null);
        try {
            await desligarEspacoMatrix(orgId, espaco.department_id);
            if (!vivo.current) return;
            setEspacoParaDesligar(null);
            setAvisoEspaco({
                tom: "ok",
                texto: "Espaço desligado. Nada foi apagado no Element.",
            });
            await recarregarEspacos();
        } catch (erro) {
            if (!vivo.current) return;
            setEspacoParaDesligar(null);
            setAvisoEspaco({ tom: "erro", texto: mensagemDoEspaco(erro, "o espaço") });
        } finally {
            if (vivo.current) setDesligandoEspaco(false);
        }
    }

    const m = modulo.dados;
    const lista = espacos.dados;
    const deptos = departamentos.dados;
    const linhas = lista ? montarLinhas(deptos, lista) : [];
    const carregandoAlgum = modulo.carregando || espacos.carregando || departamentos.carregando;
    const nomeDoEspaco = (espaco: Espaco) => espaco.department_name ?? "departamento sem nome";
    const conexao = situacaoDaConexao(m?.connected);
    const espelhando = m?.enabled === true;

    return (
        <div className="poph-pagina poph-matrix">
            <div className="poph-pagina__topo">
                <div>
                    <h1 className="poph-pagina__titulo">Matrix (Element)</h1>
                    <p className="poph-pagina__descricao">
                        Como o espelho das conversas no Element está nesta organização: se o módulo
                        está ligado, quanto falta espelhar e os espaços de cada departamento.
                    </p>
                </div>
                <div className="poph-pagina__acoes">
                    <SofaButton
                        variant="subtle"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void carregar()}
                        disabled={carregandoAlgum}
                    >
                        {carregandoAlgum ? "Atualizando…" : "Atualizar"}
                    </SofaButton>
                    {enviandoModulo ? (
                        <span className="poph-matrix__status" role="status">Gravando…</span>
                    ) : null}
                    {m ? (
                        espelhando ? (
                            <SofaButton
                                variant="danger"
                                disabled={enviandoModulo}
                                onClick={() => setConfirmandoDesligarModulo(true)}
                            >
                                Desligar módulo
                            </SofaButton>
                        ) : (
                            <SofaButton
                                variant="primary"
                                loading={enviandoModulo}
                                disabled={enviandoModulo}
                                onClick={() => void alternarModulo(true)}
                            >
                                Ligar módulo Matrix
                            </SofaButton>
                        )
                    ) : null}
                </div>
            </div>

            <section className="poph-secao poph-matrix__resumo" aria-labelledby={idModulo}>
                <h2 id={idModulo} className="poph-matrix__oculto">Módulo nesta organização</h2>
                {modulo.carregando && !m ? (
                    <dl className="poph-matrix__indicadores" aria-busy="true">
                        {["Módulo na org", "Servidor de chat (Synapse)", "Atraso do cursor", "Mensagens isoladas", "Última falha"].map(
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
                        title="Não consegui ler o estado do módulo"
                        description={modulo.erro}
                        retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                    />
                ) : m ? (
                    <>
                        {modulo.erro ? (
                            <p className="poph-matrix__aviso" role="alert">
                                A última atualização falhou: {modulo.erro} Mostrando o último resultado.
                            </p>
                        ) : null}

                        {!m.enabled ? (
                            <SofaBanner tone="warn" title="Módulo Matrix desativado para esta organização">
                                O espelhamento de conversas no Element está suspenso. Ao ligar o módulo, só o que
                                chegar daí em diante é espelhado: o histórico anterior não é trazido (corte sem
                                backfill).
                            </SofaBanner>
                        ) : null}

                        {m.enabled && m.connected === false ? (
                            <SofaBanner
                                tone="danger"
                                title="Falha de comunicação com o Synapse: o envio para o Element está acumulando fila"
                                action={
                                    <SofaButton variant="subtle" onClick={() => void carregar()}>
                                        Tentar reconectar
                                    </SofaButton>
                                }
                            >
                                O WhatsApp e o motor de IA continuam operando normalmente, sem perda de mensagens. O
                                espelhamento será retomado automaticamente assim que o Synapse responder.
                            </SofaBanner>
                        ) : null}

                        {avisoModulo ? (
                            <SofaBanner tone={avisoModulo.tom === "ok" ? "success" : "danger"}>
                                {avisoModulo.texto}
                            </SofaBanner>
                        ) : null}

                        <dl className="poph-matrix__indicadores">
                            <Indicador
                                rotulo="Módulo na org"
                                valor={m.enabled ? "Ligado" : "Desligado"}
                                tom={m.enabled ? "ok" : undefined}
                                detalhe={
                                    m.enabled
                                        ? m.enabled_at
                                            ? `ativo · desde ${dataDoCorte(m.enabled_at)}`
                                            : "ativo"
                                        : m.enabled_at
                                          ? `pausado · desde ${dataDoCorte(m.enabled_at)}`
                                          : "pausado · nunca ligado"
                                }
                            />
                            <Indicador
                                rotulo="Servidor de chat (Synapse)"
                                valor={conexao.valor}
                                tom={m.connected === true ? "ok" : m.connected === false ? "erro" : undefined}
                                detalhe={conexao.detalhe}
                            />
                            <Indicador
                                rotulo="Atraso do cursor"
                                valor={espelhando ? valorDoAtraso(m.backlog.messages) : "—"}
                                detalhe={
                                    !espelhando
                                        ? "pausado"
                                        : m.backlog.messages > 0
                                          ? `a mais antiga há ${formatarTempoNoAr(m.backlog.oldest_seconds)}`
                                          : undefined
                                }
                                tom={espelhando && m.backlog.messages > 0 ? "alerta" : undefined}
                            />
                            <Indicador
                                rotulo="Mensagens isoladas"
                                valor={valorDasFalhas(m.pending_failures)}
                                tom={m.pending_failures > 0 ? "erro" : undefined}
                            />
                            <Indicador
                                rotulo="Última falha"
                                valor={m.last_failure ?? "Nenhuma falha recente"}
                                tom={m.last_failure ? "erro" : undefined}
                            />
                        </dl>
                    </>
                ) : null}
            </section>

            <section className="poph-secao" aria-labelledby={idEspacos}>
                <h2 id={idEspacos} className="poph-secao__titulo">Espaços por departamento</h2>
                <p className="poph-matrix__ajuda">
                    Salas de atendimento são organizadas no espaço Matrix do respectivo departamento da
                    conversa. Para usar um espaço que já existe no Element, use “Ligar existente” e informe o
                    id (começa com <code>!</code>) ou o endereço (começa com <code>#</code>).
                </p>

                {espacos.carregando && !lista ? (
                    <div className="poph-tabela poph-matrix__tabela" aria-busy="true" aria-label="Carregando espaços por departamento">
                        <div className="poph-tabela__cabecalho" role="presentation">
                            <span>Departamento</span>
                            <span>Espaço Matrix (alias / room ID)</span>
                            <span>Origem</span>
                            <span>Status</span>
                            <span className="poph-tabela__celula--acoes">Ações</span>
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
                        title="Não consegui ler os espaços"
                        description={espacos.erro}
                        retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                    />
                ) : lista ? (
                    <>
                        {espacos.erro ? (
                            <p className="poph-matrix__aviso" role="alert">
                                A última atualização falhou: {espacos.erro} Mostrando o último resultado.
                            </p>
                        ) : null}

                        {departamentos.erro && !deptos ? (
                            <p className="poph-matrix__aviso" role="alert">
                                Não consegui ler os departamentos: {departamentos.erro}
                            </p>
                        ) : null}

                        {avisoEspaco ? (
                            <SofaBanner tone={avisoEspaco.tom === "ok" ? "success" : "danger"}>
                                {avisoEspaco.texto}
                            </SofaBanner>
                        ) : null}

                        {departamentos.carregando && !deptos ? (
                            <SofaSkeleton lines={3} />
                        ) : linhas.length === 0 ? (
                            <SofaEmptyState
                                title="Nenhum departamento ativo"
                                description="Crie departamentos em Estrutura para que cada um tenha o seu espaço no Element."
                            />
                        ) : (
                            <div className="poph-tabela poph-matrix__tabela" role="table" aria-label="Espaços por departamento">
                                <div className="poph-tabela__cabecalho" role="row">
                                    <span role="columnheader">Departamento</span>
                                    <span role="columnheader">Espaço Matrix (alias / room ID)</span>
                                    <span role="columnheader">Origem</span>
                                    <span role="columnheader">Status</span>
                                    <span role="columnheader" className="poph-tabela__celula--acoes">Ações</span>
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
                                                        <span className="poph-matrix__vazio">Nenhum espaço vinculado</span>
                                                        <span className="poph-matrix__pequeno">
                                                            Salas criadas sem agrupamento de espaço
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
                                                        Não configurado
                                                    </span>
                                                )}
                                            </span>
                                            <span role="cell" className="poph-tabela__celula">
                                                {espaco && !linha.arquivado ? (
                                                    <span className="poph-status poph-status--ativo">
                                                        <span className="poph-status__ponto" aria-hidden="true" />
                                                        Ativo
                                                    </span>
                                                ) : espaco ? (
                                                    <span className="poph-status poph-status--inativo">
                                                        <span className="poph-status__ponto" aria-hidden="true" />
                                                        Departamento arquivado
                                                    </span>
                                                ) : (
                                                    <span className="poph-status poph-status--inativo">
                                                        <span className="poph-status__ponto" aria-hidden="true" />
                                                        Sem espaço
                                                    </span>
                                                )}
                                            </span>
                                            <span role="cell" className="poph-tabela__celula poph-tabela__celula--acoes">
                                                {espaco ? (
                                                    <SofaButton
                                                        variant="subtle"
                                                        aria-label={`Desligar espaço do departamento ${linha.nome}`}
                                                        disabled={desligandoEspaco}
                                                        onClick={() => setEspacoParaDesligar(espaco)}
                                                    >
                                                        Desligar
                                                    </SofaButton>
                                                ) : (
                                                    <>
                                                        <SofaButton
                                                            variant="primary"
                                                            size="sm"
                                                            aria-label={`Criar espaço automático do departamento ${linha.nome}`}
                                                            loading={criandoPara === linha.id}
                                                            disabled={criandoPara !== null}
                                                            onClick={() => void criarEspaco(linha)}
                                                        >
                                                            Criar auto
                                                        </SofaButton>
                                                        <SofaButton
                                                            variant="secondary"
                                                            size="sm"
                                                            aria-label={`Ligar espaço existente ao departamento ${linha.nome}`}
                                                            disabled={criandoPara !== null}
                                                            onClick={() => abrirLigacao(linha)}
                                                        >
                                                            Ligar existente
                                                        </SofaButton>
                                                    </>
                                                )}
                                            </span>
                                        </div>
                                    );
                                })}
                            </div>
                        )}

                        <SofaBanner tone="info" title="Regra de operação">
                            A ação de desligar um espaço remove apenas o vínculo do departamento no Harness. A sala e
                            o histórico permanecem intactos no Synapse.
                        </SofaBanner>
                    </>
                ) : null}
            </section>

            <SofaModal
                open={ligandoA !== null}
                onOpenChange={(aberto) => {
                    if (!aberto) fecharLigacao();
                }}
                title="Ligar departamento a espaço existente"
                description="Vincule um espaço já existente no Element para organizar as salas deste departamento."
                closeLabel="Fechar ligação de espaço existente"
                footer={
                    <>
                        <SofaButton variant="secondary" disabled={enviandoLigacao} onClick={fecharLigacao}>
                            Cancelar
                        </SofaButton>
                        <SofaButton
                            type="submit"
                            form={idFormLigacao}
                            variant="primary"
                            loading={enviandoLigacao}
                            disabled={enviandoLigacao || espacoDigitado.trim() === ""}
                        >
                            Ligar espaço
                        </SofaButton>
                    </>
                }
            >
                <form id={idFormLigacao} className="poph-matrix__form" onSubmit={(evento) => void ligarExistente(evento)} noValidate>
                    <div className="poph-matrix__departamento-escolhido">
                        <span className="poph-matrix__pequeno">Departamento selecionado</span>
                        <strong>{ligandoA ? `${ligandoA.nome} (${ligandoA.id})` : ""}</strong>
                    </div>
                    {erroDoDialogo ? (
                        <SofaBanner tone="danger">{erroDoDialogo}</SofaBanner>
                    ) : null}
                    <SofaField
                        label="Alias ou room ID do espaço Matrix"
                        hint="O bot virtual do Harness deve ter permissão de convite e gestão neste espaço."
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
                                placeholder="#alias:servidor ou !id:servidor"
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
                title="Desligar o módulo Matrix nesta organização?"
                description="O espelho para: o que chegar depois não aparece no Element até você religar. Nada é apagado: salas, espaços e conversas continuam como estão. Ao religar, só o que chegar daí em diante é espelhado."
                confirmLabel="Desligar módulo"
                cancelLabel="Cancelar"
                closeLabel="Fechar confirmação de desligar o módulo"
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
                        ? `Desligar o espaço de ${nomeDoEspaco(espacoParaDesligar)}?`
                        : "Desligar espaço?"
                }
                description="Nada é apagado no Element: o espaço e as salas continuam como estão, e nenhuma conversa é apagada. O módulo só deixa de usar esse espaço; na próxima alteração de conversa desse departamento ele volta a usá-lo."
                confirmLabel="Desligar espaço"
                cancelLabel="Cancelar"
                closeLabel="Fechar confirmação de desligar o espaço"
                tone="danger"
                loading={desligandoEspaco}
                onConfirm={() => {
                    if (espacoParaDesligar) void desligarEspaco(espacoParaDesligar);
                }}
            />
        </div>
    );
}

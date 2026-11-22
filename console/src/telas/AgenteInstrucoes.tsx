/**
 * Aba Instruções do agente (UX_SPEC §3.5, HR-40).
 *
 * Duas colunas: o formulário estruturado à esquerda e a prévia do markdown à direita. A prévia é
 * **a do servidor** — o `markdown` que o `GET …/instructions/current` devolve; a tela não remonta o
 * texto. Enquanto houver edição pendente, a prévia é a da última versão salva, e a tela avisa.
 *
 * Gravação com concorrência otimista: `POST …/instructions` quando o agente ainda não tem
 * instruções, `PATCH …/instructions/current` com `If-Match: <versão>` quando tem. Um `412` relê as
 * instruções, adota a versão atual e **mantém o que o operador digitou**, com o aviso de conflito.
 *
 * Sair com o formulário sujo passa pelo aviso de alterações não salvas (`GuardaAlteracoes`).
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconRefresh,
    SofaBanner,
    SofaButton,
    SofaErrorState,
    SofaSkeleton,
} from "@sofacoding/sofa-ui";
import { ErroApi } from "../api/cliente";
import { obterAgente, type Agente } from "../api/agentes";
import { obterOrg } from "../api/orgs";
import {
    estruturadasPadrao,
    lerInstrucoes,
    mesmasInstrucoes,
    salvarInstrucoes,
    type Estruturadas,
} from "../api/instrucoes";
import {
    AbasDoAgente,
    CabecalhoDoAgente,
    SecaoDoAgente,
} from "../componentes/AbasDoAgente";
import {
    FormularioInstrucoes,
    type ErrosDoFormulario,
} from "../componentes/FormularioInstrucoes";
import { useGuardaDeAlteracoes } from "../componentes/GuardaAlteracoes";
import "../estilos/instrucoes.css";

/** Validação local: o que o formulário exige antes de chamar a API. */
export function validarInstrucoes(valor: Estruturadas): ErrosDoFormulario {
    const erros: ErrosDoFormulario = {};
    if (!valor.identity.name.trim()) {
        erros.nome = "informe o nome do assistente";
    }
    if (valor.principles.some((principio) => !principio.rule.trim())) {
        erros.principios =
            "preencha a regra de todos os princípios ou remova os que ficaram vazios";
    }
    return erros;
}

/**
 * Traduz o erro da API: `412` é conflito de versão (relê e mantém o rascunho), `400` com campo
 * `structured` é validação de ferramenta (aparece na seção de ferramentas) e o resto vai para o
 * aviso geral.
 */
function campoDoErro(falha: ErroApi): "ferramentas" | "nome" | null {
    const detalhes = falha.detalhes as Record<string, unknown> | null | undefined;
    const campo = typeof detalhes?.["field"] === "string" ? detalhes["field"] : "";
    if (campo.startsWith("tools")) return "ferramentas";
    if (campo.startsWith("identity")) return "nome";
    if (campo === "structured") {
        const motivo = typeof detalhes?.["reason"] === "string" ? detalhes["reason"] : "";
        if (motivo.includes("ferramenta")) return "ferramentas";
    }
    return null;
}

function motivoDoErro(falha: ErroApi): string {
    const detalhes = falha.detalhes as Record<string, unknown> | null | undefined;
    return typeof detalhes?.["reason"] === "string" ? detalhes["reason"] : falha.message;
}

export function AgenteInstrucoes() {
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardaDeAlteracoes();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [carregando, setCarregando] = useState(true);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [rascunho, setRascunho] = useState<Estruturadas | null>(null);
    const [salvo, setSalvo] = useState<Estruturadas | null>(null);
    const [versao, setVersao] = useState<number | null>(null);
    const [markdown, setMarkdown] = useState("");
    const [versaoDaPrevia, setVersaoDaPrevia] = useState<number | null>(null);

    const [salvando, setSalvando] = useState(false);
    const [recarregandoPrevia, setRecarregandoPrevia] = useState(false);
    const [conflito, setConflito] = useState<string | null>(null);
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [erros, setErros] = useState<ErrosDoFormulario>({});
    const [avisoPrevia, setAvisoPrevia] = useState<string | null>(null);

    const sujo = useMemo(
        () => (rascunho && salvo ? !mesmasInstrucoes(rascunho, salvo) : false),
        [rascunho, salvo],
    );

    const carregar = useCallback(async () => {
        setCarregando(true);
        setErroDeCarga(null);
        setConflito(null);
        setErroGeral(null);
        setErros({});
        setAvisoPrevia(null);
        try {
            const [dadosDoAgente, instrucoes] = await Promise.all([
                obterAgente(orgId, agentId),
                lerInstrucoes(orgId, agentId),
            ]);
            setAgente(dadosDoAgente);
            const base = instrucoes ? instrucoes.structured : estruturadasPadrao();
            // Agente sem instruções: o nome dele já entra como nome do assistente.
            if (!instrucoes && !base.identity.name) {
                base.identity.name = dadosDoAgente.name;
            }
            setRascunho(base);
            setSalvo(base);
            setVersao(instrucoes?.version ?? null);
            setVersaoDaPrevia(instrucoes?.version ?? null);
            setMarkdown(instrucoes?.markdown ?? "");
        } catch (falha) {
            const mensagem =
                falha instanceof Error ? falha.message : "falha ao carregar as instruções";
            setErroDeCarga(mensagem);
        } finally {
            setCarregando(false);
        }
    }, [orgId, agentId]);

    useEffect(() => {
        void carregar();
        obterOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
    }, [carregar, orgId]);

    /** Grava o rascunho. `true` quando o servidor aceitou (a tela pode sair). */
    const salvar = useCallback(async (): Promise<boolean> => {
        if (!rascunho) return false;
        const novos = validarInstrucoes(rascunho);
        setErros(novos);
        setConflito(null);
        setErroGeral(null);
        if (Object.keys(novos).length > 0) return false;

        setSalvando(true);
        try {
            const resposta = await salvarInstrucoes(orgId, agentId, rascunho, versao);
            setRascunho(resposta.structured);
            setSalvo(resposta.structured);
            setVersao(resposta.version);
            setVersaoDaPrevia(resposta.version);
            setMarkdown(resposta.markdown);
            setAvisoPrevia(null);
            return true;
        } catch (falha) {
            if (falha instanceof ErroApi && falha.status === 412) {
                // Relê para continuar de uma versão válida; o rascunho fica como está.
                let atual: Awaited<ReturnType<typeof lerInstrucoes>> = null;
                try {
                    atual = await lerInstrucoes(orgId, agentId);
                } catch {
                    // Sem a releitura, a próxima tentativa devolve 412 de novo; nada se perde.
                }
                if (atual) {
                    setVersao(atual.version);
                    setVersaoDaPrevia(atual.version);
                    setMarkdown(atual.markdown);
                }
                setConflito(
                    `Outra pessoa salvou as instruções antes de você${
                        atual ? ` (agora na versão ${atual.version})` : ""
                    }. O que você digitou foi mantido: revise e salve de novo.`,
                );
                return false;
            }
            if (falha instanceof ErroApi) {
                const campo = campoDoErro(falha);
                const motivo = motivoDoErro(falha);
                if (campo === "ferramentas") {
                    setErros({ ferramentas: motivo });
                } else if (campo === "nome") {
                    setErros({ nome: motivo });
                } else {
                    // Formato do contrato: mensagem legível + código estável, para o ticket.
                    setErroGeral(`${falha.message} (código ${falha.codigo})`);
                }
                return false;
            }
            setErroGeral("falha inesperada ao salvar as instruções");
            return false;
        } finally {
            setSalvando(false);
        }
    }, [rascunho, versao, orgId, agentId]);

    // A guarda precisa da versão mais recente de `salvar`; o provedor guarda a referência.
    useEffect(() => {
        if (!rascunho) return;
        guarda.registrar({ sujo, salvar });
    }, [guarda, sujo, salvar, rascunho]);

    useEffect(() => {
        return () => guarda.registrar(null);
    }, [guarda]);

    // Ctrl+S / Cmd+S salva quando há edição pendente (UX_SPEC §5).
    useEffect(() => {
        const aoTeclar = (evento: KeyboardEvent) => {
            if (!(evento.ctrlKey || evento.metaKey) || evento.key.toLowerCase() !== "s") return;
            if (!sujo && versao !== null) return;
            evento.preventDefault();
            void salvar();
        };
        window.addEventListener("keydown", aoTeclar);
        return () => window.removeEventListener("keydown", aoTeclar);
    }, [sujo, versao, salvar]);

    /** Relê o texto composto no servidor. Só adota a versão nova quando não há rascunho pendente. */
    async function atualizarPrevia() {
        setRecarregandoPrevia(true);
        setAvisoPrevia(null);
        try {
            const atual = await lerInstrucoes(orgId, agentId);
            if (!atual) {
                setMarkdown("");
                setVersaoDaPrevia(null);
                return;
            }
            setMarkdown(atual.markdown);
            setVersaoDaPrevia(atual.version);
            if (!sujo) {
                setRascunho(atual.structured);
                setSalvo(atual.structured);
                setVersao(atual.version);
            } else if (atual.version !== versao) {
                setAvisoPrevia(
                    `Existe uma versão mais nova salva (v${atual.version}); a prévia mostra o texto dela. Ao salvar, você recebe o aviso de conflito.`,
                );
            }
        } catch (falha) {
            setAvisoPrevia(
                falha instanceof Error
                    ? `Não consegui reler a prévia: ${falha.message}`
                    : "Não consegui reler a prévia.",
            );
        } finally {
            setRecarregandoPrevia(false);
        }
    }

    const previaDesatualizada = sujo && versaoDaPrevia !== null;

    if (carregando && !agente) {
        return (
            <div className="poph-pagina" aria-busy="true">
                <SofaSkeleton width={260} height={26} />
                <div className="poph-instrucoes__esqueleto-linha">
                    <SofaSkeleton width={360} height={12} />
                </div>
                <SofaSkeleton lines={5} />
            </div>
        );
    }

    if (erroDeCarga && !agente) {
        return (
            <div className="poph-pagina">
                <SofaErrorState
                    title="Não consegui carregar as instruções"
                    description="A instância não respondeu à leitura do agente ou das instruções. Tente de novo; se persistir, confira a saúde do servidor."
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

    if (!agente || !rascunho) return null;

    return (
        <div className="poph-pagina">
            <CabecalhoDoAgente
                org={orgId}
                nome={agente.name}
                slug={agente.slug}
                situacao={agente.status}
                organizacao={nomeDaOrg}
                descricao={
                    <>
                        {versao === null ? "sem instruções salvas" : `If-Match: v${versao}`}
                        {sujo && <span className="poph-sujo"> · alterações não salvas</span>}
                    </>
                }
                acoes={
                    <>
                        <SofaButton
                            variant="secondary"
                            disabled={!sujo || salvando}
                            onClick={() => {
                                setRascunho(salvo);
                                setErros({});
                                setConflito(null);
                                setErroGeral(null);
                            }}
                        >
                            Desfazer
                        </SofaButton>
                        <SofaButton
                            variant="primary"
                            disabled={!sujo && versao !== null}
                            loading={salvando}
                            onClick={() => void salvar()}
                        >
                            {versao === null ? "Salvar instruções" : `Salvar (v${versao})`}
                        </SofaButton>
                    </>
                }
            />

            <AbasDoAgente org={orgId} agente={agentId} ativa="instructions" nome={agente.name} />

            {(conflito || erroGeral) && (
                <div className="poph-pilha">
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
                </div>
            )}

            <div className="poph-instrucoes">
                <form
                    id="poph-form-instrucoes"
                    noValidate
                    onSubmit={(evento) => {
                        evento.preventDefault();
                        void salvar();
                    }}
                >
                    <FormularioInstrucoes
                        valor={rascunho}
                        aoMudar={setRascunho}
                        erros={erros}
                        desabilitado={salvando}
                    />
                </form>

                <aside className="poph-instrucoes__previa">
                    <SecaoDoAgente
                        titulo="Prévia do texto composto"
                        descricao="Markdown composto pelo servidor a partir das instruções salvas."
                        acoes={
                            <SofaButton
                                variant="subtle"
                                icon={<IconRefresh aria-hidden="true" />}
                                loading={recarregandoPrevia}
                                onClick={() => void atualizarPrevia()}
                            >
                                Atualizar prévia
                            </SofaButton>
                        }
                    >
                        <div className="poph-pilha">
                            {previaDesatualizada && (
                                <SofaBanner tone="info" title="Prévia do texto salvo">
                                    Você tem alterações não salvas. A prévia mostra a última versão
                                    salva
                                    {versaoDaPrevia !== null ? ` (v${versaoDaPrevia})` : ""}; ela é
                                    atualizada pelo servidor quando você salva.
                                </SofaBanner>
                            )}
                            {avisoPrevia && (
                                <SofaBanner tone="warn" title="Prévia">
                                    {avisoPrevia}
                                </SofaBanner>
                            )}
                            {markdown ? (
                                <pre
                                    className="poph-previa"
                                    aria-label="Prévia do texto composto em markdown"
                                >
                                    {markdown}
                                </pre>
                            ) : (
                                <p className="poph-mono-mudo">
                                    Nenhum texto composto ainda: o servidor compõe o markdown quando
                                    as instruções são salvas.
                                </p>
                            )}
                        </div>
                    </SecaoDoAgente>
                </aside>
            </div>
        </div>
    );
}

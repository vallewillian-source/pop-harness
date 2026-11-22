/**
 * Formulário de integração (criar e alterar), em modal (HR-44 e HR-44e).
 *
 * **O formulário vem do plugin, não do Console.** Ao escolher (ou abrir) um `kind`, a tela pede o
 * manifesto (`GET …/manifests/{kind}`) e desenha `FormularioEsquema` com a `configuracao` e os
 * `segredos` que ele declara. Qualquer plugin que fale o protocolo ganha formulário sem uma linha de
 * código por integração.
 *
 * Três desfechos do manifesto, tratados de forma diferente:
 * - `200` → desenha o formulário e envia `config_json`/`secrets` junto com o `kind`/`status`;
 * - `404` → este tipo **não declara** formulário no servidor: a tela avisa e salva só `kind`/`status`
 *   (nunca inventa configuração sem esquema — o servidor responderia `422 manifest_indisponivel`);
 * - `503` → o plugin está declarado mas não respondeu ao handshake: a tela explica e **não deixa
 *   salvar**, porque o servidor recusaria a criação do mesmo jeito.
 *
 * Configuração e segredo são edição de configuração: `PATCH` com `If-Match` da versão lida (sem ele
 * `428`, versão velha `412`). No `412` a tela relê a integração, passa a usar a versão atual e
 * **mantém o que foi digitado** — configuração e segredos desta sessão.
 *
 * Segredo nunca sai da tela nem volta para ela: o campo é `password`, começa sempre vazio, e a única
 * coisa que a API conta de um segredo gravado são os **nomes** (`segredos_preenchidos`) — ou que
 * existe algum, quando a chave de cifra mudou.
 */

import { useCallback, useEffect, useState, type FormEvent } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaField,
    SofaInput,
    SofaModal,
    SofaSelect,
    SofaSkeleton,
    SofaStatusIndicator,
    type SofaFieldControlProps,
    type SofaStatusTone,
} from "@sofacoding/sofa-ui";
import { ErroApi } from "../api/cliente";
import {
    KIND_WHATSAPP,
    LIMITE_KIND,
    SITUACOES_INTEGRACAO,
    alterarIntegracao,
    criarIntegracao,
    obterIntegracao,
    obterManifesto,
    rotuloDaConexao,
    semManifestoParaEsteTipo,
    validarIntegracao,
    type DadosDaIntegracao,
    type Integracao,
    type ManifestoDoPlugin,
    type SituacaoIntegracao,
} from "../api/integracoes";
import {
    FormularioEsquema,
    camposDoEsquema,
    validarValoresDeEsquema,
    valoresDaIntegracao,
    valoresParaConfiguracao,
    valoresParaSegredos,
    type ValoresDeEsquema,
} from "./FormularioEsquema";

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

/** Campo da API (`details.field`) → campo do formulário, para os campos que não vêm do esquema. */
const CAMPOS_DA_API: Record<string, string> = {
    kind: "kind",
    tipo: "kind",
    status: "situacao",
    situacao: "situacao",
};

/**
 * `details.field` da API → campo do formulário. Os campos do manifesto chegam prefixados
 * (`config_json.remetente`, `secrets.token`, e aninhado com ponto): o prefixo diz de qual grupo é, e
 * o primeiro segmento do resto é o campo que o `FormularioEsquema` conhece.
 */
export function campoDoErroDaApi(campo: string): string | null {
    const direto = CAMPOS_DA_API[campo];
    if (direto) return direto;
    for (const prefixo of ["config_json.", "secrets."]) {
        if (campo.startsWith(prefixo)) {
            const resto = campo.slice(prefixo.length);
            return resto.split(".")[0] || null;
        }
    }
    return null;
}

/**
 * Nomes dos segredos já gravados: a lista quando a chave de cifra abre o blob, `null` quando existe
 * segredo mas não é possível saber quais (chave trocada), e `[]` quando não há segredo nenhum.
 */
export function estadoInicialDosSegredos(integracao: Integracao | null): string[] | null {
    if (!integracao) return [];
    const preenchidos = integracao.segredos_preenchidos;
    if (Array.isArray(preenchidos)) return preenchidos;
    if (preenchidos === null) return null;
    return integracao.tem_segredo ? null : [];
}

function tomDaConexao(estado: string): SofaStatusTone {
    switch (estado) {
        case "connected":
            return "success";
        case "awaiting_scan":
            return "attention";
        case "disconnected":
            return "danger";
        default:
            return "muted";
    }
}

/** Estado da leitura do manifesto para o `kind` escolhido. */
type EstadoDoManifesto =
    | { situacao: "ocioso" }
    | { situacao: "carregando" }
    | { situacao: "pronto"; manifesto: ManifestoDoPlugin }
    /** `404`: este tipo não tem plugin declarado, logo não tem formulário. */
    | { situacao: "sem_formulario" }
    /** `503` (ou falha de transporte): não há como configurar sem quebrar o contrato. */
    | { situacao: "falha"; mensagem: string; codigo?: string; dica?: string };

const DICAS_DO_MANIFESTO: Record<string, string> = {
    manifesto_indisponivel:
        "O plugin deste tipo está declarado na configuração do servidor, mas não respondeu. Sem o formulário dele não há como gravar configuração — confira o plugin e tente de novo.",
    rede: "Não foi possível falar com a instância para ler o formulário do plugin.",
    not_found: "Este tipo não declara formulário no servidor.",
};

/**
 * Configuração a enviar: parte do que já está gravado (para não perder chave que o formulário não
 * desenha, como objeto aninhado ou `additionalProperties`), aplica os campos editáveis e remove os
 * que o operador deixou em branco.
 */
export function configuracaoParaEnviar(
    manifesto: ManifestoDoPlugin,
    valores: ValoresDeEsquema,
    gravada: Record<string, unknown> | null | undefined,
): Record<string, unknown> {
    const enviada: Record<string, unknown> = { ...(gravada ?? {}) };
    const editaveis = valoresParaConfiguracao(manifesto.configuracao, valores);
    for (const campo of camposDoEsquema(manifesto.configuracao, null)) {
        if (campo.segredo) continue;
        const desenhado =
            campo.propriedade.enum !== undefined ||
            campo.propriedade.type === "string" ||
            campo.propriedade.type === "number" ||
            campo.propriedade.type === "integer" ||
            campo.propriedade.type === "boolean";
        if (!desenhado) continue;
        if (campo.nome in editaveis) {
            enviada[campo.nome] = editaveis[campo.nome];
        } else {
            delete enviada[campo.nome];
        }
    }
    return enviada;
}

export type ModalIntegracaoProps = {
    org: string;
    agente: string;
    aberto: boolean;
    aoFechar: () => void;
    /** Integração em edição; `null` cria uma nova. */
    integracao: Integracao | null;
    /** Chamado com a integração devolvida pela API (criada ou alterada). */
    aoSalvar: (integracao: Integracao) => void;
};

export function ModalIntegracao({
    org,
    agente,
    aberto,
    aoFechar,
    integracao,
    aoSalvar,
}: ModalIntegracaoProps) {
    const editando = integracao !== null;
    const [kind, setKind] = useState("");
    const [situacao, setSituacao] = useState<SituacaoIntegracao>("active");
    const [versao, setVersao] = useState(0);
    const [erros, setErros] = useState<Record<string, string>>({});
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [conflito, setConflito] = useState<string | null>(null);
    const [enviando, setEnviando] = useState(false);

    const [manifesto, setManifesto] = useState<EstadoDoManifesto>({ situacao: "ocioso" });
    const [valores, setValores] = useState<ValoresDeEsquema>({});
    /** `kind` para o qual os `valores` atuais foram montados (evita apagar o que foi digitado). */
    const [tipoDosValores, setTipoDosValores] = useState<string | null>(null);
    const [segredosRemovidos, setSegredosRemovidos] = useState<string[]>([]);
    const [segredosPreenchidos, setSegredosPreenchidos] = useState<string[] | null>([]);

    useEffect(() => {
        if (!aberto) return;
        setKind(integracao?.kind ?? "");
        setSituacao((integracao?.status as SituacaoIntegracao | undefined) ?? "active");
        setVersao(integracao?.version ?? 0);
        setErros({});
        setErroGeral(null);
        setConflito(null);
        setEnviando(false);
        setManifesto({ situacao: "ocioso" });
        setValores({});
        setTipoDosValores(null);
        setSegredosRemovidos([]);
        setSegredosPreenchidos(estadoInicialDosSegredos(integracao));
    }, [aberto, integracao]);

    // ── manifesto do tipo escolhido ────────────────────────────────────────
    // Ao digitar o tipo, a leitura espera o operador parar (400 ms) para não subir o plugin a cada
    // tecla; na edição o tipo já está definido e a leitura é imediata.
    useEffect(() => {
        if (!aberto) return;
        const tipo = (editando ? (integracao?.kind ?? "") : kind).trim();
        if (tipo === "") {
            setManifesto({ situacao: "ocioso" });
            return;
        }
        let cancelado = false;
        const temporizador = setTimeout(
            () => {
                setManifesto({ situacao: "carregando" });
                obterManifesto(org, tipo)
                    .then((lido) => {
                        if (cancelado) return;
                        // Manifesto sem esquema de objeto não vira formulário: melhor dizer que a
                        // leitura falhou do que desenhar uma tela vazia e gravar às cegas.
                        if (
                            !lido ||
                            typeof lido.configuracao !== "object" ||
                            lido.configuracao === null
                        ) {
                            setManifesto({
                                situacao: "falha",
                                mensagem:
                                    "o servidor respondeu sem o esquema de configuração deste tipo",
                                codigo: "manifesto_invalido",
                            });
                            return;
                        }
                        setManifesto({ situacao: "pronto", manifesto: lido });
                        // Valores e formulário chegam juntos: o primeiro render já vem preenchido
                        // (`default` do esquema + o que está gravado), sem piscar vazio. O tipo
                        // mudou para chegar aqui, então os valores são montados do zero.
                        setValores(
                            valoresDaIntegracao(
                                lido.configuracao,
                                lido.segredos ?? null,
                                editando ? integracao?.configuracao : null,
                            ),
                        );
                        setTipoDosValores(tipo);
                    })
                    .catch((falha: unknown) => {
                        if (cancelado) return;
                        if (semManifestoParaEsteTipo(falha)) {
                            setManifesto({ situacao: "sem_formulario" });
                            return;
                        }
                        if (falha instanceof ErroApi) {
                            setManifesto({
                                situacao: "falha",
                                mensagem: falha.message,
                                codigo: falha.codigo,
                                dica: DICAS_DO_MANIFESTO[falha.codigo],
                            });
                            return;
                        }
                        setManifesto({
                            situacao: "falha",
                            mensagem:
                                falha instanceof Error
                                    ? falha.message
                                    : "falha ao ler o formulário do plugin",
                        });
                    });
            },
            editando ? 0 : 400,
        );
        return () => {
            cancelado = true;
            clearTimeout(temporizador);
        };
    }, [aberto, kind, editando, integracao, org]);

    // Valores iniciais quando o formulário do tipo chega: `default` do esquema + o que está gravado.
    // Só monta uma vez por tipo, para uma releitura não apagar o que o operador digitou.
    useEffect(() => {
        if (manifesto.situacao !== "pronto") return;
        const tipo = manifesto.manifesto.kind || (editando ? (integracao?.kind ?? "") : kind);
        if (tipoDosValores === tipo) return;
        setValores(
            valoresDaIntegracao(
                manifesto.manifesto.configuracao,
                manifesto.manifesto.segredos ?? null,
                editando ? integracao?.configuracao : null,
            ),
        );
        setTipoDosValores(tipo);
    }, [manifesto, tipoDosValores, editando, integracao, kind]);

    /** Põe o erro da API no campo que ele aponta; o resto vai para o aviso geral. */
    function mostrarErroDaApi(falha: ErroApi) {
        const detalhes = falha.detalhes as Record<string, unknown> | null | undefined;
        const campo = typeof detalhes?.["field"] === "string" ? detalhes["field"] : undefined;
        const motivo =
            typeof detalhes?.["reason"] === "string" ? detalhes["reason"] : falha.message;
        const alvo = campo ? campoDoErroDaApi(campo) : null;
        if (alvo) {
            setErros({ [alvo]: motivo });
            return;
        }
        setErroGeral(falha.message);
    }

    const alternarRemocao = useCallback((nome: string, remover: boolean) => {
        setSegredosRemovidos((atuais) =>
            remover ? [...new Set([...atuais, nome])] : atuais.filter((item) => item !== nome),
        );
    }, []);

    async function salvar(evento?: FormEvent) {
        evento?.preventDefault();
        const novos = validarIntegracao({ kind, situacao, editando });

        if (manifesto.situacao === "carregando") {
            setErroGeral("Aguarde: ainda estou lendo o formulário deste tipo.");
            return;
        }
        const pronto = manifesto.situacao === "pronto" ? manifesto.manifesto : null;
        if (pronto) {
            Object.assign(
                novos,
                validarValoresDeEsquema(
                    pronto.configuracao,
                    pronto.segredos ?? null,
                    valores,
                    { preenchidos: segredosPreenchidos, removidos: segredosRemovidos },
                ),
            );
        }
        setErros(novos);
        setErroGeral(null);
        setConflito(null);
        if (Object.keys(novos).length > 0) return;

        const dados: DadosDaIntegracao = { status: situacao };
        if (pronto) {
            dados.configuracao = configuracaoParaEnviar(
                pronto,
                valores,
                editando ? integracao?.configuracao : null,
            );
            const segredos = valoresParaSegredos(
                pronto.segredos ?? null,
                valores,
                segredosRemovidos,
            );
            if (Object.keys(segredos).length > 0) dados.segredos = segredos;
        }

        setEnviando(true);
        try {
            if (!integracao) {
                const criada = await criarIntegracao(org, agente, kind.trim(), dados);
                aoSalvar(criada);
                aoFechar();
                return;
            }
            const alterada = await alterarIntegracao(
                org,
                agente,
                integracao.id,
                versao,
                dados,
            );
            aoSalvar(alterada);
            aoFechar();
        } catch (falha) {
            if (falha instanceof ErroApi && falha.status === 412 && integracao) {
                let versaoAtual: number | undefined;
                try {
                    const atual = await obterIntegracao(org, agente, integracao.id);
                    versaoAtual = atual.version;
                    setSegredosPreenchidos(estadoInicialDosSegredos(atual));
                } catch {
                    // Sem a releitura a próxima tentativa devolve 412 de novo; nada se perde.
                }
                if (typeof versaoAtual === "number") setVersao(versaoAtual);
                setConflito(
                    `Outra pessoa alterou esta integração antes de você${
                        typeof versaoAtual === "number" ? ` (agora na versão ${versaoAtual})` : ""
                    }. O que você preencheu foi mantido: revise e salve de novo.`,
                );
                return;
            }
            if (falha instanceof ErroApi) {
                mostrarErroDaApi(falha);
                return;
            }
            setErroGeral("falha inesperada ao salvar a integração");
        } finally {
            setEnviando(false);
        }
    }

    const titulo = editando ? "Editar integração" : "Nova integração";
    /** Plugin declarado que não respondeu: sem formulário, não dá para gravar. */
    const bloqueadoPeloManifesto = manifesto.situacao === "falha";
    const semConfiguracao = manifesto.situacao === "sem_formulario";

    return (
        <SofaModal
            open={aberto}
            onOpenChange={(novo) => {
                if (!novo && !enviando) aoFechar();
            }}
            title={titulo}
            description={
                editando
                    ? "O tipo não muda depois de criado. O salvamento usa a versão lida; configuração e segredo entram pelo formulário do plugin."
                    : "Escolha o tipo: o formulário de configuração é o que o plugin daquele tipo declara no servidor."
            }
            closeLabel="Fechar formulário de integração"
            id="poph-formulario-integracao"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={enviando} onClick={aoFechar}>
                        Cancelar
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        onClick={() => void salvar()}
                        loading={enviando}
                        disabled={bloqueadoPeloManifesto}
                    >
                        {editando ? "Salvar alterações" : "Criar integração"}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-integracao"
                className="poph-pilha"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void salvar(evento)}
                noValidate
            >
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

                <SofaField
                    label="Tipo da integração"
                    hint={`Identifica o canal no servidor. Exemplos: ${KIND_WHATSAPP}, eco. Até ${LIMITE_KIND} caracteres; não muda depois de criado.`}
                    error={erros["kind"]}
                    required
                    htmlFor="poph-integracao-kind"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={kind}
                            autoComplete="off"
                            disabled={editando}
                            onChange={(evento) => setKind(evento.target.value)}
                        />
                    )}
                </SofaField>

                {editando && integracao && (
                    <div className="poph-pilha" style={{ gap: 4 }}>
                        <span className="poph-mono-mudo">Estado de conexão (informado pelo canal)</span>
                        <div>
                            <SofaStatusIndicator tone={tomDaConexao(integracao.connection_state)}>
                                {rotuloDaConexao(integracao.connection_state)}
                            </SofaStatusIndicator>
                        </div>
                    </div>
                )}

                {manifesto.situacao === "carregando" && (
                    <div className="poph-pilha" style={{ gap: 4 }} role="status">
                        <span className="poph-mono-mudo">Lendo o formulário deste tipo…</span>
                        <SofaSkeleton lines={3} />
                    </div>
                )}

                {manifesto.situacao === "pronto" && (
                    <div className="poph-pilha" style={{ gap: "var(--pop-spacing-sm, 8px)" }}>
                        <span className="poph-mono-mudo">
                            Configuração do plugin “{manifesto.manifesto.nome}” (versão{" "}
                            {manifesto.manifesto.versao})
                        </span>
                        <FormularioEsquema
                            configuracao={manifesto.manifesto.configuracao}
                            segredos={manifesto.manifesto.segredos ?? null}
                            valores={valores}
                            aoMudar={(nome, valor) =>
                                setValores((atuais) => ({ ...atuais, [nome]: valor }))
                            }
                            erros={erros}
                            idPrefixo="poph-integracao"
                            estadoDosSegredos={{
                                preenchidos: segredosPreenchidos,
                                removidos: segredosRemovidos,
                            }}
                            aoAlternarRemocao={alternarRemocao}
                        />
                    </div>
                )}

                {semConfiguracao && (
                    <SofaBanner tone="info" title="Este tipo não tem formulário">
                        O servidor não declara manifesto para “{kind.trim()}”: a integração é criada
                        só com o tipo e a situação, e nenhuma configuração é gravada.
                    </SofaBanner>
                )}

                {manifesto.situacao === "falha" && (
                    <SofaBanner tone="danger" title="Não consegui ler o formulário do plugin">
                        {manifesto.mensagem}
                        {manifesto.dica ? ` ${manifesto.dica}` : ""}
                        {manifesto.codigo ? (
                            <span className="poph-mono-mudo"> (código: {manifesto.codigo})</span>
                        ) : null}
                    </SofaBanner>
                )}

                <SofaField
                    label="Situação"
                    hint="Inativa e arquivada param o atendimento por este canal; arquivada é o estado final."
                    error={erros["situacao"]}
                    required
                    htmlFor="poph-integracao-situacao"
                >
                    {(controle) => (
                        <SofaSelect
                            {...semDescribedBy(controle)}
                            value={situacao}
                            onChange={(evento) =>
                                setSituacao(evento.target.value as SituacaoIntegracao)
                            }
                        >
                            {SITUACOES_INTEGRACAO.map((opcao) => (
                                <option key={opcao.valor} value={opcao.valor}>
                                    {opcao.rotulo}
                                </option>
                            ))}
                        </SofaSelect>
                    )}
                </SofaField>

                {segredosPreenchidos === null && integracao?.tem_segredo !== false && (
                    <p className="poph-mono-mudo" style={{ margin: 0 }}>
                        Há segredo gravado nesta integração, mas a chave de criptografia atual do
                        servidor não consegue lê-lo: a tela não sabe quais campos estão preenchidos.
                        Digitar o valor de novo resolve.
                    </p>
                )}

                {manifesto.situacao === "pronto" && (
                    <p className="poph-mono-mudo" style={{ margin: 0 }}>
                        Segredo em branco mantém o que já está gravado; digitar troca o valor. O
                        valor de um segredo nunca volta para esta tela depois de salvo.
                    </p>
                )}
            </form>
        </SofaModal>
    );
}

/**
 * Aba Chat de teste do agente (UX_SPEC §3.6, HR-46): conversas de teste com memória zerada, o
 * histórico, o envio de texto e arquivos e o turno que chega em streaming.
 *
 * O que a tela faz, pelo contrato que existe (`api/testeChat.ts`, `docs/API.md`):
 * - lista as conversas do agente e cria uma nova (memória zerada);
 * - envia texto e arquivos (imagem, PDF e Office) e mostra o texto do agente conforme chega;
 * - mostra as ferramentas usadas, a transferência para uma pessoa e o fim do turno com tokens e
 *   tempo; em falha, o motivo;
 * - quando a conversa está transferida, bloqueia o envio e oferece "Reabrir atendimento da IA"
 *   (com `If-Match`; no `412` relê a conversa e mantém a tela);
 * - cancela o turno em andamento abortando a conexão: a queda da conexão é o cancelamento no servidor;
 * - trocar de conversa no meio de um turno aborta o turno daquela conversa e não escreve nada dele
 *   na conversa nova.
 *
 * Arquivos do cliente no histórico (HR-46c): imagem abre numa prévia ("Ver"); imagem, PDF e Office
 * podem ser baixados ("Baixar"). Arquivos que o agente enviou (HR-46e) aparecem no turno dele, com o
 * estado do envio, e seguem os mesmos botões. O conteúdo sai pelo cliente HTTP com o token, não por
 * `<a href>`.
 *
 * O painel do porquê (HR-47) abre pelo botão "Ver por quê" de cada resposta do agente e pelo turno
 * sem resposta no registro.
 */

import {
    useCallback,
    useEffect,
    useRef,
    useState,
    type ChangeEvent,
    type FormEvent,
    type KeyboardEvent,
} from "react";
import { useNavigate, useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconMessage,
    IconPlus,
    IconRefresh,
    IconSend,
    IconUpload,
    IconXCircle,
    SofaBanner,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaSkeleton,
} from "@sofacoding/sofa-ui";
import { getAgent, type Agente } from "../api/agents";
import { ApiError } from "../api/client";
import { getOrg } from "../api/orgs";
import {
    LIMIT_FILE_BYTES,
    createTestConversation,
    sendTestMessage,
    listTestConversations,
    getTestHistory,
    reopenTestConversation,
    type TestAttachment,
    type TestSentFile,
    type TestConversation,
    type TurnEvent,
    type TestHistory,
    type TestMessage,
    type TestTurn,
} from "../api/testChat";
import {
    downloadContent,
    readAsDataUrl,
    clientAttachmentOrigin,
    agentFileOrigin,
    canSeeOnScreen,
    sendLabel,
    type FileOrigin,
} from "../api/testFile";
import { interleaveTurns, phraseForStateWithoutResponse, turnStateLabel } from "../api/turnWhy";
import { CabecalhoDoAgente, AbasDoAgente } from "../componentes/AbasDoAgente";
import { PainelPorQue } from "../componentes/PainelPorQue";
import { PreviaArquivoTeste, type PreviaDeImagem } from "../componentes/PreviaArquivoTeste";
import { useI18n, translate } from "../i18n";
import { formatDateTime, formatNumber } from "../i18n/format";
import "../estilos/chat-teste.css";

/** Tipos de arquivo que o recebimento aceita (imagem, PDF e Office). A API confere o conteúdo. */
const ACEITOS = ".jpg,.jpeg,.png,.gif,.webp,.pdf,.doc,.docx,.xls,.xlsx,.ppt,.pptx";

type Aviso = {
    tom: "info" | "warn" | "danger";
    titulo: string;
    texto: string;
} | null;

/** O turno que está em voo: o que o cliente mandou e o que o agente já escreveu. */
type TurnoEmCurso = {
    textoDoCliente: string;
    anexosDoCliente: string[];
    resposta: string;
    ferramentas: string[];
    fontes: number | null;
    transferido: boolean;
    erro: string | null;
};

/** Números do fim do turno (`fim`): tokens e tempo, mostrados depois que a resposta fecha. */
type Encerramento = {
    estado: string;
    tokensEntrada: number | null;
    tokensSaida: number | null;
    totalMs: number | null;
} | null;

type ErroDeCampo = { campo: "text" | "file"; mensagem: string } | null;

function mensagemDe(falha: unknown): string {
    return falha instanceof Error ? falha.message : translate("testChat.fallbackUnknown");
}

function ehAbortado(falha: unknown): boolean {
    return (
        (falha instanceof DOMException || falha instanceof Error) && falha.name === "AbortError"
    );
}

function dataCurta(ms: number): string {
    return formatDateTime(ms);
}

/** Identificador curto para a lista e o cabeçalho (o id completo fica no atributo). */
function idCurto(id: string): string {
    return id.length > 8 ? `…${id.slice(-8)}` : id;
}

function tamanhoLegivel(bytes: number): string {
    if (bytes >= 1024 * 1024) {
        return `${formatNumber(bytes / (1024 * 1024), { minimumFractionDigits: 1, maximumFractionDigits: 1 })} MB`;
    }
    return `${formatNumber(Math.max(1, Math.round(bytes / 1024)))} KB`;
}

function ehTransferida(conversa: TestConversation | undefined | null): boolean {
    return conversa?.status === "transferred";
}

function rotuloDoStatus(status: string): string {
    return status === "transferred"
        ? translate("testChat.statusTransferred")
        : status === "open"
          ? translate("testChat.statusActive")
          : status;
}

function classeDoStatus(status: string): string {
    return status === "transferred" ? "poph-status--atencao" : "poph-status--ativo";
}

/** Texto de um campo do `fim`/`erro`, quando vier como texto. */
function texto(valor: unknown): string | null {
    return typeof valor === "string" && valor ? valor : null;
}

function numero(valor: unknown): number | null {
    return typeof valor === "number" ? valor : null;
}

/** Os eventos do turno chegam com o JSON já lido; `dados` vazio vira objeto vazio. */
function campos(evento: TurnEvent): Record<string, unknown> {
    return evento.dados && typeof evento.dados === "object"
        ? (evento.dados as Record<string, unknown>)
        : {};
}

export function AgenteChatTeste() {
    const { orgId = "", agentId = "" } = useParams();
    const navegar = useNavigate();
    const { translate: t } = useI18n();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [conversas, setConversas] = useState<TestConversation[]>([]);
    const [proximoCursor, setProximoCursor] = useState<string | null>(null);
    const [carregandoLista, setCarregandoLista] = useState(true);
    const [erroLista, setErroLista] = useState<string | null>(null);
    const [criando, setCriando] = useState(false);

    const [ativa, setAtiva] = useState<string | null>(null);
    const [historico, setHistorico] = useState<TestHistory | null>(null);
    const [carregandoHistorico, setCarregandoHistorico] = useState(false);
    const [erroHistorico, setErroHistorico] = useState<string | null>(null);

    const [rascunho, setRascunho] = useState("");
    const [arquivos, setArquivos] = useState<File[]>([]);
    const [erroDeCampo, setErroDeCampo] = useState<ErroDeCampo>(null);
    const [enviando, setEnviando] = useState(false);
    const [turno, setTurno] = useState<TurnoEmCurso | null>(null);
    const [encerramento, setEncerramento] = useState<Encerramento>(null);
    const [aviso, setAviso] = useState<Aviso>(null);
    const [reabrindo, setReabrindo] = useState(false);
    /** Turno cujo "por quê" está aberto no painel (HR-47). `null`: painel fechado. */
    const [turnoAberto, setTurnoAberto] = useState<string | null>(null);
    /** Prévia de imagem do cliente aberta (HR-46c). `null`: fechada. */
    const [previa, setPrevia] = useState<PreviaDeImagem | null>(null);
    /** Arquivo que está baixando agora (chave), para o botão mostrar o carregando. */
    const [baixandoAnexo, setBaixandoAnexo] = useState<string | null>(null);
    /** Falha do último download, com a chave do arquivo: a linha mostra o motivo e "Tentar de novo". */
    const [erroDeDownload, setErroDeDownload] = useState<{ chave: string; texto: string } | null>(
        null,
    );

    /** Conversa que está na tela agora: evento de outra conversa é descartado. */
    const ativaRef = useRef<string | null>(null);
    /** Turno em voo (um por vez). Abortar é o cancelamento. */
    const controleRef = useRef<AbortController | null>(null);
    const registroRef = useRef<HTMLDivElement | null>(null);
    /** Pedido de prévia mais recente: resposta de pedido antigo (fechada ou de outra conversa) cai fora. */
    const pedidoPreviaRef = useRef(0);
    /** URLs `blob:` de download ainda não liberadas. Liberadas ao trocar de conversa e ao sair da tela. */
    const urlsDeDownloadRef = useRef<Set<string>>(new Set());

    const transferida = ehTransferida(historico?.conversation);
    const bloqueada = transferida || !historico;

    const carregarAgente = useCallback(async () => {
        setErroDeCarga(null);
        try {
            setAgente(await getAgent(orgId, agentId));
        } catch (falha) {
            setErroDeCarga(mensagemDe(falha));
        }
    }, [orgId, agentId]);

    /** Lê a primeira página. Mantém conversas de páginas seguintes que já estavam na lista. */
    const atualizarLista = useCallback(async () => {
        const lista = await listTestConversations(orgId, agentId);
        setConversas((atuais) => {
            const novas = new Set(lista.items.map((item) => item.id));
            return [...lista.items, ...atuais.filter((item) => !novas.has(item.id))];
        });
        setProximoCursor(lista.next_cursor);
        return lista;
    }, [orgId, agentId]);

    const carregarHistorico = useCallback(
        async (conversa: string) => {
            setCarregandoHistorico(true);
            setErroHistorico(null);
            try {
                const lido = await getTestHistory(orgId, agentId, conversa);
                if (ativaRef.current !== conversa) return;
                setHistorico(lido);
                setConversas((atuais) =>
                    atuais.map((item) => (item.id === lido.conversation.id ? lido.conversation : item)),
                );
            } catch (falha) {
                if (ativaRef.current === conversa) setErroHistorico(mensagemDe(falha));
            } finally {
                if (ativaRef.current === conversa) setCarregandoHistorico(false);
            }
        },
        [orgId, agentId],
    );

    /** Solta uma URL `blob:` de download: a memória do arquivo volta e a URL deixa de valer. */
    const liberarUrl = useCallback((url: string) => {
        if (urlsDeDownloadRef.current.delete(url)) URL.revokeObjectURL(url);
    }, []);

    /** Solta todas as URLs de download pendentes (troca de conversa e saída da tela). */
    const liberarDownloads = useCallback(() => {
        for (const url of [...urlsDeDownloadRef.current]) liberarUrl(url);
    }, [liberarUrl]);

    /**
     * Troca a conversa da tela. Se há turno em voo na conversa anterior, ele é abortado (o servidor
     * cancela) e nada dele entra na conversa nova: o rascunho e os arquivos também ficam para trás.
     */
    const abrirConversa = useCallback(
        (id: string) => {
            controleRef.current?.abort();
            controleRef.current = null;
            ativaRef.current = id;
            setAtiva(id);
            setHistorico(null);
            setErroHistorico(null);
            setTurno(null);
            setEncerramento(null);
            setAviso(null);
            setEnviando(false);
            setErroDeCampo(null);
            setTurnoAberto(null);
            setRascunho("");
            setArquivos([]);
            // Arquivo de outra conversa não fica na tela nem em download pendente (HR-46c).
            pedidoPreviaRef.current += 1;
            setPrevia(null);
            setBaixandoAnexo(null);
            setErroDeDownload(null);
            liberarDownloads();
            void carregarHistorico(id);
        },
        [carregarHistorico, liberarDownloads],
    );

    /** Ao sair da tela, nenhuma prévia que chegue depois aparece e as URLs de download são soltas. */
    useEffect(() => {
        return () => {
            pedidoPreviaRef.current += 1;
            liberarDownloads();
        };
    }, [liberarDownloads]);

    useEffect(() => {
        void carregarAgente();
        getOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
    }, [carregarAgente, orgId]);

    useEffect(() => {
        let cancelado = false;
        setCarregandoLista(true);
        setErroLista(null);
        listTestConversations(orgId, agentId)
            .then((lista) => {
                if (cancelado) return;
                setConversas(lista.items);
                setProximoCursor(lista.next_cursor);
                const primeira = lista.items[0];
                if (primeira) abrirConversa(primeira.id);
            })
            .catch((falha: unknown) => {
                if (!cancelado) setErroLista(mensagemDe(falha));
            })
            .finally(() => {
                if (!cancelado) setCarregandoLista(false);
            });
        return () => {
            cancelado = true;
            controleRef.current?.abort();
        };
    }, [orgId, agentId, abrirConversa]);

    /** Mantém a última mensagem à vista quando o texto cresce. */
    useEffect(() => {
        const registro = registroRef.current;
        if (registro) registro.scrollTop = registro.scrollHeight;
    }, [historico, turno]);

    async function carregarMais() {
        if (!proximoCursor) return;
        setErroLista(null);
        try {
            const lista = await listTestConversations(orgId, agentId, proximoCursor);
            setConversas((atuais) => [...atuais, ...lista.items]);
            setProximoCursor(lista.next_cursor);
        } catch (falha) {
            setErroLista(mensagemDe(falha));
        }
    }

    async function novaConversa() {
        setCriando(true);
        setAviso(null);
        try {
            const nova = await createTestConversation(orgId, agentId);
            setConversas((atuais) => [nova, ...atuais]);
            abrirConversa(nova.id);
        } catch (falha) {
            setAviso({
                tom: "danger",
                titulo: t("testChat.newConversationError"),
                texto: mensagemDe(falha),
            });
        } finally {
            setCriando(false);
        }
    }

    /** Aplica um evento do turno ao que a tela mostra. Só chamado para a conversa da tela. */
    function tratarEvento(evento: TurnEvent) {
        const dados = campos(evento);
        switch (evento.tipo) {
            case "texto": {
                const delta = texto(dados.delta) ?? "";
                setTurno((atual) => (atual ? { ...atual, resposta: atual.resposta + delta } : atual));
                break;
            }
            case "base": {
                const fontes = Array.isArray(dados.sources) ? dados.sources.length : 0;
                setTurno((atual) => (atual ? { ...atual, fontes } : atual));
                break;
            }
            case "ferramenta": {
                const nome = texto(dados.name) ?? t("testChat.toolFallback");
                setTurno((atual) =>
                    atual ? { ...atual, ferramentas: [...atual.ferramentas, nome] } : atual,
                );
                break;
            }
            case "transferencia":
                setTurno((atual) => (atual ? { ...atual, transferido: true } : atual));
                break;
            case "fim": {
                const timings = (dados.timings ?? {}) as Record<string, unknown>;
                const estado = texto(dados.state) ?? "desconhecido";
                setEncerramento({
                    estado,
                    tokensEntrada: numero(dados.input_tokens),
                    tokensSaida: numero(dados.output_tokens),
                    totalMs: numero(timings.total_ms),
                });
                const final = texto(dados.text);
                if (final) {
                    setTurno((atual) => (atual ? { ...atual, resposta: final } : atual));
                }
                if (estado === "failed") {
                    setAviso({
                        tom: "danger",
                        titulo: t("testChat.turnFailedTitle"),
                        texto: texto(dados.error) ?? t("testChat.turnFailedText"),
                    });
                } else if (estado === "skipped") {
                    setAviso({
                        tom: "info",
                        titulo: t("testChat.skippedTitle"),
                        texto: t("testChat.skippedText"),
                    });
                }
                break;
            }
            case "erro":
                setTurno((atual) =>
                    atual
                        ? {
                              ...atual,
                              erro: texto(dados.message) ?? t("testChat.turnStartError"),
                          }
                        : atual,
                );
                break;
            default:
                // `mensagem` (o que foi gravado) e `inicio`: não mudam nada na tela.
                break;
        }
    }

    /** Traduz um erro antes do turno começar. Devolve o aviso, ou marca o campo que a API apontou. */
    function tratarFalhaDeEnvio(falha: unknown): Aviso {
        if (falha instanceof ApiError) {
            if (falha.status === 409) {
                return {
                    tom: "warn",
                    titulo: t("testChat.busyTitle"),
                    texto: t("testChat.busyText"),
                };
            }
            if (falha.status === 503) {
                return {
                    tom: "danger",
                    titulo: t("testChat.noTurnTitle"),
                    texto: t("testChat.noTurnText"),
                };
            }
            if (falha.status === 400) {
                const detalhes = (falha.detalhes ?? {}) as Record<string, unknown>;
                const campo = detalhes.field === "file" ? "file" : "text";
                setErroDeCampo({ campo, mensagem: falha.message });
                return null;
            }
        }
        return {
            tom: "danger",
            titulo: t("testChat.sendErrorTitle"),
            texto: mensagemDe(falha),
        };
    }

    async function enviar(evento?: FormEvent<HTMLFormElement>) {
        evento?.preventDefault();
        if (bloqueada || enviando || !historico) return;

        const textoDoCliente = rascunho.trim();
        if (!textoDoCliente && arquivos.length === 0) {
            setErroDeCampo({ campo: "text", mensagem: t("testChat.invalidMessage") });
            return;
        }

        const conversa = historico.conversation.id;
        const enviados = arquivos;
        setErroDeCampo(null);
        setAviso(null);
        setEncerramento(null);
        setRascunho("");
        setArquivos([]);

        const controle = new AbortController();
        controleRef.current = controle;
        setEnviando(true);
        setTurno({
            textoDoCliente,
            anexosDoCliente: enviados.map((arquivo) => arquivo.name),
            resposta: "",
            ferramentas: [],
            fontes: null,
            transferido: false,
            erro: null,
        });

        let iniciou = false;
        try {
            await sendTestMessage(
                orgId,
                agentId,
                conversa,
                { texto: textoDoCliente, arquivos: enviados },
                {
                    sinal: controle.signal,
                    aoEvento: (evento) => {
                        if (ativaRef.current !== conversa) return;
                        iniciou = true;
                        tratarEvento(evento);
                    },
                },
            );
        } catch (falha) {
            if (ativaRef.current === conversa) {
                if (ehAbortado(falha)) {
                    setAviso({
                        tom: "info",
                        titulo: t("testChat.cancelTitle"),
                        texto: t("testChat.cancelText"),
                    });
                } else {
                    const avisoDaFalha = tratarFalhaDeEnvio(falha);
                    if (avisoDaFalha) setAviso(avisoDaFalha);
                    if (!iniciou) {
                        // Nada foi gravado: devolve o que o operador digitou e anexou.
                        setRascunho(textoDoCliente);
                        setArquivos(enviados);
                    }
                }
            }
        } finally {
            if (ativaRef.current === conversa) {
                await carregarHistorico(conversa);
                void atualizarLista().catch(() => undefined);
            }
            if (controleRef.current === controle) {
                controleRef.current = null;
                setEnviando(false);
                setTurno(null);
            }
        }
    }

    /**
     * Abre a prévia de uma imagem do cliente. Busca o arquivo a cada abertura (a tela não guarda
     * conteúdo) e só mostra a imagem se a conversa e a prévia ainda forem as mesmas quando ela chega.
     */
    async function abrirPrevia(origem: FileOrigin) {
        if (!ativaRef.current) return;
        const pedido = ++pedidoPreviaRef.current;
        setPrevia({ origem, estado: "carregando", src: null, erro: null });
        try {
            const arquivo = await downloadContent(origem.caminho);
            const src = await readAsDataUrl(arquivo.blob, origem.mime);
            if (pedido !== pedidoPreviaRef.current) return;
            setPrevia({ origem, estado: "pronta", src, erro: null });
        } catch (falha) {
            if (pedido !== pedidoPreviaRef.current) return;
            setPrevia({ origem, estado: "erro", src: null, erro: mensagemDe(falha) });
        }
    }

    function fecharPrevia() {
        pedidoPreviaRef.current += 1;
        setPrevia(null);
    }

    /**
     * Baixa o arquivo (do cliente ou do agente) para o computador do operador: um `blob:` com o nome
     * do arquivo, clicado num link temporário. A URL é solta logo depois (um minuto, ou antes se a
     * conversa mudar).
     */
    async function baixarArquivo(origem: FileOrigin) {
        const conversa = ativaRef.current;
        if (!conversa) return;
        setBaixandoAnexo(origem.chave);
        setErroDeDownload(null);
        try {
            const arquivo = await downloadContent(origem.caminho);
            if (ativaRef.current !== conversa) return;
            const url = URL.createObjectURL(arquivo.blob);
            urlsDeDownloadRef.current.add(url);
            const link = document.createElement("a");
            link.href = url;
            link.download = arquivo.nome ?? origem.nome;
            document.body.appendChild(link);
            link.click();
            link.remove();
            window.setTimeout(() => liberarUrl(url), 60_000);
        } catch (falha) {
            if (ativaRef.current !== conversa) return;
            setErroDeDownload({ chave: origem.chave, texto: mensagemDe(falha) });
        } finally {
            if (ativaRef.current === conversa) setBaixandoAnexo(null);
        }
    }

    const acoesDeArquivo: AcoesDeArquivo = {
        baixando: baixandoAnexo,
        erro: erroDeDownload,
        ver: (origem) => void abrirPrevia(origem),
        baixar: (origem) => void baixarArquivo(origem),
        doAnexo: (mensagemId, anexo) =>
            clientAttachmentOrigin(orgId, agentId, ativa ?? "", mensagemId, anexo),
        doAgente: (arquivo) => agentFileOrigin(orgId, agentId, arquivo),
    };

    function cancelarTurno() {
        controleRef.current?.abort();
    }

    /**
     * Abre o "por quê" de um turno. Relê a conversa antes, para o painel mostrar o que o turno gravou
     * até agora; se a leitura falhar, o painel mostra o erro com "Tentar de novo".
     */
    function abrirPorQue(turnoId: string) {
        setTurnoAberto(turnoId);
        if (ativaRef.current) void carregarHistorico(ativaRef.current);
    }

    function escolherArquivos(evento: ChangeEvent<HTMLInputElement>) {
        const escolhidos = Array.from(evento.target.files ?? []);
        evento.target.value = "";
        const aceitos: File[] = [];
        for (const arquivo of escolhidos) {
            if (arquivo.size > LIMIT_FILE_BYTES) {
                setErroDeCampo({
                    campo: "file",
                    mensagem: t("testChat.fileTooLarge", {
                        name: arquivo.name,
                        size: tamanhoLegivel(arquivo.size),
                    }),
                });
                continue;
            }
            aceitos.push(arquivo);
        }
        if (aceitos.length > 0) {
            setErroDeCampo(null);
            setArquivos((atuais) => [...atuais, ...aceitos]);
        }
    }

    function tirarArquivo(indice: number) {
        setArquivos((atuais) => atuais.filter((_, posicao) => posicao !== indice));
    }

    /** Enter envia; Shift+Enter quebra a linha. */
    function teclaNoTexto(evento: KeyboardEvent<HTMLTextAreaElement>) {
        if (evento.key === "Enter" && !evento.shiftKey && !evento.nativeEvent.isComposing) {
            evento.preventDefault();
            void enviar();
        }
    }

    async function reabrir() {
        if (!historico || reabrindo || enviando) return;
        const conversa = historico.conversation.id;
        setReabrindo(true);
        setAviso(null);
        try {
            await reopenTestConversation(orgId, agentId, conversa, historico.conversation.version);
            if (ativaRef.current === conversa) {
                setAviso({
                    tom: "info",
                    titulo: t("testChat.reopenedTitle"),
                    texto: t("testChat.reopenedText"),
                });
            }
        } catch (falha) {
            if (ativaRef.current === conversa) {
                if (falha instanceof ApiError && falha.status === 412) {
                    setAviso({
                        tom: "warn",
                        titulo: t("testChat.conflictTitle"),
                        texto: t("testChat.conflictText"),
                    });
                } else {
                    setAviso({
                        tom: "danger",
                        titulo: t("testChat.reopenErrorTitle"),
                        texto: mensagemDe(falha),
                    });
                }
            }
        } finally {
            setReabrindo(false);
            if (ativaRef.current === conversa) {
                await carregarHistorico(conversa);
                void atualizarLista().catch(() => undefined);
            }
        }
    }

    if (!agente && erroDeCarga) {
        return (
            <div className="poph-pagina">
                <SofaErrorState
                    title={t("testChat.pageLoadTitle")}
                    description={t("testChat.pageLoadDescription")}
                    details={erroDeCarga}
                    retry={{ label: t("testChat.retry"), onClick: () => void carregarAgente() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => navegar(`/orgs/${orgId}/agents`)}
                        >
                            {t("testChat.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente) {
        return (
            <div className="poph-pagina" aria-busy="true">
                <SofaSkeleton width={260} height={26} />
                <SofaSkeleton lines={6} />
            </div>
        );
    }

    const mensagens: TestMessage[] = historico?.messages ?? [];
    /** Posição de cada turno na conversa (1 é o primeiro), como o painel e os botões a mostram. */
    const numeroDoTurno = new Map<string, number>(
        (historico?.turns ?? []).map((turnoDaLista, indice): [string, number] => [turnoDaLista.id, indice + 1]),
    );
    /** Turnos sem mensagem de resposta: falha, corte, "não respondeu" ou ainda em andamento. */
    const turnosPorId = new Map<string, TestTurn>(
        (historico?.turns ?? []).map((turnoDaLista): [string, TestTurn] => [turnoDaLista.id, turnoDaLista]),
    );
    const turnosSemResposta = (historico?.turns ?? []).filter(
        (turnoDaLista) => !mensagens.some((mensagem) => mensagem.turn_id === turnoDaLista.id),
    );
    const itensDoRegistro = interleaveTurns(mensagens, turnosSemResposta);
    const turnoDoPainel = turnoAberto
        ? (historico?.turns ?? []).find((turnoDaLista) => turnoDaLista.id === turnoAberto) ?? null
        : null;

    return (
        <div className="poph-pagina poph-chat-teste">
            <CabecalhoDoAgente
                org={orgId}
                nome={agente.name}
                slug={agente.slug}
                situacao={agente.status}
                organizacao={nomeDaOrg}
                descricao={t("testChat.chatDescription")}
            />
            <AbasDoAgente org={orgId} agente={agentId} ativa="test-chat" nome={agente.name} model={agente.model ?? null} />

            <div className="poph-chat-teste__corpo">
                <aside className="poph-chat-teste__lista" aria-label={t("testChat.conversationsLabel")}>
                    <div className="poph-chat-teste__lista-topo">
                        <SofaButton
                            variant="primary"
                            icon={<IconPlus aria-hidden="true" />}
                            onClick={() => void novaConversa()}
                            loading={criando}
                            disabled={enviando}
                            fullWidth
                        >
                            {t("testChat.newConversation")}
                        </SofaButton>
                        <SofaButton
                            variant="subtle"
                            icon={<IconRefresh aria-hidden="true" />}
                            onClick={() => {
                                setErroLista(null);
                                void atualizarLista().catch((falha: unknown) =>
                                    setErroLista(mensagemDe(falha)),
                                );
                            }}
                        >
                            {t("testChat.refresh")}
                        </SofaButton>
                    </div>

                    {carregandoLista && conversas.length === 0 ? (
                        <SofaSkeleton lines={4} />
                    ) : erroLista && conversas.length === 0 ? (
                        <SofaErrorState
                            title={t("testChat.listErrorTitle")}
                            description={t("testChat.listErrorDescription")}
                            details={erroLista}
                            retry={{
                                label: t("testChat.retry"),
                                onClick: () => {
                                    setErroLista(null);
                                    void atualizarLista().catch((falha: unknown) =>
                                        setErroLista(mensagemDe(falha)),
                                    );
                                },
                            }}
                        />
                    ) : conversas.length === 0 ? (
                        <p className="poph-chat-teste__vazio">{t("testChat.listEmpty")}</p>
                    ) : (
                        <ul className="poph-chat-teste__itens" aria-label={t("testChat.conversationsListLabel")}>
                            {conversas.map((conversa) => {
                                const selecionada = conversa.id === ativa;
                                return (
                                    <li key={conversa.id}>
                                        <button
                                            type="button"
                                            className={
                                                selecionada
                                                    ? "poph-chat-teste__item poph-chat-teste__item--ativo"
                                                    : "poph-chat-teste__item"
                                            }
                                            aria-current={selecionada ? "true" : undefined}
                                            title={conversa.id}
                                            onClick={() => abrirConversa(conversa.id)}
                                        >
                                            <span className="poph-chat-teste__item-linha">
                                                <span className="poph-mono-mudo">
                                                    {idCurto(conversa.id)}
                                                </span>
                                                <span
                                                    className={`poph-status ${classeDoStatus(conversa.status)}`}
                                                >
                                                    <span className="poph-status__ponto" aria-hidden="true" />
                                                    {rotuloDoStatus(conversa.status)}
                                                </span>
                                            </span>
                                            <span className="poph-chat-teste__item-meta">
                                                {t("testChat.conversationMeta", {
                                                    date: dataCurta(conversa.created_at),
                                                    count: formatNumber(conversa.message_count),
                                                })}
                                            </span>
                                        </button>
                                    </li>
                                );
                            })}
                        </ul>
                    )}

                    {proximoCursor && (
                        <SofaButton variant="subtle" onClick={() => void carregarMais()}>
                            {t("testChat.loadMore")}
                        </SofaButton>
                    )}
                </aside>

                <section className="poph-chat-teste__conversa" aria-label={t("testChat.conversationSectionLabel")}>
                    {!ativa ? (
                        <SofaEmptyState
                            icon={<IconMessage aria-hidden="true" />}
                            title={t("testChat.chooseTitle")}
                            description={t("testChat.chooseDescription")}
                            action={
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => void novaConversa()}
                                    loading={criando}
                                >
                                    {t("testChat.newConversation")}
                                </SofaButton>
                            }
                        />
                    ) : erroHistorico && !historico ? (
                        <SofaErrorState
                            title={t("testChat.openErrorTitle")}
                            description={t("testChat.openErrorDescription")}
                            details={erroHistorico}
                            retry={{ label: t("testChat.retry"), onClick: () => void carregarHistorico(ativa) }}
                        />
                    ) : !historico ? (
                        <SofaSkeleton lines={6} />
                    ) : (
                        <>
                            <div className="poph-chat-teste__cabecalho">
                                <span className="poph-mono-mudo" title={historico.conversation.id}>
                                    {t("testChat.conversationLabel", { id: idCurto(historico.conversation.id) })}
                                </span>
                                <span
                                    className={`poph-status ${classeDoStatus(historico.conversation.status)}`}
                                >
                                    <span className="poph-status__ponto" aria-hidden="true" />
                                    {rotuloDoStatus(historico.conversation.status)}
                                </span>
                                {carregandoHistorico && (
                                    <span className="poph-chat-teste__atualizando">{t("testChat.updating")}</span>
                                )}
                            </div>

                            {transferida && (
                                <div className="poph-chat-teste__transferida">
                                    <SofaBanner tone="warn" title={t("testChat.transferredTitle")}>
                                        {t("testChat.transferredDescription")}
                                    </SofaBanner>
                                    <SofaButton
                                        variant="secondary"
                                        onClick={() => void reabrir()}
                                        loading={reabrindo}
                                        disabled={enviando}
                                    >
                                        {t("testChat.reopen")}
                                    </SofaButton>
                                </div>
                            )}

                            <div
                                ref={registroRef}
                                className="poph-chat-teste__registro"
                                role="log"
                                aria-live="polite"
                                aria-label={t("testChat.logLabel")}
                            >
                                {mensagens.length === 0 && turnosSemResposta.length === 0 && !turno && (
                                    <p className="poph-chat-teste__vazio">{t("testChat.noMessages")}</p>
                                )}
                                {itensDoRegistro.map((item) =>
                                    item.tipo === "mensagem" ? (
                                        <Balao
                                            key={item.mensagem.id}
                                            mensagem={item.mensagem}
                                            numeroDoTurno={
                                                item.mensagem.turn_id
                                                    ? (numeroDoTurno.get(item.mensagem.turn_id) ?? null)
                                                    : null
                                            }
                                            arquivosDoAgente={
                                                item.mensagem.turn_id
                                                    ? (turnosPorId.get(item.mensagem.turn_id)?.files_out ?? [])
                                                    : []
                                            }
                                            aoVerPorQue={abrirPorQue}
                                            acoesDeArquivo={acoesDeArquivo}
                                        />
                                    ) : (
                                        <TurnoSemResposta
                                            key={`turno-${item.turno.id}`}
                                            turno={item.turno}
                                            numero={numeroDoTurno.get(item.turno.id) ?? null}
                                            aoVerPorQue={abrirPorQue}
                                            acoesDeArquivo={acoesDeArquivo}
                                        />
                                    ),
                                )}

                                {turno && (
                                    <>
                                        <div className="poph-chat-teste__balao poph-chat-teste__balao--cliente">
                                            <span className="poph-chat-teste__autor">{t("testChat.client")}</span>
                                            {turno.textoDoCliente && <p>{turno.textoDoCliente}</p>}
                                            {turno.anexosDoCliente.length > 0 && (
                                                <ul className="poph-chat-teste__anexos">
                                                    {turno.anexosDoCliente.map((nome, indice) => (
                                                        <li key={`${nome}-${indice}`}>{nome}</li>
                                                    ))}
                                                </ul>
                                            )}
                                        </div>
                                        <div
                                            className="poph-chat-teste__balao poph-chat-teste__balao--agente"
                                            aria-busy={turno.resposta === "" && !turno.erro}
                                        >
                                            <span className="poph-chat-teste__autor">{t("testChat.agent")}</span>
                                            {turno.ferramentas.map((nome, indice) => (
                                                <p key={`${nome}-${indice}`} className="poph-chat-teste__evento">
                                                    {t("testChat.toolRunning", { name: nome })}
                                                </p>
                                            ))}
                                            {turno.fontes !== null && (
                                                <p className="poph-chat-teste__evento">
                                                    {turno.fontes === 1
                                                        ? t("testChat.sourcesOne")
                                                        : t("testChat.sourcesMany", { count: formatNumber(turno.fontes) })}
                                                </p>
                                            )}
                                            {turno.transferido && (
                                                <p className="poph-chat-teste__evento">
                                                    {t("testChat.conversationTransferred")}
                                                </p>
                                            )}
                                            {turno.resposta ? (
                                                <p className="poph-chat-teste__texto-agente">
                                                    {turno.resposta}
                                                    {enviando && !encerramento && (
                                                        <span aria-hidden="true" className="poph-chat-teste__cursor">
                                                            ▌
                                                        </span>
                                                    )}
                                                </p>
                                            ) : (
                                                !turno.erro && (
                                                    <p className="poph-chat-teste__evento">{t("testChat.thinking")}</p>
                                                )
                                            )}
                                            {turno.erro && (
                                                <p className="poph-chat-teste__erro">{turno.erro}</p>
                                            )}
                                        </div>
                                    </>
                                )}
                            </div>

                            {encerramento?.estado === "done" && !turno && (
                                <p className="poph-chat-teste__fim">
                                    {t("testChat.turnDone")}
                                    {encerramento.tokensEntrada !== null &&
                                        t("testChat.turnDoneInput", {
                                            value: formatNumber(encerramento.tokensEntrada),
                                        })}
                                    {encerramento.tokensSaida !== null &&
                                        t("testChat.turnDoneOutput", {
                                            value: formatNumber(encerramento.tokensSaida),
                                        })}
                                    {encerramento.totalMs !== null &&
                                        t("testChat.turnDoneTime", {
                                            value: formatNumber(encerramento.totalMs),
                                        })}
                                </p>
                            )}

                            {aviso && (
                                <div className="poph-chat-teste__aviso" role="status">
                                    <SofaBanner tone={aviso.tom} title={aviso.titulo}>
                                        {aviso.texto}
                                    </SofaBanner>
                                </div>
                            )}

                            <PreviaArquivoTeste
                                previa={previa}
                                aoFechar={fecharPrevia}
                                aoTentarDeNovo={() => {
                                    if (previa) void abrirPrevia(previa.origem);
                                }}
                            />

                            <PainelPorQue
                                aberto={turnoAberto !== null}
                                numero={turnoDoPainel ? (numeroDoTurno.get(turnoDoPainel.id) ?? null) : null}
                                turno={turnoDoPainel}
                                atualizando={carregandoHistorico}
                                erroDeLeitura={turnoAberto !== null ? erroHistorico : null}
                                aoFechar={() => setTurnoAberto(null)}
                                aoTentarDeNovo={() => {
                                    if (ativaRef.current) void carregarHistorico(ativaRef.current);
                                }}
                            />

                            <form
                                className="poph-chat-teste__formulario"
                                onSubmit={(evento) => void enviar(evento)}
                                aria-label={t("testChat.formLabel")}
                            >
                                <div className="poph-chat-teste__campo">
                                    <label htmlFor="chat-teste-texto" className="poph-chat-teste__rotulo">
                                        {t("testChat.messageLabel")}
                                    </label>
                                    <textarea
                                        id="chat-teste-texto"
                                        className="poph-chat-teste__texto"
                                        rows={3}
                                        value={rascunho}
                                        disabled={bloqueada || enviando}
                                        placeholder={
                                            transferida
                                                ? t("testChat.placeholderTransferred")
                                                : t("testChat.placeholder")
                                        }
                                        aria-invalid={erroDeCampo?.campo === "text" || undefined}
                                        aria-describedby={
                                            erroDeCampo?.campo === "text" ? "chat-teste-erro-texto" : undefined
                                        }
                                        onChange={(evento) => setRascunho(evento.target.value)}
                                        onKeyDown={teclaNoTexto}
                                    />
                                    {erroDeCampo?.campo === "text" && (
                                        <p id="chat-teste-erro-texto" className="poph-chat-teste__erro">
                                            {erroDeCampo.mensagem}
                                        </p>
                                    )}
                                </div>

                                <div className="poph-chat-teste__anexar">
                                    <label
                                        htmlFor="chat-teste-arquivos"
                                        className="poph-chat-teste__botao-anexar"
                                    >
                                        <IconUpload aria-hidden="true" size={14} />
                                        {t("testChat.attach")}
                                    </label>
                                    <input
                                        id="chat-teste-arquivos"
                                        type="file"
                                        multiple
                                        accept={ACEITOS}
                                        className="poph-chat-teste__arquivo"
                                        disabled={bloqueada || enviando}
                                        aria-describedby="chat-teste-ajuda-arquivos"
                                        onChange={escolherArquivos}
                                    />
                                    <span id="chat-teste-ajuda-arquivos" className="poph-chat-teste__ajuda">
                                        {t("testChat.attachHelp")}
                                    </span>
                                </div>

                                {arquivos.length > 0 && (
                                    <ul className="poph-chat-teste__anexos-na-fila" aria-label={t("testChat.filesToSendLabel")}>
                                        {arquivos.map((arquivo, indice) => (
                                            <li key={`${arquivo.name}-${indice}`}>
                                                <span>
                                                    {arquivo.name} · {tamanhoLegivel(arquivo.size)}
                                                </span>
                                                <button
                                                    type="button"
                                                    className="poph-chat-teste__tirar"
                                                    aria-label={t("testChat.removeFileLabel", { name: arquivo.name })}
                                                    disabled={enviando}
                                                    onClick={() => tirarArquivo(indice)}
                                                >
                                                    <IconXCircle aria-hidden="true" size={14} />
                                                </button>
                                            </li>
                                        ))}
                                    </ul>
                                )}

                                {erroDeCampo?.campo === "file" && (
                                    <p className="poph-chat-teste__erro" role="alert">
                                        {erroDeCampo.mensagem}
                                    </p>
                                )}

                                <div className="poph-chat-teste__acoes">
                                    {enviando ? (
                                        <SofaButton variant="danger" onClick={cancelarTurno}>
                                            {t("testChat.cancelTurn")}
                                        </SofaButton>
                                    ) : (
                                        <SofaButton
                                            type="submit"
                                            variant="primary"
                                            icon={<IconSend aria-hidden="true" />}
                                            disabled={bloqueada}
                                        >
                                            {t("testChat.send")}
                                        </SofaButton>
                                    )}
                                </div>
                            </form>
                        </>
                    )}
                </section>
            </div>
        </div>
    );
}

/** Ações sobre os arquivos do histórico: do cliente (HR-46c) e do agente (HR-46e). */
type AcoesDeArquivo = {
    /** Chave do arquivo que está baixando agora, se houver. */
    baixando: string | null;
    /** Falha do último download, com a chave do arquivo a que se refere. */
    erro: { chave: string; texto: string } | null;
    ver: (origem: FileOrigin) => void;
    baixar: (origem: FileOrigin) => void;
    /** Monta a origem de um anexo que o cliente mandou numa mensagem da conversa. */
    doAnexo: (mensagemId: string, anexo: TestAttachment) => FileOrigin;
    /** Monta a origem de um arquivo que o agente enviou (item de `files_out`). */
    doAgente: (arquivo: TestSentFile) => FileOrigin;
};

/** Um balão do histórico. `in` é o cliente, `out` é o agente. */
function Balao({
    mensagem,
    numeroDoTurno,
    arquivosDoAgente,
    aoVerPorQue,
    acoesDeArquivo,
}: {
    mensagem: TestMessage;
    numeroDoTurno: number | null;
    /** Arquivos que o agente enviou no turno desta resposta (`files_out`). */
    arquivosDoAgente: TestSentFile[];
    aoVerPorQue: (turnoId: string) => void;
    acoesDeArquivo: AcoesDeArquivo;
}) {
    const { translate: t } = useI18n();
    const doCliente = mensagem.direction === "in";
    const turnoDaResposta = mensagem.turn_id;
    return (
        <div
            className={
                doCliente
                    ? "poph-chat-teste__balao poph-chat-teste__balao--cliente"
                    : "poph-chat-teste__balao poph-chat-teste__balao--agente"
            }
        >
            <span className="poph-chat-teste__autor">{doCliente ? t("testChat.client") : t("testChat.agent")}</span>
            {mensagem.text && <p className="poph-chat-teste__texto-agente">{mensagem.text}</p>}
            {mensagem.attachments.length > 0 && (
                <ul className="poph-chat-teste__anexos">
                    {mensagem.attachments.map((anexo) => (
                        <li key={anexo.id}>
                            {anexo.name} · {tamanhoLegivel(anexo.size_bytes)}
                            {anexo.has_text ? t("testChat.extractedText") : ""}
                            {/* Só o anexo do cliente tem ação aqui; o arquivo do agente vem do turno (`files_out`). */}
                            {doCliente && (
                                <AcoesDoArquivo
                                    origem={acoesDeArquivo.doAnexo(mensagem.id, anexo)}
                                    acoes={acoesDeArquivo}
                                />
                            )}
                        </li>
                    ))}
                </ul>
            )}
            {!doCliente && arquivosDoAgente.length > 0 && (
                <ArquivosDoAgente arquivos={arquivosDoAgente} acoes={acoesDeArquivo} />
            )}
            {!doCliente && turnoDaResposta && numeroDoTurno !== null && (
                <div>
                    <SofaButton variant="subtle" onClick={() => aoVerPorQue(turnoDaResposta)}>
                        {t("testChat.viewWhy", { number: numeroDoTurno })}
                    </SofaButton>
                </div>
            )}
        </div>
    );
}

/**
 * Ver e baixar um arquivo (do cliente ou do agente). Imagem tem "Ver" (prévia) e "Baixar"; PDF e
 * Office têm só "Baixar", porque abrir o arquivo em outra aba depende de bloqueador de pop-up e de
 * visualizador do navegador. O rótulo de cada botão traz o nome do arquivo, para quem usa leitor de
 * tela. Baixar não depende do estado do envio (um arquivo "falhou" ainda pode ser baixado).
 */
function AcoesDoArquivo({ origem, acoes }: { origem: FileOrigin; acoes: AcoesDeArquivo }) {
    const { translate: t } = useI18n();
    const baixandoEste = acoes.baixando === origem.chave;
    const falhou = acoes.erro?.chave === origem.chave ? acoes.erro.texto : null;
    return (
        <span className="poph-chat-teste__anexo-acoes">
            {canSeeOnScreen(origem.mime) && (
                <SofaButton
                    variant="subtle"
                    aria-label={t("testChat.viewFileLabel", { name: origem.nome })}
                    onClick={() => acoes.ver(origem)}
                >
                    {t("testChat.view")}
                </SofaButton>
            )}
            <SofaButton
                variant="subtle"
                aria-label={t("testChat.downloadFileLabel", { name: origem.nome })}
                loading={baixandoEste}
                disabled={acoes.baixando !== null && !baixandoEste}
                onClick={() => acoes.baixar(origem)}
            >
                {t("testChat.download")}
            </SofaButton>
            {falhou && (
                <span className="poph-chat-teste__anexo-erro" role="alert">
                    <span className="poph-chat-teste__erro">{falhou}</span>
                    <SofaButton
                        variant="secondary"
                        aria-label={t("testChat.retryDownloadLabel", { name: origem.nome })}
                        onClick={() => acoes.baixar(origem)}
                    >
                        {t("testChat.retry")}
                    </SofaButton>
                </span>
            )}
        </span>
    );
}

/**
 * Arquivos que o agente enviou no turno (HR-46e). Cada um mostra o nome, o tamanho, o estado do
 * envio (`enviado`, `na fila`, `falhou`) e as mesmas ações de baixar e ver.
 */
function ArquivosDoAgente({
    arquivos,
    acoes,
}: {
    arquivos: TestSentFile[];
    acoes: AcoesDeArquivo;
}) {
    const { translate: t } = useI18n();
    return (
        <ul className="poph-chat-teste__anexos" aria-label={t("testChat.agentFilesLabel")}>
            {arquivos.map((arquivo) => (
                <li key={arquivo.id}>
                    {arquivo.name} · {tamanhoLegivel(arquivo.size_bytes)} ·{" "}
                    <span className={arquivo.state === "failed" ? "poph-chat-teste__erro" : undefined}>
                        {sendLabel(arquivo.state)}
                    </span>
                    <AcoesDoArquivo origem={acoes.doAgente(arquivo)} acoes={acoes} />
                </li>
            ))}
        </ul>
    );
}

/** Turno que não deixou mensagem do agente (falha, corte, "não respondeu"): aparece no registro. */
function TurnoSemResposta({
    turno,
    numero,
    aoVerPorQue,
    acoesDeArquivo,
}: {
    turno: TestTurn;
    numero: number | null;
    aoVerPorQue: (turnoId: string) => void;
    acoesDeArquivo: AcoesDeArquivo;
}) {
    const { translate: t } = useI18n();
    return (
        <div className="poph-chat-teste__balao poph-chat-teste__balao--agente poph-chat-teste__sem-resposta">
            <span className="poph-chat-teste__autor">
                {numero !== null
                    ? t("testChat.turnHeading", { number: numero, state: turnStateLabel(turno.state) })
                    : t("testChat.turnHeadingNoNumber", { state: turnStateLabel(turno.state) })}
            </span>
            <p className="poph-chat-teste__evento">{phraseForStateWithoutResponse(turno.state)}</p>
            {turno.error && <p className="poph-chat-teste__erro">{turno.error}</p>}
            {(turno.files_out ?? []).length > 0 && (
                <ArquivosDoAgente arquivos={turno.files_out ?? []} acoes={acoesDeArquivo} />
            )}
            {numero !== null && (
                <div>
                    <SofaButton variant="subtle" onClick={() => aoVerPorQue(turno.id)}>
                        {t("testChat.viewWhy", { number: numero })}
                    </SofaButton>
                </div>
            )}
        </div>
    );
}

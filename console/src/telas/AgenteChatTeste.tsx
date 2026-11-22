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
import { obterAgente, type Agente } from "../api/agentes";
import { ErroApi } from "../api/cliente";
import { obterOrg } from "../api/orgs";
import {
    LIMITE_ARQUIVO_BYTES,
    criarConversaTeste,
    enviarMensagemTeste,
    listarConversasTeste,
    obterHistoricoTeste,
    reabrirConversaTeste,
    type AnexoTeste,
    type ArquivoEnviadoTeste,
    type ConversaTeste,
    type EventoDoTurno,
    type HistoricoTeste,
    type MensagemTeste,
    type TurnoTeste,
} from "../api/testeChat";
import {
    baixarConteudo,
    lerComoDataUrl,
    origemDoAnexoDoCliente,
    origemDoArquivoDoAgente,
    podeVerNaTela,
    rotuloDoEnvio,
    type OrigemDeArquivo,
} from "../api/arquivoTeste";
import { intercalarTurnos, frasePeloEstadoSemResposta, rotuloDoEstadoDoTurno } from "../api/porQue";
import { CabecalhoDoAgente, AbasDoAgente } from "../componentes/AbasDoAgente";
import { PainelPorQue } from "../componentes/PainelPorQue";
import { PreviaArquivoTeste, type PreviaDeImagem } from "../componentes/PreviaArquivoTeste";
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
    return falha instanceof Error ? falha.message : "falha desconhecida";
}

function ehAbortado(falha: unknown): boolean {
    return (
        (falha instanceof DOMException || falha instanceof Error) && falha.name === "AbortError"
    );
}

function dataCurta(ms: number): string {
    return new Date(ms).toLocaleString("pt-BR", {
        day: "2-digit",
        month: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
    });
}

/** Identificador curto para a lista e o cabeçalho (o id completo fica no atributo). */
function idCurto(id: string): string {
    return id.length > 8 ? `…${id.slice(-8)}` : id;
}

function tamanhoLegivel(bytes: number): string {
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

function ehTransferida(conversa: ConversaTeste | undefined | null): boolean {
    return conversa?.status === "transferred";
}

function rotuloDoStatus(status: string): string {
    return status === "transferred" ? "Transferida" : status === "open" ? "Ativa" : status;
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
function campos(evento: EventoDoTurno): Record<string, unknown> {
    return evento.dados && typeof evento.dados === "object"
        ? (evento.dados as Record<string, unknown>)
        : {};
}

export function AgenteChatTeste() {
    const { orgId = "", agentId = "" } = useParams();
    const navegar = useNavigate();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [conversas, setConversas] = useState<ConversaTeste[]>([]);
    const [proximoCursor, setProximoCursor] = useState<string | null>(null);
    const [carregandoLista, setCarregandoLista] = useState(true);
    const [erroLista, setErroLista] = useState<string | null>(null);
    const [criando, setCriando] = useState(false);

    const [ativa, setAtiva] = useState<string | null>(null);
    const [historico, setHistorico] = useState<HistoricoTeste | null>(null);
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
            setAgente(await obterAgente(orgId, agentId));
        } catch (falha) {
            setErroDeCarga(mensagemDe(falha));
        }
    }, [orgId, agentId]);

    /** Lê a primeira página. Mantém conversas de páginas seguintes que já estavam na lista. */
    const atualizarLista = useCallback(async () => {
        const lista = await listarConversasTeste(orgId, agentId);
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
                const lido = await obterHistoricoTeste(orgId, agentId, conversa);
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
        obterOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
    }, [carregarAgente, orgId]);

    useEffect(() => {
        let cancelado = false;
        setCarregandoLista(true);
        setErroLista(null);
        listarConversasTeste(orgId, agentId)
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
            const lista = await listarConversasTeste(orgId, agentId, proximoCursor);
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
            const nova = await criarConversaTeste(orgId, agentId);
            setConversas((atuais) => [nova, ...atuais]);
            abrirConversa(nova.id);
        } catch (falha) {
            setAviso({
                tom: "danger",
                titulo: "Não consegui abrir uma conversa nova",
                texto: mensagemDe(falha),
            });
        } finally {
            setCriando(false);
        }
    }

    /** Aplica um evento do turno ao que a tela mostra. Só chamado para a conversa da tela. */
    function tratarEvento(evento: EventoDoTurno) {
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
                const nome = texto(dados.name) ?? "ferramenta";
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
                        titulo: "O turno falhou",
                        texto:
                            texto(dados.error) ??
                            "A IA não conseguiu responder a esta mensagem. Tente de novo.",
                    });
                } else if (estado === "skipped") {
                    setAviso({
                        tom: "info",
                        titulo: "A IA não respondeu a esta mensagem",
                        texto: "A conversa não recebe resposta automática neste momento.",
                    });
                }
                break;
            }
            case "erro":
                setTurno((atual) =>
                    atual
                        ? {
                              ...atual,
                              erro: texto(dados.message) ?? "o turno não pôde começar",
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
        if (falha instanceof ErroApi) {
            if (falha.status === 409) {
                return {
                    tom: "warn",
                    titulo: "Esta conversa já tem uma resposta em andamento",
                    texto: "Espere a resposta terminar ou feche a outra aba onde ela está sendo escrita. Sua mensagem não foi enviada.",
                };
            }
            if (falha.status === 503) {
                return {
                    tom: "danger",
                    titulo: "O turno não está ligado neste servidor",
                    texto: "Falta a chave do modelo. Sua mensagem não foi gravada.",
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
            titulo: "Não consegui enviar a mensagem",
            texto: mensagemDe(falha),
        };
    }

    async function enviar(evento?: FormEvent<HTMLFormElement>) {
        evento?.preventDefault();
        if (bloqueada || enviando || !historico) return;

        const textoDoCliente = rascunho.trim();
        if (!textoDoCliente && arquivos.length === 0) {
            setErroDeCampo({ campo: "text", mensagem: "Escreva uma mensagem ou anexe um arquivo." });
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
            await enviarMensagemTeste(
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
                        titulo: "Turno cancelado",
                        texto: "A sua mensagem ficou registrada, mas a resposta não foi gerada. Você pode enviar outra.",
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
    async function abrirPrevia(origem: OrigemDeArquivo) {
        if (!ativaRef.current) return;
        const pedido = ++pedidoPreviaRef.current;
        setPrevia({ origem, estado: "carregando", src: null, erro: null });
        try {
            const arquivo = await baixarConteudo(origem.caminho);
            const src = await lerComoDataUrl(arquivo.blob, origem.mime);
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
    async function baixarArquivo(origem: OrigemDeArquivo) {
        const conversa = ativaRef.current;
        if (!conversa) return;
        setBaixandoAnexo(origem.chave);
        setErroDeDownload(null);
        try {
            const arquivo = await baixarConteudo(origem.caminho);
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
            origemDoAnexoDoCliente(orgId, agentId, ativa ?? "", mensagemId, anexo),
        doAgente: (arquivo) => origemDoArquivoDoAgente(orgId, agentId, arquivo),
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
            if (arquivo.size > LIMITE_ARQUIVO_BYTES) {
                setErroDeCampo({
                    campo: "file",
                    mensagem: `${arquivo.name} tem ${tamanhoLegivel(arquivo.size)}: o limite é 16 MB por arquivo.`,
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
            await reabrirConversaTeste(orgId, agentId, conversa, historico.conversation.version);
            if (ativaRef.current === conversa) {
                setAviso({
                    tom: "info",
                    titulo: "Atendimento reaberto",
                    texto: "A IA volta a responder nesta conversa.",
                });
            }
        } catch (falha) {
            if (ativaRef.current === conversa) {
                if (falha instanceof ErroApi && falha.status === 412) {
                    setAviso({
                        tom: "warn",
                        titulo: "A conversa mudou em outro lugar",
                        texto: "Atualizei a versão na tela. Confira a conversa e tente reabrir de novo. O que você digitou continua aqui.",
                    });
                } else {
                    setAviso({
                        tom: "danger",
                        titulo: "Não consegui reabrir a conversa",
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
                    title="Não consegui carregar o chat de teste"
                    description="A instância não respondeu à leitura do agente. Tente de novo; se persistir, confira a saúde do servidor."
                    details={erroDeCarga}
                    retry={{ label: "Tentar de novo", onClick: () => void carregarAgente() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => navegar(`/orgs/${orgId}/agents`)}
                        >
                            Voltar para agentes
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

    const mensagens: MensagemTeste[] = historico?.messages ?? [];
    /** Posição de cada turno na conversa (1 é o primeiro), como o painel e os botões a mostram. */
    const numeroDoTurno = new Map<string, number>(
        (historico?.turns ?? []).map((turnoDaLista, indice): [string, number] => [turnoDaLista.id, indice + 1]),
    );
    /** Turnos sem mensagem de resposta: falha, corte, "não respondeu" ou ainda em andamento. */
    const turnosPorId = new Map<string, TurnoTeste>(
        (historico?.turns ?? []).map((turnoDaLista): [string, TurnoTeste] => [turnoDaLista.id, turnoDaLista]),
    );
    const turnosSemResposta = (historico?.turns ?? []).filter(
        (turnoDaLista) => !mensagens.some((mensagem) => mensagem.turn_id === turnoDaLista.id),
    );
    const itensDoRegistro = intercalarTurnos(mensagens, turnosSemResposta);
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
                descricao="conversas de teste com memória zerada: nada do que você escreve aqui sai para o WhatsApp"
            />
            <AbasDoAgente org={orgId} agente={agentId} ativa="test-chat" nome={agente.name} />

            <div className="poph-chat-teste__corpo">
                <aside className="poph-chat-teste__lista" aria-label="Conversas de teste">
                    <div className="poph-chat-teste__lista-topo">
                        <SofaButton
                            variant="primary"
                            icon={<IconPlus aria-hidden="true" />}
                            onClick={() => void novaConversa()}
                            loading={criando}
                            disabled={enviando}
                            fullWidth
                        >
                            Nova conversa
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
                            Atualizar
                        </SofaButton>
                    </div>

                    {carregandoLista && conversas.length === 0 ? (
                        <SofaSkeleton lines={4} />
                    ) : erroLista && conversas.length === 0 ? (
                        <SofaErrorState
                            title="Não consegui listar as conversas"
                            description="A lista de conversas de teste não respondeu. Tente de novo."
                            details={erroLista}
                            retry={{
                                label: "Tentar de novo",
                                onClick: () => {
                                    setErroLista(null);
                                    void atualizarLista().catch((falha: unknown) =>
                                        setErroLista(mensagemDe(falha)),
                                    );
                                },
                            }}
                        />
                    ) : conversas.length === 0 ? (
                        <p className="poph-chat-teste__vazio">
                            Nenhuma conversa de teste ainda. Crie uma para começar.
                        </p>
                    ) : (
                        <ul className="poph-chat-teste__itens" aria-label="Conversas">
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
                                                {dataCurta(conversa.created_at)} ·{" "}
                                                {conversa.message_count} mensagem(ns)
                                            </span>
                                        </button>
                                    </li>
                                );
                            })}
                        </ul>
                    )}

                    {proximoCursor && (
                        <SofaButton variant="subtle" onClick={() => void carregarMais()}>
                            Carregar mais conversas
                        </SofaButton>
                    )}
                </aside>

                <section className="poph-chat-teste__conversa" aria-label="Conversa de teste">
                    {!ativa ? (
                        <SofaEmptyState
                            icon={<IconMessage aria-hidden="true" />}
                            title="Escolha uma conversa ou crie uma nova"
                            description="Cada conversa de teste começa com a memória zerada, como um cliente novo."
                            action={
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => void novaConversa()}
                                    loading={criando}
                                >
                                    Nova conversa
                                </SofaButton>
                            }
                        />
                    ) : erroHistorico && !historico ? (
                        <SofaErrorState
                            title="Não consegui abrir esta conversa"
                            description="O histórico não respondeu. Tente de novo."
                            details={erroHistorico}
                            retry={{ label: "Tentar de novo", onClick: () => void carregarHistorico(ativa) }}
                        />
                    ) : !historico ? (
                        <SofaSkeleton lines={6} />
                    ) : (
                        <>
                            <div className="poph-chat-teste__cabecalho">
                                <span className="poph-mono-mudo" title={historico.conversation.id}>
                                    Conversa {idCurto(historico.conversation.id)}
                                </span>
                                <span
                                    className={`poph-status ${classeDoStatus(historico.conversation.status)}`}
                                >
                                    <span className="poph-status__ponto" aria-hidden="true" />
                                    {rotuloDoStatus(historico.conversation.status)}
                                </span>
                                {carregandoHistorico && (
                                    <span className="poph-chat-teste__atualizando">Atualizando…</span>
                                )}
                            </div>

                            {transferida && (
                                <div className="poph-chat-teste__transferida">
                                    <SofaBanner
                                        tone="warn"
                                        title="Atendimento transferido para uma pessoa"
                                    >
                                        A IA não responde nesta conversa até alguém reabrir o atendimento.
                                        A mensagem nova fica registrada.
                                    </SofaBanner>
                                    <SofaButton
                                        variant="secondary"
                                        onClick={() => void reabrir()}
                                        loading={reabrindo}
                                        disabled={enviando}
                                    >
                                        Reabrir atendimento da IA nesta conversa
                                    </SofaButton>
                                </div>
                            )}

                            <div
                                ref={registroRef}
                                className="poph-chat-teste__registro"
                                role="log"
                                aria-live="polite"
                                aria-label="Mensagens da conversa de teste"
                            >
                                {mensagens.length === 0 && turnosSemResposta.length === 0 && !turno && (
                                    <p className="poph-chat-teste__vazio">
                                        Esta conversa ainda não tem mensagens. Escreva a primeira abaixo.
                                    </p>
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
                                            <span className="poph-chat-teste__autor">Cliente</span>
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
                                            <span className="poph-chat-teste__autor">Agente</span>
                                            {turno.ferramentas.map((nome, indice) => (
                                                <p key={`${nome}-${indice}`} className="poph-chat-teste__evento">
                                                    Usando a ferramenta {nome}…
                                                </p>
                                            ))}
                                            {turno.fontes !== null && (
                                                <p className="poph-chat-teste__evento">
                                                    Base consultada: {turno.fontes} trecho(s).
                                                </p>
                                            )}
                                            {turno.transferido && (
                                                <p className="poph-chat-teste__evento">
                                                    A conversa foi transferida para uma pessoa.
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
                                                    <p className="poph-chat-teste__evento">Pensando…</p>
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
                                    Turno concluído
                                    {encerramento.tokensEntrada !== null &&
                                        ` · entrada ${encerramento.tokensEntrada} tokens`}
                                    {encerramento.tokensSaida !== null &&
                                        ` · saída ${encerramento.tokensSaida} tokens`}
                                    {encerramento.totalMs !== null &&
                                        ` · ${encerramento.totalMs} ms`}
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
                                aria-label="Enviar mensagem de teste"
                            >
                                <div className="poph-chat-teste__campo">
                                    <label htmlFor="chat-teste-texto" className="poph-chat-teste__rotulo">
                                        Mensagem do cliente
                                    </label>
                                    <textarea
                                        id="chat-teste-texto"
                                        className="poph-chat-teste__texto"
                                        rows={3}
                                        value={rascunho}
                                        disabled={bloqueada || enviando}
                                        placeholder={
                                            transferida
                                                ? "Reabra o atendimento para escrever de novo"
                                                : "Escreva como o cliente escreveria. Enter envia; Shift+Enter quebra a linha."
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
                                        Anexar arquivos
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
                                        Imagem, PDF ou Office, até 16 MB por arquivo.
                                    </span>
                                </div>

                                {arquivos.length > 0 && (
                                    <ul className="poph-chat-teste__anexos-na-fila" aria-label="Arquivos a enviar">
                                        {arquivos.map((arquivo, indice) => (
                                            <li key={`${arquivo.name}-${indice}`}>
                                                <span>
                                                    {arquivo.name} · {tamanhoLegivel(arquivo.size)}
                                                </span>
                                                <button
                                                    type="button"
                                                    className="poph-chat-teste__tirar"
                                                    aria-label={`Tirar ${arquivo.name} da mensagem`}
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
                                            Cancelar turno em andamento
                                        </SofaButton>
                                    ) : (
                                        <SofaButton
                                            type="submit"
                                            variant="primary"
                                            icon={<IconSend aria-hidden="true" />}
                                            disabled={bloqueada}
                                        >
                                            Enviar
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
    ver: (origem: OrigemDeArquivo) => void;
    baixar: (origem: OrigemDeArquivo) => void;
    /** Monta a origem de um anexo que o cliente mandou numa mensagem da conversa. */
    doAnexo: (mensagemId: string, anexo: AnexoTeste) => OrigemDeArquivo;
    /** Monta a origem de um arquivo que o agente enviou (item de `files_out`). */
    doAgente: (arquivo: ArquivoEnviadoTeste) => OrigemDeArquivo;
};

/** Um balão do histórico. `in` é o cliente, `out` é o agente. */
function Balao({
    mensagem,
    numeroDoTurno,
    arquivosDoAgente,
    aoVerPorQue,
    acoesDeArquivo,
}: {
    mensagem: MensagemTeste;
    numeroDoTurno: number | null;
    /** Arquivos que o agente enviou no turno desta resposta (`files_out`). */
    arquivosDoAgente: ArquivoEnviadoTeste[];
    aoVerPorQue: (turnoId: string) => void;
    acoesDeArquivo: AcoesDeArquivo;
}) {
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
            <span className="poph-chat-teste__autor">{doCliente ? "Cliente" : "Agente"}</span>
            {mensagem.text && <p className="poph-chat-teste__texto-agente">{mensagem.text}</p>}
            {mensagem.attachments.length > 0 && (
                <ul className="poph-chat-teste__anexos">
                    {mensagem.attachments.map((anexo) => (
                        <li key={anexo.id}>
                            {anexo.name} · {tamanhoLegivel(anexo.size_bytes)}
                            {anexo.has_text ? " · com texto extraído" : ""}
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
                        Ver por quê (turno {numeroDoTurno})
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
function AcoesDoArquivo({ origem, acoes }: { origem: OrigemDeArquivo; acoes: AcoesDeArquivo }) {
    const baixandoEste = acoes.baixando === origem.chave;
    const falhou = acoes.erro?.chave === origem.chave ? acoes.erro.texto : null;
    return (
        <span className="poph-chat-teste__anexo-acoes">
            {podeVerNaTela(origem.mime) && (
                <SofaButton
                    variant="subtle"
                    aria-label={`Ver ${origem.nome}`}
                    onClick={() => acoes.ver(origem)}
                >
                    Ver
                </SofaButton>
            )}
            <SofaButton
                variant="subtle"
                aria-label={`Baixar ${origem.nome}`}
                loading={baixandoEste}
                disabled={acoes.baixando !== null && !baixandoEste}
                onClick={() => acoes.baixar(origem)}
            >
                Baixar
            </SofaButton>
            {falhou && (
                <span className="poph-chat-teste__anexo-erro" role="alert">
                    <span className="poph-chat-teste__erro">{falhou}</span>
                    <SofaButton
                        variant="secondary"
                        aria-label={`Tentar de novo: baixar ${origem.nome}`}
                        onClick={() => acoes.baixar(origem)}
                    >
                        Tentar de novo
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
    arquivos: ArquivoEnviadoTeste[];
    acoes: AcoesDeArquivo;
}) {
    return (
        <ul className="poph-chat-teste__anexos" aria-label="Arquivos enviados pelo agente">
            {arquivos.map((arquivo) => (
                <li key={arquivo.id}>
                    {arquivo.name} · {tamanhoLegivel(arquivo.size_bytes)} ·{" "}
                    <span className={arquivo.state === "failed" ? "poph-chat-teste__erro" : undefined}>
                        {rotuloDoEnvio(arquivo.state)}
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
    turno: TurnoTeste;
    numero: number | null;
    aoVerPorQue: (turnoId: string) => void;
    acoesDeArquivo: AcoesDeArquivo;
}) {
    return (
        <div className="poph-chat-teste__balao poph-chat-teste__balao--agente poph-chat-teste__sem-resposta">
            <span className="poph-chat-teste__autor">
                Turno {numero !== null ? `#${numero}` : ""} · {rotuloDoEstadoDoTurno(turno.state)}
            </span>
            <p className="poph-chat-teste__evento">{frasePeloEstadoSemResposta(turno.state)}</p>
            {turno.error && <p className="poph-chat-teste__erro">{turno.error}</p>}
            {(turno.files_out ?? []).length > 0 && (
                <ArquivosDoAgente arquivos={turno.files_out ?? []} acoes={acoesDeArquivo} />
            )}
            {numero !== null && (
                <div>
                    <SofaButton variant="subtle" onClick={() => aoVerPorQue(turno.id)}>
                        Ver por quê (turno {numero})
                    </SofaButton>
                </div>
            )}
        </div>
    );
}


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
import { HeaderAgent, TabsAgent } from "../components/TabsAgent";
import { PanelWhy } from "../components/PanelWhy";
import { PreviewFileTest, type PreviewImage } from "../components/PreviewFileTest";
import { useI18n, translate } from "../i18n";
import { formatDateTime, formatNumber } from "../i18n/format";
import "../styles/chat-test.css";

const ACCEPTED = ".jpg,.jpeg,.png,.gif,.webp,.pdf,.doc,.docx,.xls,.xlsx,.ppt,.pptx";

type Notice = {
    tom: "info" | "warn" | "danger";
    titulo: string;
    texto: string;
} | null;

type TurnInProgress = {
    textClient: string;
    attachmentsClient: string[];
    response: string;
    ferramentas: string[];
    fontes: number | null;
    transferred: boolean;
    erro: string | null;
};

type Closing = {
    estado: string;
    tokensInput: number | null;
    tokensOutput: number | null;
    totalMs: number | null;
} | null;

type ErrorField = { campo: "text" | "file"; mensagem: string } | null;

function message(failure: unknown): string {
    return failure instanceof Error ? failure.message : translate("testChat.fallbackUnknown");
}

function ehAbortado(failure: unknown): boolean {
    return (
        (failure instanceof DOMException || failure instanceof Error) && failure.name === "AbortError"
    );
}

function shortDate(ms: number): string {
    return formatDateTime(ms);
}

function shortId(id: string): string {
    return id.length > 8 ? `…${id.slice(-8)}` : id;
}

function readableSize(bytes: number): string {
    if (bytes >= 1024 * 1024) {
        return `${formatNumber(bytes / (1024 * 1024), { minimumFractionDigits: 1, maximumFractionDigits: 1 })} MB`;
    }
    return `${formatNumber(Math.max(1, Math.round(bytes / 1024)))} KB`;
}

function isTransferred(conversa: TestConversation | undefined | null): boolean {
    return conversa?.status === "transferred";
}

function labelStatus(status: string): string {
    return status === "transferred"
        ? translate("testChat.statusTransferred")
        : status === "open"
          ? translate("testChat.statusActive")
          : status;
}

function classStatus(status: string): string {
    return status === "transferred" ? "poph-status--attention" : "poph-status--active";
}

function texto(value: unknown): string | null {
    return typeof value === "string" && value ? value : null;
}

function numero(value: unknown): number | null {
    return typeof value === "number" ? value : null;
}

function campos(evento: TurnEvent): Record<string, unknown> {
    return evento.dados && typeof evento.dados === "object"
        ? (evento.dados as Record<string, unknown>)
        : {};
}

export function AgentChatTest() {
    const { orgId = "", agentId = "" } = useParams();
    const navigate = useNavigate();
    const { translate: t } = useI18n();

    const [agente, setAgent] = useState<Agente | null>(null);
    const [nameOrg, setOrgName] = useState<string | null>(null);
    const [errorLoad, setLoadError] = useState<string | null>(null);

    const [conversas, setConversations] = useState<TestConversation[]>([]);
    const [nextCursor, setNextCursor] = useState<string | null>(null);
    const [loadingList, setLoadingList] = useState(true);
    const [errorList, setListError] = useState<string | null>(null);
    const [creating, setCreating] = useState(false);

    const [active, setActive] = useState<string | null>(null);
    const [transcript, setHistory] = useState<TestHistory | null>(null);
    const [loadingHistory, setLoadingHistory] = useState(false);
    const [errorHistory, setHistoryError] = useState<string | null>(null);

    const [draft, setDraft] = useState("");
    const [files, setFiles] = useState<File[]>([]);
    const [errorField, setFieldError] = useState<ErrorField>(null);
    const [sending, setSending] = useState(false);
    const [turno, setTurn] = useState<TurnInProgress | null>(null);
    const [closing, setClosing] = useState<Closing>(null);
    const [notice, setNotice] = useState<Notice>(null);
    const [reopening, setReopening] = useState(false);
    const [turnOpened, setTurnOpened] = useState<string | null>(null);
    const [preview, setPreview] = useState<PreviewImage | null>(null);
    const [downloadingAttachment, setDownloadingAttachment] = useState<string | null>(null);
    const [downloadError, setDownloadError] = useState<{ chave: string; texto: string } | null>(
        null,
    );

    const activeRef = useRef<string | null>(null);
    const controlRef = useRef<AbortController | null>(null);
    const recordRef = useRef<HTMLDivElement | null>(null);
    const orderPreviewRef = useRef(0);
    const downloadUrlsRef = useRef<Set<string>>(new Set());

    const transferred = isTransferred(transcript?.conversation);
    const bloqueada = transferred || !transcript;

    const loadAgent = useCallback(async () => {
        setLoadError(null);
        try {
            setAgent(await getAgent(orgId, agentId));
        } catch (failure) {
            setLoadError(message(failure));
        }
    }, [orgId, agentId]);

    const updateList = useCallback(async () => {
        const list = await listTestConversations(orgId, agentId);
        setConversations((current) => {
            const freshItems = new Set(list.items.map((item) => item.id));
            return [...list.items, ...current.filter((item) => !freshItems.has(item.id))];
        });
        setNextCursor(list.next_cursor);
        return list;
    }, [orgId, agentId]);

    const loadHistory = useCallback(
        async (conversa: string) => {
            setLoadingHistory(true);
            setHistoryError(null);
            try {
                const read = await getTestHistory(orgId, agentId, conversa);
                if (activeRef.current !== conversa) return;
                setHistory(read);
                setConversations((current) =>
                    current.map((item) => (item.id === read.conversation.id ? read.conversation : item)),
                );
            } catch (failure) {
                if (activeRef.current === conversa) setHistoryError(message(failure));
            } finally {
                if (activeRef.current === conversa) setLoadingHistory(false);
            }
        },
        [orgId, agentId],
    );

    const releaseUrl = useCallback((url: string) => {
        if (downloadUrlsRef.current.delete(url)) URL.revokeObjectURL(url);
    }, []);

    const releaseDownloads = useCallback(() => {
        for (const url of [...downloadUrlsRef.current]) releaseUrl(url);
    }, [releaseUrl]);

    const openConversation = useCallback(
        (id: string) => {
            controlRef.current?.abort();
            controlRef.current = null;
            activeRef.current = id;
            setActive(id);
            setHistory(null);
            setHistoryError(null);
            setTurn(null);
            setClosing(null);
            setNotice(null);
            setSending(false);
            setFieldError(null);
            setTurnOpened(null);
            setDraft("");
            setFiles([]);
            orderPreviewRef.current += 1;
            setPreview(null);
            setDownloadingAttachment(null);
            setDownloadError(null);
            releaseDownloads();
            void loadHistory(id);
        },
        [loadHistory, releaseDownloads],
    );

    useEffect(() => {
        return () => {
            orderPreviewRef.current += 1;
            releaseDownloads();
        };
    }, [releaseDownloads]);

    useEffect(() => {
        void loadAgent();
        getOrg(orgId)
            .then((org) => setOrgName(org.name))
            .catch(() => setOrgName(null));
    }, [loadAgent, orgId]);

    useEffect(() => {
        let cancelled = false;
        setLoadingList(true);
        setListError(null);
        listTestConversations(orgId, agentId)
            .then((list) => {
                if (cancelled) return;
                setConversations(list.items);
                setNextCursor(list.next_cursor);
                const first = list.items[0];
                if (first) openConversation(first.id);
            })
            .catch((failure: unknown) => {
                if (!cancelled) setListError(message(failure));
            })
            .finally(() => {
                if (!cancelled) setLoadingList(false);
            });
        return () => {
            cancelled = true;
            controlRef.current?.abort();
        };
    }, [orgId, agentId, openConversation]);

    useEffect(() => {
        const registro = recordRef.current;
        if (registro) registro.scrollTop = registro.scrollHeight;
    }, [transcript, turno]);

    async function loadMore() {
        if (!nextCursor) return;
        setListError(null);
        try {
            const list = await listTestConversations(orgId, agentId, nextCursor);
            setConversations((current) => [...current, ...list.items]);
            setNextCursor(list.next_cursor);
        } catch (failure) {
            setListError(message(failure));
        }
    }

    async function newConversation() {
        setCreating(true);
        setNotice(null);
        try {
            const fresh = await createTestConversation(orgId, agentId);
            setConversations((current) => [fresh, ...current]);
            openConversation(fresh.id);
        } catch (failure) {
            setNotice({
                tom: "danger",
                titulo: t("testChat.newConversationError"),
                texto: message(failure),
            });
        } finally {
            setCreating(false);
        }
    }

    function handleEvent(evento: TurnEvent) {
        const dados = campos(evento);
        switch (evento.tipo) {
            case "texto": {
                const delta = texto(dados.delta) ?? "";
                setTurn((current) => (current ? { ...current, response: current.response + delta } : current));
                break;
            }
            case "base": {
                const sources = Array.isArray(dados.sources) ? dados.sources.length : 0;
                setTurn((current) => (current ? { ...current, fontes: sources } : current));
                break;
            }
            case "ferramenta": {
                const nome = texto(dados.name) ?? t("testChat.toolFallback");
                setTurn((current) =>
                    current ? { ...current, ferramentas: [...current.ferramentas, nome] } : current,
                );
                break;
            }
            case "transferencia":
                setTurn((current) => (current ? { ...current, transferred: true } : current));
                break;
            case "fim": {
                const timings = (dados.timings ?? {}) as Record<string, unknown>;
                const estado = texto(dados.state) ?? "desconhecido";
                setClosing({
                    estado,
                    tokensInput: numero(dados.input_tokens),
                    tokensOutput: numero(dados.output_tokens),
                    totalMs: numero(timings.total_ms),
                });
                const final = texto(dados.text);
                if (final) {
                    setTurn((current) => (current ? { ...current, response: final } : current));
                }
                if (estado === "failed") {
                    setNotice({
                        tom: "danger",
                        titulo: t("testChat.turnFailedTitle"),
                        texto: texto(dados.error) ?? t("testChat.turnFailedText"),
                    });
                } else if (estado === "skipped") {
                    setNotice({
                        tom: "info",
                        titulo: t("testChat.skippedTitle"),
                        texto: t("testChat.skippedText"),
                    });
                }
                break;
            }
            case "erro":
                setTurn((current) =>
                    current
                        ? {
                              ...current,
                              erro: texto(dados.message) ?? t("testChat.turnStartError"),
                          }
                        : current,
                );
                break;
            default:
                break;
        }
    }

    function handleFailureSend(failure: unknown): Notice {
        if (failure instanceof ApiError) {
            if (failure.status === 409) {
                return {
                    tom: "warn",
                    titulo: t("testChat.busyTitle"),
                    texto: t("testChat.busyText"),
                };
            }
            if (failure.status === 503) {
                return {
                    tom: "danger",
                    titulo: t("testChat.noTurnTitle"),
                    texto: t("testChat.noTurnText"),
                };
            }
            if (failure.status === 400) {
                const details = (failure.details ?? {}) as Record<string, unknown>;
                const campo = details.field === "file" ? "file" : "text";
                setFieldError({ campo, mensagem: failure.message });
                return null;
            }
        }
        return {
            tom: "danger",
            titulo: t("testChat.sendErrorTitle"),
            texto: message(failure),
        };
    }

    async function send(evento?: FormEvent<HTMLFormElement>) {
        evento?.preventDefault();
        if (bloqueada || sending || !transcript) return;

        const textClient = draft.trim();
        if (!textClient && files.length === 0) {
            setFieldError({ campo: "text", mensagem: t("testChat.invalidMessage") });
            return;
        }

        const conversa = transcript.conversation.id;
        const sent = files;
        setFieldError(null);
        setNotice(null);
        setClosing(null);
        setDraft("");
        setFiles([]);

        const control = new AbortController();
        controlRef.current = control;
        setSending(true);
        setTurn({
            textClient,
            attachmentsClient: sent.map((file) => file.name),
            response: "",
            ferramentas: [],
            fontes: null,
            transferred: false,
            erro: null,
        });

        let started = false;
        try {
            await sendTestMessage(
                orgId,
                agentId,
                conversa,
                { texto: textClient, files: sent },
                {
                    sinal: control.signal,
                    onEvent: (evento) => {
                        if (activeRef.current !== conversa) return;
                        started = true;
                        handleEvent(evento);
                    },
                },
            );
        } catch (failure) {
            if (activeRef.current === conversa) {
                if (ehAbortado(failure)) {
                    setNotice({
                        tom: "info",
                        titulo: t("testChat.cancelTitle"),
                        texto: t("testChat.cancelText"),
                    });
                } else {
                    const noticeFailure = handleFailureSend(failure);
                    if (noticeFailure) setNotice(noticeFailure);
                    if (!started) {
                        setDraft(textClient);
                        setFiles(sent);
                    }
                }
            }
        } finally {
            if (activeRef.current === conversa) {
                await loadHistory(conversa);
                void updateList().catch(() => undefined);
            }
            if (controlRef.current === control) {
                controlRef.current = null;
                setSending(false);
                setTurn(null);
            }
        }
    }

    async function openPreview(source: FileOrigin) {
        if (!activeRef.current) return;
        const order = ++orderPreviewRef.current;
        setPreview({ origem: source, estado: "carregando", src: null, erro: null });
        try {
            const file = await downloadContent(source.path);
            const src = await readAsDataUrl(file.blob, source.mime);
            if (order !== orderPreviewRef.current) return;
            setPreview({ origem: source, estado: "pronta", src, erro: null });
        } catch (failure) {
            if (order !== orderPreviewRef.current) return;
            setPreview({ origem: source, estado: "erro", src: null, erro: message(failure) });
        }
    }

    function closePreview() {
        orderPreviewRef.current += 1;
        setPreview(null);
    }

    async function downloadFile(source: FileOrigin) {
        const conversa = activeRef.current;
        if (!conversa) return;
        setDownloadingAttachment(source.chave);
        setDownloadError(null);
        try {
            const file = await downloadContent(source.path);
            if (activeRef.current !== conversa) return;
            const url = URL.createObjectURL(file.blob);
            downloadUrlsRef.current.add(url);
            const link = document.createElement("a");
            link.href = url;
            link.download = file.nome ?? source.nome;
            document.body.appendChild(link);
            link.click();
            link.remove();
            window.setTimeout(() => releaseUrl(url), 60_000);
        } catch (failure) {
            if (activeRef.current !== conversa) return;
            setDownloadError({ chave: source.chave, texto: message(failure) });
        } finally {
            if (activeRef.current === conversa) setDownloadingAttachment(null);
        }
    }

    const actionsFile: ActionsFile = {
        downloading: downloadingAttachment,
        erro: downloadError,
        view: (source) => void openPreview(source),
        download: (source) => void downloadFile(source),
        attachment: (messageId, attachment) =>
            clientAttachmentOrigin(orgId, agentId, active ?? "", messageId, attachment),
        agent: (file) => agentFileOrigin(orgId, agentId, file),
    };

    function cancelTurn() {
        controlRef.current?.abort();
    }

    function openWhy(turnId: string) {
        setTurnOpened(turnId);
        if (activeRef.current) void loadHistory(activeRef.current);
    }

    function chooseFiles(evento: ChangeEvent<HTMLInputElement>) {
        const chosen = Array.from(evento.target.files ?? []);
        evento.target.value = "";
        const accepted: File[] = [];
        for (const file of chosen) {
            if (file.size > LIMIT_FILE_BYTES) {
                setFieldError({
                    campo: "file",
                    mensagem: t("testChat.fileTooLarge", {
                        name: file.name,
                        size: readableSize(file.size),
                    }),
                });
                continue;
            }
            accepted.push(file);
        }
        if (accepted.length > 0) {
            setFieldError(null);
            setFiles((current) => [...current, ...accepted]);
        }
    }

    function removeFile(index: number) {
        setFiles((current) => current.filter((_, position) => position !== index));
    }

    function keyText(evento: KeyboardEvent<HTMLTextAreaElement>) {
        if (evento.key === "Enter" && !evento.shiftKey && !evento.nativeEvent.isComposing) {
            evento.preventDefault();
            void send();
        }
    }

    async function reopen() {
        if (!transcript || reopening || sending) return;
        const conversa = transcript.conversation.id;
        setReopening(true);
        setNotice(null);
        try {
            await reopenTestConversation(orgId, agentId, conversa, transcript.conversation.version);
            if (activeRef.current === conversa) {
                setNotice({
                    tom: "info",
                    titulo: t("testChat.reopenedTitle"),
                    texto: t("testChat.reopenedText"),
                });
            }
        } catch (failure) {
            if (activeRef.current === conversa) {
                if (failure instanceof ApiError && failure.status === 412) {
                    setNotice({
                        tom: "warn",
                        titulo: t("testChat.conflictTitle"),
                        texto: t("testChat.conflictText"),
                    });
                } else {
                    setNotice({
                        tom: "danger",
                        titulo: t("testChat.reopenErrorTitle"),
                        texto: message(failure),
                    });
                }
            }
        } finally {
            setReopening(false);
            if (activeRef.current === conversa) {
                await loadHistory(conversa);
                void updateList().catch(() => undefined);
            }
        }
    }

    if (!agente && errorLoad) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("testChat.pageLoadTitle")}
                    description={t("testChat.pageLoadDescription")}
                    details={errorLoad}
                    retry={{ label: t("testChat.retry"), onClick: () => void loadAgent() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => navigate(`/orgs/${orgId}/agents`)}
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
            <div className="poph-page" aria-busy="true">
                <SofaSkeleton width={260} height={26} />
                <SofaSkeleton lines={6} />
            </div>
        );
    }

    const messages: TestMessage[] = transcript?.messages ?? [];
    const numberTurn = new Map<string, number>(
        (transcript?.turns ?? []).map((turnList, index): [string, number] => [turnList.id, index + 1]),
    );
    const turnsById = new Map<string, TestTurn>(
        (transcript?.turns ?? []).map((turnList): [string, TestTurn] => [turnList.id, turnList]),
    );
    const turnsNoResponse = (transcript?.turns ?? []).filter(
        (turnList) => !messages.some((mensagem) => mensagem.turn_id === turnList.id),
    );
    const recordItems = interleaveTurns(messages, turnsNoResponse);
    const turnPanel = turnOpened
        ? (transcript?.turns ?? []).find((turnList) => turnList.id === turnOpened) ?? null
        : null;

    return (
        <div className="poph-page poph-chat-test">
            <HeaderAgent
                org={orgId}
                nome={agente.name}
                slug={agente.slug}
                situacao={agente.status}
                organization={nameOrg}
                descricao={t("testChat.chatDescription")}
            />
            <TabsAgent org={orgId} agente={agentId} active="test-chat" nome={agente.name} model={agente.model ?? null} />

            <div className="poph-chat-test__body">
                <aside className="poph-chat-test__list" aria-label={t("testChat.conversationsLabel")}>
                    <div className="poph-chat-test__list-top">
                        <SofaButton
                            variant="primary"
                            icon={<IconPlus aria-hidden="true" />}
                            onClick={() => void newConversation()}
                            loading={creating}
                            disabled={sending}
                            fullWidth
                        >
                            {t("testChat.newConversation")}
                        </SofaButton>
                        <SofaButton
                            variant="subtle"
                            icon={<IconRefresh aria-hidden="true" />}
                            onClick={() => {
                                setListError(null);
                                void updateList().catch((failure: unknown) =>
                                    setListError(message(failure)),
                                );
                            }}
                        >
                            {t("testChat.refresh")}
                        </SofaButton>
                    </div>

                    {loadingList && conversas.length === 0 ? (
                        <SofaSkeleton lines={4} />
                    ) : errorList && conversas.length === 0 ? (
                        <SofaErrorState
                            title={t("testChat.listErrorTitle")}
                            description={t("testChat.listErrorDescription")}
                            details={errorList}
                            retry={{
                                label: t("testChat.retry"),
                                onClick: () => {
                                    setListError(null);
                                    void updateList().catch((failure: unknown) =>
                                        setListError(message(failure)),
                                    );
                                },
                            }}
                        />
                    ) : conversas.length === 0 ? (
                        <p className="poph-chat-test__empty">{t("testChat.listEmpty")}</p>
                    ) : (
                        <ul className="poph-chat-test__items" aria-label={t("testChat.conversationsListLabel")}>
                            {conversas.map((conversa) => {
                                const selected = conversa.id === active;
                                return (
                                    <li key={conversa.id}>
                                        <button
                                            type="button"
                                            className={
                                                selected
                                                    ? "poph-chat-test__item poph-chat-test__item--active"
                                                    : "poph-chat-test__item"
                                            }
                                            aria-current={selected ? "true" : undefined}
                                            title={conversa.id}
                                            onClick={() => openConversation(conversa.id)}
                                        >
                                            <span className="poph-chat-test__item-line">
                                                <span className="poph-mono-muted">
                                                    {shortId(conversa.id)}
                                                </span>
                                                <span
                                                    className={`poph-status ${classStatus(conversa.status)}`}
                                                >
                                                    <span className="poph-status__point" aria-hidden="true" />
                                                    {labelStatus(conversa.status)}
                                                </span>
                                            </span>
                                            <span className="poph-chat-test__item-meta">
                                                {t("testChat.conversationMeta", {
                                                    date: shortDate(conversa.created_at),
                                                    count: formatNumber(conversa.message_count),
                                                })}
                                            </span>
                                        </button>
                                    </li>
                                );
                            })}
                        </ul>
                    )}

                    {nextCursor && (
                        <SofaButton variant="subtle" onClick={() => void loadMore()}>
                            {t("testChat.loadMore")}
                        </SofaButton>
                    )}
                </aside>

                <section className="poph-chat-test__conversation" aria-label={t("testChat.conversationSectionLabel")}>
                    {!active ? (
                        <SofaEmptyState
                            icon={<IconMessage aria-hidden="true" />}
                            title={t("testChat.chooseTitle")}
                            description={t("testChat.chooseDescription")}
                            action={
                                <SofaButton
                                    variant="primary"
                                    icon={<IconPlus aria-hidden="true" />}
                                    onClick={() => void newConversation()}
                                    loading={creating}
                                >
                                    {t("testChat.newConversation")}
                                </SofaButton>
                            }
                        />
                    ) : errorHistory && !transcript ? (
                        <SofaErrorState
                            title={t("testChat.openErrorTitle")}
                            description={t("testChat.openErrorDescription")}
                            details={errorHistory}
                            retry={{ label: t("testChat.retry"), onClick: () => void loadHistory(active) }}
                        />
                    ) : !transcript ? (
                        <SofaSkeleton lines={6} />
                    ) : (
                        <>
                            <div className="poph-chat-test__header">
                                <span className="poph-mono-muted" title={transcript.conversation.id}>
                                    {t("testChat.conversationLabel", { id: shortId(transcript.conversation.id) })}
                                </span>
                                <span
                                    className={`poph-status ${classStatus(transcript.conversation.status)}`}
                                >
                                    <span className="poph-status__point" aria-hidden="true" />
                                    {labelStatus(transcript.conversation.status)}
                                </span>
                                {loadingHistory && (
                                    <span className="poph-chat-test__updating">{t("testChat.updating")}</span>
                                )}
                            </div>

                            {transferred && (
                                <div className="poph-chat-test__transferred">
                                    <SofaBanner tone="warn" title={t("testChat.transferredTitle")}>
                                        {t("testChat.transferredDescription")}
                                    </SofaBanner>
                                    <SofaButton
                                        variant="secondary"
                                        onClick={() => void reopen()}
                                        loading={reopening}
                                        disabled={sending}
                                    >
                                        {t("testChat.reopen")}
                                    </SofaButton>
                                </div>
                            )}

                            <div
                                ref={recordRef}
                                className="poph-chat-test__record"
                                role="log"
                                aria-live="polite"
                                aria-label={t("testChat.logLabel")}
                            >
                                {messages.length === 0 && turnsNoResponse.length === 0 && !turno && (
                                    <p className="poph-chat-test__empty">{t("testChat.noMessages")}</p>
                                )}
                                {recordItems.map((item) =>
                                    item.tipo === "mensagem" ? (
                                        <Balao
                                            key={item.mensagem.id}
                                            mensagem={item.mensagem}
                                            numberTurn={
                                                item.mensagem.turn_id
                                                    ? (numberTurn.get(item.mensagem.turn_id) ?? null)
                                                    : null
                                            }
                                            filesAgent={
                                                item.mensagem.turn_id
                                                    ? (turnsById.get(item.mensagem.turn_id)?.files_out ?? [])
                                                    : []
                                            }
                                            onView={openWhy}
                                            actionsFile={actionsFile}
                                        />
                                    ) : (
                                        <TurnNoResponse
                                            key={`turno-${item.turno.id}`}
                                            turno={item.turno}
                                            numero={numberTurn.get(item.turno.id) ?? null}
                                            onView={openWhy}
                                            actionsFile={actionsFile}
                                        />
                                    ),
                                )}

                                {turno && (
                                    <>
                                        <div className="poph-chat-test__bubble poph-chat-test__bubble--client">
                                            <span className="poph-chat-test__author">{t("testChat.client")}</span>
                                            {turno.textClient && <p>{turno.textClient}</p>}
                                            {turno.attachmentsClient.length > 0 && (
                                                <ul className="poph-chat-test__attachments">
                                                    {turno.attachmentsClient.map((nome, index) => (
                                                        <li key={`${nome}-${index}`}>{nome}</li>
                                                    ))}
                                                </ul>
                                            )}
                                        </div>
                                        <div
                                            className="poph-chat-test__bubble poph-chat-test__bubble--agent"
                                            aria-busy={turno.response === "" && !turno.erro}
                                        >
                                            <span className="poph-chat-test__author">{t("testChat.agent")}</span>
                                            {turno.ferramentas.map((nome, index) => (
                                                <p key={`${nome}-${index}`} className="poph-chat-test__event">
                                                    {t("testChat.toolRunning", { name: nome })}
                                                </p>
                                            ))}
                                            {turno.fontes !== null && (
                                                <p className="poph-chat-test__event">
                                                    {turno.fontes === 1
                                                        ? t("testChat.sourcesOne")
                                                        : t("testChat.sourcesMany", { count: formatNumber(turno.fontes) })}
                                                </p>
                                            )}
                                            {turno.transferred && (
                                                <p className="poph-chat-test__event">
                                                    {t("testChat.conversationTransferred")}
                                                </p>
                                            )}
                                            {turno.response ? (
                                                <p className="poph-chat-test__text-agent">
                                                    {turno.response}
                                                    {sending && !closing && (
                                                        <span aria-hidden="true" className="poph-chat-test__cursor">
                                                            ▌
                                                        </span>
                                                    )}
                                                </p>
                                            ) : (
                                                !turno.erro && (
                                                    <p className="poph-chat-test__event">{t("testChat.thinking")}</p>
                                                )
                                            )}
                                            {turno.erro && (
                                                <p className="poph-chat-test__error">{turno.erro}</p>
                                            )}
                                        </div>
                                    </>
                                )}
                            </div>

                            {closing?.estado === "done" && !turno && (
                                <p className="poph-chat-test__end">
                                    {t("testChat.turnDone")}
                                    {closing.tokensInput !== null &&
                                        t("testChat.turnDoneInput", {
                                            value: formatNumber(closing.tokensInput),
                                        })}
                                    {closing.tokensOutput !== null &&
                                        t("testChat.turnDoneOutput", {
                                            value: formatNumber(closing.tokensOutput),
                                        })}
                                    {closing.totalMs !== null &&
                                        t("testChat.turnDoneTime", {
                                            value: formatNumber(closing.totalMs),
                                        })}
                                </p>
                            )}

                            {notice && (
                                <div className="poph-chat-test__notice" role="status">
                                    <SofaBanner tone={notice.tom} title={notice.titulo}>
                                        {notice.texto}
                                    </SofaBanner>
                                </div>
                            )}

                            <PreviewFileTest
                                preview={preview}
                                onClose={closePreview}
                                onTryNew={() => {
                                    if (preview) void openPreview(preview.origem);
                                }}
                            />

                            <PanelWhy
                                opened={turnOpened !== null}
                                numero={turnPanel ? (numberTurn.get(turnPanel.id) ?? null) : null}
                                turno={turnPanel}
                                updating={loadingHistory}
                                errorRead={turnOpened !== null ? errorHistory : null}
                                onClose={() => setTurnOpened(null)}
                                onTryNew={() => {
                                    if (activeRef.current) void loadHistory(activeRef.current);
                                }}
                            />

                            <form
                                className="poph-chat-test__form"
                                onSubmit={(evento) => void send(evento)}
                                aria-label={t("testChat.formLabel")}
                            >
                                <div className="poph-chat-test__field">
                                    <label htmlFor="chat-teste-texto" className="poph-chat-test__label">
                                        {t("testChat.messageLabel")}
                                    </label>
                                    <textarea
                                        id="chat-teste-texto"
                                        className="poph-chat-test__text"
                                        rows={3}
                                        value={draft}
                                        disabled={bloqueada || sending}
                                        placeholder={
                                            transferred
                                                ? t("testChat.placeholderTransferred")
                                                : t("testChat.placeholder")
                                        }
                                        aria-invalid={errorField?.campo === "text" || undefined}
                                        aria-describedby={
                                            errorField?.campo === "text" ? "chat-teste-erro-texto" : undefined
                                        }
                                        onChange={(evento) => setDraft(evento.target.value)}
                                        onKeyDown={keyText}
                                    />
                                    {errorField?.campo === "text" && (
                                        <p id="chat-teste-erro-texto" className="poph-chat-test__error">
                                            {errorField.mensagem}
                                        </p>
                                    )}
                                </div>

                                <div className="poph-chat-test__attach">
                                    <label
                                        htmlFor="chat-teste-arquivos"
                                        className="poph-chat-test__button-attach"
                                    >
                                        <IconUpload aria-hidden="true" size={14} />
                                        {t("testChat.attach")}
                                    </label>
                                    <input
                                        id="chat-teste-arquivos"
                                        type="file"
                                        multiple
                                        accept={ACCEPTED}
                                        className="poph-chat-test__file"
                                        disabled={bloqueada || sending}
                                        aria-describedby="chat-teste-ajuda-arquivos"
                                        onChange={chooseFiles}
                                    />
                                    <span id="chat-teste-ajuda-arquivos" className="poph-chat-test__help">
                                        {t("testChat.attachHelp")}
                                    </span>
                                </div>

                                {files.length > 0 && (
                                    <ul className="poph-chat-test__attachments-queued" aria-label={t("testChat.filesToSendLabel")}>
                                        {files.map((file, index) => (
                                            <li key={`${file.name}-${index}`}>
                                                <span>
                                                    {file.name} · {readableSize(file.size)}
                                                </span>
                                                <button
                                                    type="button"
                                                    className="poph-chat-test__remove"
                                                    aria-label={t("testChat.removeFileLabel", { name: file.name })}
                                                    disabled={sending}
                                                    onClick={() => removeFile(index)}
                                                >
                                                    <IconXCircle aria-hidden="true" size={14} />
                                                </button>
                                            </li>
                                        ))}
                                    </ul>
                                )}

                                {errorField?.campo === "file" && (
                                    <p className="poph-chat-test__error" role="alert">
                                        {errorField.mensagem}
                                    </p>
                                )}

                                <div className="poph-chat-test__actions">
                                    {sending ? (
                                        <SofaButton variant="danger" onClick={cancelTurn}>
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

type ActionsFile = {
    downloading: string | null;
    erro: { chave: string; texto: string } | null;
    view: (source: FileOrigin) => void;
    download: (source: FileOrigin) => void;
    attachment: (messageId: string, attachment: TestAttachment) => FileOrigin;
    agent: (file: TestSentFile) => FileOrigin;
};

function Balao({
    mensagem,
    numberTurn,
    filesAgent,
    onView,
    actionsFile,
}: {
    mensagem: TestMessage;
    numberTurn: number | null;
    filesAgent: TestSentFile[];
    onView: (turnId: string) => void;
    actionsFile: ActionsFile;
}) {
    const { translate: t } = useI18n();
    const client = mensagem.direction === "in";
    const turnResponse = mensagem.turn_id;
    return (
        <div
            className={
                client
                    ? "poph-chat-test__bubble poph-chat-test__bubble--client"
                    : "poph-chat-test__bubble poph-chat-test__bubble--agent"
            }
        >
            <span className="poph-chat-test__author">{client ? t("testChat.client") : t("testChat.agent")}</span>
            {mensagem.text && <p className="poph-chat-test__text-agent">{mensagem.text}</p>}
            {mensagem.attachments.length > 0 && (
                <ul className="poph-chat-test__attachments">
                    {mensagem.attachments.map((attachment) => (
                        <li key={attachment.id}>
                            {attachment.name} · {readableSize(attachment.size_bytes)}
                            {attachment.has_text ? t("testChat.extractedText") : ""}
                            {}
                            {client && (
                                <FileActions
                                    origin={actionsFile.attachment(mensagem.id, attachment)}
                                    actions={actionsFile}
                                />
                            )}
                        </li>
                    ))}
                </ul>
            )}
            {!client && filesAgent.length > 0 && (
                <FilesAgent files={filesAgent} actions={actionsFile} />
            )}
            {!client && turnResponse && numberTurn !== null && (
                <div>
                    <SofaButton variant="subtle" onClick={() => onView(turnResponse)}>
                        {t("testChat.viewWhy", { number: numberTurn })}
                    </SofaButton>
                </div>
            )}
        </div>
    );
}

function FileActions({ origin: source, actions }: { origin: FileOrigin; actions: ActionsFile }) {
    const { translate: t } = useI18n();
    const downloadingThis = actions.downloading === source.chave;
    const failed = actions.erro?.chave === source.chave ? actions.erro.texto : null;
    return (
        <span className="poph-chat-test__attachment-actions">
            {canSeeOnScreen(source.mime) && (
                <SofaButton
                    variant="subtle"
                    aria-label={t("testChat.viewFileLabel", { name: source.nome })}
                    onClick={() => actions.view(source)}
                >
                    {t("testChat.view")}
                </SofaButton>
            )}
            <SofaButton
                variant="subtle"
                aria-label={t("testChat.downloadFileLabel", { name: source.nome })}
                loading={downloadingThis}
                disabled={actions.downloading !== null && !downloadingThis}
                onClick={() => actions.download(source)}
            >
                {t("testChat.download")}
            </SofaButton>
            {failed && (
                <span className="poph-chat-test__attachment-error" role="alert">
                    <span className="poph-chat-test__error">{failed}</span>
                    <SofaButton
                        variant="secondary"
                        aria-label={t("testChat.retryDownloadLabel", { name: source.nome })}
                        onClick={() => actions.download(source)}
                    >
                        {t("testChat.retry")}
                    </SofaButton>
                </span>
            )}
        </span>
    );
}

function FilesAgent({
    files,
    actions,
}: {
    files: TestSentFile[];
    actions: ActionsFile;
}) {
    const { translate: t } = useI18n();
    return (
        <ul className="poph-chat-test__attachments" aria-label={t("testChat.agentFilesLabel")}>
            {files.map((file) => (
                <li key={file.id}>
                    {file.name} · {readableSize(file.size_bytes)} ·{" "}
                    <span className={file.state === "failed" ? "poph-chat-test__error" : undefined}>
                        {sendLabel(file.state)}
                    </span>
                    <FileActions origin={actions.agent(file)} actions={actions} />
                </li>
            ))}
        </ul>
    );
}

function TurnNoResponse({
    turno,
    numero,
    onView,
    actionsFile,
}: {
    turno: TestTurn;
    numero: number | null;
    onView: (turnId: string) => void;
    actionsFile: ActionsFile;
}) {
    const { translate: t } = useI18n();
    return (
        <div className="poph-chat-test__bubble poph-chat-test__bubble--agent poph-chat-test__no-response">
            <span className="poph-chat-test__author">
                {numero !== null
                    ? t("testChat.turnHeading", { number: numero, state: turnStateLabel(turno.state) })
                    : t("testChat.turnHeadingNoNumber", { state: turnStateLabel(turno.state) })}
            </span>
            <p className="poph-chat-test__event">{phraseForStateWithoutResponse(turno.state)}</p>
            {turno.error && <p className="poph-chat-test__error">{turno.error}</p>}
            {(turno.files_out ?? []).length > 0 && (
                <FilesAgent files={turno.files_out ?? []} actions={actionsFile} />
            )}
            {numero !== null && (
                <div>
                    <SofaButton variant="subtle" onClick={() => onView(turno.id)}>
                        {t("testChat.viewWhy", { number: numero })}
                    </SofaButton>
                </div>
            )}
        </div>
    );
}

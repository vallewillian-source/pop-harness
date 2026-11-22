/**
 * Painel do WhatsApp na aba Integrações (HR-44d): pareamento com QR, estado ao vivo e desconexão.
 *
 * Fala com as três rotas de admin da HR-44b (`api/integracoes.ts`):
 * - `POST …/whatsapp/qrcode` com `If-Match` → QR (data URL), rodada, código de pareamento;
 * - `GET …/whatsapp/state` → estado ao vivo, gravado em `connection_state` pelo servidor;
 * - `POST …/whatsapp/disconnect` com `If-Match` → o aparelho sai e a integração continua cadastrada.
 *
 * Regras que a tela respeita:
 * - **Sem contagem regressiva**: a Evolution não informa validade do QR (`expira_em` é sempre
 *   `null`); o código gira sozinho a cada ~20 s e o botão “Gerar outro QR” resolve.
 * - **Estado ao vivo em laço curto** só enquanto o aparelho aguarda leitura: uma consulta a cada
 *   {@link INTERVALO_DE_CONSULTA_MS}, no máximo {@link MAXIMO_DE_CONSULTAS} vezes (~3 min), parando
 *   ao conectar, ao dar erro, ao sair da tela ou no botão “Parar”.
 * - **Segredo nunca é exibido** e o `base64` do QR não vai para o console do navegador.
 * - `412` relê a integração, adota a versão atual e mantém o painel usável (mesma mensagem das
 *   outras abas); o `409 whatsapp_nao_configurado` aparece com a explicação, sem quebrar a aba.
 *
 * Re-layout (RL-12): só a moldura mudou — o painel deixou de ser cartão e virou seção plana (a
 * seção fica na aba Integrações). Nenhuma consulta, intervalo ou estado foi alterado.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import {
    IconAlertTriangle,
    IconLink,
    IconLogOut,
    IconMessage,
    IconRefresh,
    SofaBanner,
    SofaButton,
    SofaField,
    SofaInput,
    SofaModal,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { ErroApi } from "../api/cliente";
import {
    desconectarWhatsapp,
    explicarFalhaDoWhatsapp,
    fonteDoQr,
    gerarQrDoWhatsapp,
    lerEstadoDoWhatsapp,
    obterIntegracao,
    rotuloDaConexao,
    type EstadoDoWhatsapp,
    type FalhaDoWhatsapp,
    type Integracao,
    type QrDoWhatsapp,
} from "../api/integracoes";
import "../estilos/whatsapp.css";

/** Intervalo entre consultas automáticas do estado (≥ 3 s, como pede a tarefa). */
export const INTERVALO_DE_CONSULTA_MS = 4_000;

/** Teto de consultas automáticas: 45 × 4 s ≈ 3 minutos de espera pelo pareamento. */
export const MAXIMO_DE_CONSULTAS = 45;

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

/** Número no formato que a Evolution aceita: só dígitos, com DDI (10 a 15). */
export const NUMERO_WHATSAPP = /^\d{10,15}$/;

/** Classe do ponto de status (`.poph-status` de `relayout.css`) para o estado da conexão. */
function classeDaConexao(estado: string): string {
    switch (estado) {
        case "connected":
            return "poph-status--ativo";
        case "awaiting_scan":
            return "poph-status--atencao";
        case "disconnected":
            return "poph-status--erro";
        default:
            return "poph-status--inativo";
    }
}

export type PainelWhatsAppProps = {
    org: string;
    agente: string;
    integracao: Integracao;
    /** Devolve ao pai a integração com o estado de conexão / versão atualizados. */
    aoAtualizarIntegracao: (integracao: Integracao) => void;
};

export function PainelWhatsApp({
    org,
    agente,
    integracao,
    aoAtualizarIntegracao,
}: PainelWhatsAppProps) {
    const [qr, setQr] = useState<QrDoWhatsapp | null>(null);
    const [estadoAoVivo, setEstadoAoVivo] = useState<EstadoDoWhatsapp | null>(null);
    const [carregandoQr, setCarregandoQr] = useState(false);
    const [consultando, setConsultando] = useState(false);
    const [desconectando, setDesconectando] = useState(false);
    const [confirmandoDesconexao, setConfirmandoDesconexao] = useState(false);
    const [falha, setFalha] = useState<FalhaDoWhatsapp | null>(null);
    const [aviso, setAviso] = useState<string | null>(null);
    const [lacoAtivo, setLacoAtivo] = useState(false);
    const [numero, setNumero] = useState("");
    const [erroDoNumero, setErroDoNumero] = useState<string | null>(null);

    // A versão é a da integração que o pai mantém; o `412` atualiza as duas.
    const versao = integracao.version;
    const id = integracao.id;
    const estado = estadoAoVivo?.connection_state ?? integracao.connection_state;
    const aguardandoLeitura = estado === "awaiting_scan";

    // O objeto `integracao` muda de identidade a cada leitura de estado; o laço usa a versão atual
    // sem reiniciar o intervalo por isso.
    const integracaoRef = useRef(integracao);
    integracaoRef.current = integracao;

    /** No `412`, relê a integração, adota a versão atual e mantém o painel usável. */
    const relerDepoisDoConflito = useCallback(async () => {
        try {
            const atual = await obterIntegracao(org, agente, id);
            aoAtualizarIntegracao(atual);
            setFalha({
                texto: `Outra pessoa alterou esta integração antes de você (agora na versão ${atual.version}). O estado foi relido: revise e repita a operação.`,
                codigo: "version_conflict",
                dica: "Nada foi perdido: a integração continua como está no servidor.",
            });
        } catch {
            setFalha({
                texto: "Outra pessoa alterou esta integração antes de você, e a releitura falhou. Recarregue a lista e repita.",
                codigo: "version_conflict",
            });
        }
    }, [org, agente, id, aoAtualizarIntegracao]);

    async function conectar() {
        const digitado = numero.trim();
        if (digitado !== "" && !NUMERO_WHATSAPP.test(digitado)) {
            setErroDoNumero("use só dígitos, com DDI e DDD (ex.: 5511999998888)");
            return;
        }
        setErroDoNumero(null);
        setCarregandoQr(true);
        setFalha(null);
        setAviso(null);
        try {
            const resposta = await gerarQrDoWhatsapp(
                org,
                agente,
                id,
                versao,
                digitado === "" ? undefined : digitado,
            );
            setQr(resposta);
            setEstadoAoVivo({
                state: resposta.state,
                connection_state: resposta.connection_state,
                instance: resposta.instance,
            });
            aoAtualizarIntegracao({
                ...integracaoRef.current,
                connection_state: resposta.connection_state,
            });
            if (resposta.connection_state === "awaiting_scan") {
                setLacoAtivo(true);
                setAviso(
                    resposta.pairing_code
                        ? "QR gerado. Leia com o celular ou use o código de pareamento."
                        : "QR gerado. Leia com o WhatsApp do celular.",
                );
            } else if (resposta.state === "open") {
                setLacoAtivo(false);
                setAviso("Este aparelho já está conectado; não há QR para ler.");
            }
        } catch (erro) {
            setLacoAtivo(false);
            if (erro instanceof ErroApi && erro.status === 412) {
                await relerDepoisDoConflito();
            } else {
                setFalha(explicarFalhaDoWhatsapp(erro));
            }
        } finally {
            setCarregandoQr(false);
        }
    }

    const consultarEstado = useCallback(async () => {
        setConsultando(true);
        setFalha(null);
        try {
            const resposta = await lerEstadoDoWhatsapp(org, agente, id);
            setEstadoAoVivo(resposta);
            aoAtualizarIntegracao({
                ...integracaoRef.current,
                connection_state: resposta.connection_state,
            });
            // Conectou (ou caiu): o QR que estava na tela não vale mais.
            if (resposta.connection_state !== "awaiting_scan") setQr(null);
            setAviso(
                `Estado ao vivo: ${rotuloDaConexao(resposta.connection_state).toLowerCase()}.`,
            );
            return resposta;
        } catch (erro) {
            setFalha(explicarFalhaDoWhatsapp(erro));
            return null;
        } finally {
            setConsultando(false);
        }
    }, [org, agente, id, aoAtualizarIntegracao]);

    // Laço curto enquanto o aparelho aguarda leitura do QR.
    useEffect(() => {
        if (!lacoAtivo || !aguardandoLeitura) return;
        let vivo = true;
        let consultas = 0;
        const intervalo = window.setInterval(() => {
            consultas += 1;
            if (consultas > MAXIMO_DE_CONSULTAS) {
                if (!vivo) return;
                setLacoAtivo(false);
                setAviso(
                    "Paramos de consultar o estado automaticamente depois de 3 minutos. Use “Verificar estado” ou “Gerar outro QR”.",
                );
                return;
            }
            void lerEstadoDoWhatsapp(org, agente, id)
                .then((resposta) => {
                    if (!vivo) return;
                    setEstadoAoVivo(resposta);
                    aoAtualizarIntegracao({
                        ...integracaoRef.current,
                        connection_state: resposta.connection_state,
                    });
                    if (resposta.connection_state !== "awaiting_scan") {
                        setLacoAtivo(false);
                        setQr(null);
                        setAviso(
                            resposta.connection_state === "connected"
                                ? "Aparelho conectado."
                                : `Estado ao vivo: ${rotuloDaConexao(resposta.connection_state).toLowerCase()}.`,
                        );
                    }
                })
                .catch((erro) => {
                    if (!vivo) return;
                    setLacoAtivo(false);
                    setFalha(explicarFalhaDoWhatsapp(erro));
                });
        }, INTERVALO_DE_CONSULTA_MS);
        return () => {
            vivo = false;
            window.clearInterval(intervalo);
        };
    }, [lacoAtivo, aguardandoLeitura, org, agente, id, aoAtualizarIntegracao]);

    async function desconectar() {
        setConfirmandoDesconexao(false);
        setDesconectando(true);
        setFalha(null);
        setAviso(null);
        try {
            const resposta = await desconectarWhatsapp(org, agente, id, versao);
            setLacoAtivo(false);
            setQr(null);
            setEstadoAoVivo({
                state: "close",
                connection_state: resposta.connection_state,
                instance: resposta.instance,
            });
            aoAtualizarIntegracao({
                ...integracaoRef.current,
                connection_state: resposta.connection_state,
            });
            setAviso(
                "Aparelho desconectado. A integração continua cadastrada; para voltar, gere um novo QR.",
            );
        } catch (erro) {
            if (erro instanceof ErroApi && erro.status === 412) {
                await relerDepoisDoConflito();
            } else {
                setFalha(explicarFalhaDoWhatsapp(erro));
            }
        } finally {
            setDesconectando(false);
        }
    }

    const base64 = qr?.qrcode?.base64 ?? null;
    const rodada = qr?.qrcode?.count ?? null;

    return (
        <div className="poph-whatsapp">
            <div className="poph-whatsapp__estado">
                <span className="poph-mono-mudo">{id}</span>
                <span className={`poph-status ${classeDaConexao(estado)}`}>
                    <span className="poph-status__ponto" aria-hidden="true" />
                    {rotuloDaConexao(estado)}
                </span>
                <span className="poph-mono-mudo">
                    {integracao.tem_segredo
                        ? "segredo gravado — nunca exibido"
                        : "sem segredo gravado"}
                </span>
                {qr?.instance && <span className="poph-mono-mudo">instância: {qr.instance}</span>}
            </div>

            <SofaField
                label="Número do WhatsApp (opcional)"
                hint="Só dígitos, com DDI e DDD. Em branco, o pareamento é pelo QR. Com o número, o WhatsApp também devolve o código de 8 caracteres."
                error={erroDoNumero ?? undefined}
                htmlFor="poph-whatsapp-numero"
            >
                {(controle) => (
                    <SofaInput
                        {...semDescribedBy(controle)}
                        value={numero}
                        inputMode="numeric"
                        autoComplete="off"
                        placeholder="5511999998888"
                        onChange={(evento) => setNumero(evento.target.value)}
                    />
                )}
            </SofaField>

            <div className="poph-whatsapp__acoes">
                <SofaButton
                    variant="primary"
                    size="sm"
                    icon={<IconMessage aria-hidden="true" />}
                    onClick={() => void conectar()}
                    loading={carregandoQr}
                >
                    {qr ? "Gerar outro QR" : "Conectar / gerar QR"}
                </SofaButton>
                <SofaButton
                    variant="subtle"
                    size="sm"
                    icon={<IconRefresh aria-hidden="true" />}
                    onClick={() => {
                        void consultarEstado();
                    }}
                    loading={consultando}
                >
                    Verificar estado
                </SofaButton>
                <SofaButton
                    variant="subtle"
                    size="sm"
                    icon={<IconLogOut aria-hidden="true" />}
                    onClick={() => {
                        setAviso(null);
                        setConfirmandoDesconexao(true);
                    }}
                    loading={desconectando}
                    disabled={estado === "disconnected"}
                >
                    Desconectar aparelho
                </SofaButton>
                {lacoAtivo && aguardandoLeitura && (
                    <SofaButton variant="subtle" size="sm" onClick={() => setLacoAtivo(false)}>
                        Parar consulta automática
                    </SofaButton>
                )}
            </div>

            {aviso && (
                <p className="poph-mono-mudo" style={{ margin: 0 }} role="status">
                    {aviso}
                </p>
            )}

            {falha && (
                <SofaBanner tone="danger" title="Não consegui completar a operação">
                    <span>{falha.texto}</span>
                    {falha.dica && <div className="poph-whatsapp__dica">{falha.dica}</div>}
                    {falha.codigo && (
                        <div className="poph-mono-mudo poph-whatsapp__dica">
                            código: {falha.codigo}
                        </div>
                    )}
                </SofaBanner>
            )}

            {base64 ? (
                <div className="poph-whatsapp__qr-bloco">
                    <img
                        className="poph-whatsapp__qr-imagem"
                        src={fonteDoQr(base64)}
                        alt="QR de pareamento do WhatsApp"
                        width={220}
                        height={220}
                    />
                    <div className="poph-whatsapp__qr-texto">
                        <strong>Leia este QR com o WhatsApp do celular</strong>
                        <span className="poph-mono-mudo">
                            No celular: WhatsApp → Aparelhos conectados → Conectar aparelho.
                        </span>
                        {rodada !== null && (
                            <span className="poph-mono-mudo">rodada {rodada} do QR</span>
                        )}
                        {qr?.pairing_code && (
                            <span>
                                Código de pareamento:{" "}
                                <code className="poph-mono-mudo">{qr.pairing_code}</code>
                            </span>
                        )}
                        <span className="poph-mono-mudo">
                            O QR muda sozinho a cada ~20 s e não tem prazo informado pelo WhatsApp.
                            Se demorar, clique em “Gerar outro QR”.
                        </span>
                    </div>
                </div>
            ) : qr?.state === "open" || estado === "connected" ? (
                <SofaBanner tone="success" title="Aparelho conectado">
                    O aparelho está conectado a esta integração; não há QR para ler. Para trocar de
                    aparelho, desconecte e gere um novo QR.
                </SofaBanner>
            ) : (
                <p className="poph-mono-mudo" style={{ margin: 0 }}>
                    Nenhum QR na tela. Use “Conectar / gerar QR” para parear o celular; o estado
                    acima é o que o servidor tem gravado até você consultar o estado ao vivo.
                </p>
            )}

            {aguardandoLeitura && lacoAtivo && (
                <p className="poph-mono-mudo" style={{ margin: 0 }}>
                    Consultando o estado a cada {INTERVALO_DE_CONSULTA_MS / 1000} s por até{" "}
                    {Math.round((INTERVALO_DE_CONSULTA_MS * MAXIMO_DE_CONSULTAS) / 60_000)} min.
                </p>
            )}

            <SofaModal
                open={confirmandoDesconexao}
                onOpenChange={(novo) => {
                    if (!novo && !desconectando) setConfirmandoDesconexao(false);
                }}
                title="Desconectar o WhatsApp?"
                description="O aparelho para de receber e enviar mensagens por esta integração."
                closeLabel="Fechar confirmação de desconexão"
                size="sm"
                id="poph-confirma-desconexao-whatsapp"
                footer={
                    <>
                        <SofaButton
                            variant="secondary"
                            disabled={desconectando}
                            onClick={() => setConfirmandoDesconexao(false)}
                        >
                            Cancelar
                        </SofaButton>
                        <SofaButton
                            variant="danger"
                            icon={<IconLogOut aria-hidden="true" />}
                            loading={desconectando}
                            onClick={() => void desconectar()}
                        >
                            Desconectar
                        </SofaButton>
                    </>
                }
            >
                <div className="poph-whatsapp__confirmacao">
                    <div className="poph-whatsapp__confirmacao-linha">
                        <IconAlertTriangle aria-hidden="true" />
                        <span>
                            A integração <strong>continua cadastrada</strong>; o que sai é o
                            pareamento do aparelho. Para reconectar, é só gerar um novo QR e ler com
                            o celular.
                        </span>
                    </div>
                    <p className="poph-mono-mudo" style={{ margin: 0 }}>
                        <IconLink aria-hidden="true" /> integração {id}
                    </p>
                </div>
            </SofaModal>
        </div>
    );
}

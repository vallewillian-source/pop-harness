/**
 * Operação da instância (HR-48a, HR-48c): o servidor está de pé, em que versão, há quanto tempo,
 * como estão as filas e quanto de recurso ele usa, e quais plugins estão declarados (com o detalhe
 * de cada um).
 *
 * Quatro seções independentes. Cada uma tem carga, erro com "Tentar de novo" e texto próprio: uma
 * falha em uma não derruba as outras. A atualização automática acontece a cada 15 s, só com a aba
 * visível; o botão "Atualizar" faz o mesmo na hora. As rotas estão em `api/operacao.ts`.
 *
 * Seção "Cluster" (HR-48f): recarrega o mapa de cluster no processo vivo, com confirmação. A tela
 * não mostra o mapa em uso agora; mostra o resultado das recargas feitas nesta sessão.
 *
 * O que a tela não mostra, de propósito: instalar ou remover plugin (a API não tem essa ação) e a
 * latência por etapa do turno (é um histograma, que a tela ainda não desenha).
 */

import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useParams } from "react-router-dom";
import {
    IconRefresh,
    SofaBanner,
    SofaButton,
    SofaConfirmDialog,
    SofaErrorState,
    SofaSkeleton,
} from "@sofacoding/sofa-ui";
import { ErroApi } from "../api/cliente";
import {
    consultarMetricas,
    consultarPlugin,
    consultarSaude,
    listarPlugins,
    recarregarCluster,
    resumoDoManifesto,
    type ItemDePlugin,
    type Medicoes,
    type RelatorioRecarga,
    type ResumoDoMapa,
    type SaudeDaInstancia,
} from "../api/operacao";
import type { ManifestoDoPlugin } from "../api/integracoes";
import { ExportarImportarOrganizacao } from "../componentes/ExportarImportarOrganizacao";
import "../estilos/operacao.css";

/** Intervalo da atualização automática. Nunca menos que isso (sem laço de requisições). */
export const INTERVALO_DE_ATUALIZACAO_MS = 15_000;

/** Tempo no ar em português, do mais grosso ao mais fino: "2 dias e 3 h", "5 min", "menos de 1 min". */
export function formatarTempoNoAr(segundos: number): string {
    const total = Math.max(0, Math.floor(segundos));
    const dias = Math.floor(total / 86_400);
    const horas = Math.floor((total % 86_400) / 3_600);
    const minutos = Math.floor((total % 3_600) / 60);
    if (dias > 0) return `${dias} ${dias === 1 ? "dia" : "dias"} e ${horas} h`;
    if (horas > 0) return `${horas} h e ${minutos} min`;
    if (minutos > 0) return `${minutos} min`;
    return "menos de 1 min";
}

/** Bytes em KB, MB ou GB, com vírgula brasileira. */
export function formatarMemoria(bytes: number): string {
    const kb = 1024;
    const mb = kb * 1024;
    const gb = mb * 1024;
    const formatar = (valor: number, unidade: string) =>
        `${valor.toLocaleString("pt-BR", { maximumFractionDigits: 1 })} ${unidade}`;
    if (bytes >= gb) return formatar(bytes / gb, "GB");
    if (bytes >= mb) return formatar(bytes / mb, "MB");
    return formatar(bytes / kb, "KB");
}

/**
 * Versão de esquema. O servidor manda `0` quando o arquivo do banco ainda não foi migrado (ou não
 * pôde ser lido): não é falha de conexão, então a tela diz só que não há versão gravada.
 */
export function formatarVersaoDoBanco(versao: number): string {
    return versao > 0 ? `esquema ${versao}` : "sem versão gravada ainda";
}

function numero(valor: number | undefined): string {
    return valor === undefined ? "não informado" : valor.toLocaleString("pt-BR");
}

function memoria(valor: number | undefined): string {
    return valor === undefined ? "não informado" : formatarMemoria(valor);
}

/** Frase para o operador a partir do erro da API (sem caminho, sem corpo cru). */
export function mensagemDeFalha(erro: unknown, oQue: string): string {
    if (erro instanceof ErroApi) {
        if (erro.semServidor) return "Não consegui falar com a instância. Confira se o servidor está no ar.";
        if (erro.status === 401) return "Sua sessão não vale mais. Entre de novo para continuar.";
        if (erro.status === 403) {
            return "Seu token não alcança esta organização. Use um token dela ou um token global.";
        }
        if (erro.status === 404) return `Não encontrei ${oQue}.`;
        if (erro.status >= 500) {
            return `A instância respondeu com erro (${erro.status}). Tente de novo em instantes.`;
        }
        return erro.message;
    }
    return "Algo deu errado. Tente de novo.";
}

type Estado<T> = { carregando: boolean; dados: T | null; erro: string | null };

const estadoInicial = <T,>(): Estado<T> => ({ carregando: true, dados: null, erro: null });

type Consulta = {
    carregando: boolean;
    kind: string;
    manifesto: ManifestoDoPlugin | null;
    erro: string | null;
};

function Propriedade({ chave, valor, mono }: { chave: string; valor: string; mono?: boolean }) {
    return (
        <div className="poph-propriedade">
            <dt className="poph-propriedade__chave">{chave}</dt>
            <dd className={mono ? "poph-propriedade__valor poph-propriedade__valor--mono" : "poph-propriedade__valor"}>
                {valor}
            </dd>
        </div>
    );
}

/**
 * Uma linha da lista de plugins. Plugin no ar tem botão de detalhe com o tipo no rótulo. Plugin
 * indisponível não tem ação: mostra a situação em texto (não só cor) e a mensagem do servidor.
 */
function LinhaDePlugin({
    item,
    aberto,
    onAbrir,
}: {
    item: ItemDePlugin;
    aberto: boolean;
    onAbrir: () => void;
}) {
    if (item.situacao === "ok") {
        return (
            <li className="poph-operacao__plugin">
                <div className="poph-operacao__plugin-texto">
                    <span className="poph-operacao__selo poph-operacao__selo--ok">No ar</span>
                    <span className="poph-operacao__plugin-nome">{item.nome ?? item.kind}</span>
                    <span className="poph-operacao__plugin-meta">
                        tipo <code>{item.kind}</code>
                        {item.versao ? ` · versão ${item.versao}` : ""}
                    </span>
                </div>
                <SofaButton
                    variant="subtle"
                    aria-label={`Ver detalhes do plugin ${item.kind}`}
                    aria-pressed={aberto}
                    onClick={onAbrir}
                >
                    Ver detalhes
                </SofaButton>
            </li>
        );
    }
    return (
        <li className="poph-operacao__plugin poph-operacao__plugin--indisponivel">
            <div className="poph-operacao__plugin-texto">
                <span className="poph-operacao__selo poph-operacao__selo--indisponivel">Indisponível</span>
                <span className="poph-operacao__plugin-nome">
                    tipo <code>{item.kind}</code>
                </span>
                <span className="poph-operacao__plugin-meta">{(item.mensagem ?? "").replace(/`/g, "")}</span>
            </div>
        </li>
    );
}

/** Quantos caracteres da impressão digital aparecem na tela; o valor completo fica no `title`. */
const TAMANHO_DA_IMPRESSAO = 12;

/** Impressão digital encurtada para a tela (o valor completo continua no atributo `title`). */
export function impressaoNaTela(impressao: string): string {
    if (!impressao) return "não informada";
    return impressao.length > TAMANHO_DA_IMPRESSAO
        ? `${impressao.slice(0, TAMANHO_DA_IMPRESSAO)}…`
        : impressao;
}

/**
 * Falha de uma recarga, já em forma de tela. Em `invalido` o mapa anterior segue em uso; em
 * `sem_permissao` o token não é global; em `outra` a frase vem pronta.
 */
export type FalhaDaRecarga =
    | { tipo: "invalido"; arquivo: string | null; problemas: string[] }
    | { tipo: "sem_permissao" }
    | { tipo: "outra"; mensagem: string };

/** Traduz o erro da recarga. Sem resposta da instância, a tela diz que não sabe se foi aplicada. */
export function falhaDaRecarga(erro: unknown): FalhaDaRecarga {
    if (erro instanceof ErroApi) {
        if (erro.status === 400) {
            const bruto: unknown = erro.detalhes;
            const detalhes =
                bruto !== null && typeof bruto === "object" ? (bruto as Record<string, unknown>) : {};
            const problemas = Array.isArray(detalhes.problemas)
                ? detalhes.problemas.filter((item): item is string => typeof item === "string")
                : [];
            return {
                tipo: "invalido",
                arquivo: typeof detalhes.arquivo === "string" ? detalhes.arquivo : null,
                problemas: problemas.length > 0 ? problemas : [erro.message],
            };
        }
        if (erro.status === 403) return { tipo: "sem_permissao" };
        if (erro.semServidor) {
            return {
                tipo: "outra",
                mensagem:
                    "Não recebi resposta da instância, então não sei se a recarga chegou a ser aplicada. Tente de novo e confira o resultado que aparecer aqui.",
            };
        }
    }
    return { tipo: "outra", mensagem: mensagemDeFalha(erro, "a recarga do mapa") };
}

function listaOuNenhuma(itens: string[]): string {
    return itens.length > 0 ? itens.join(", ") : "nenhuma";
}

function modoEmTexto(resumo: ResumoDoMapa): string {
    return resumo.modo === "cluster" ? "Cluster" : "Instância única";
}

/** Resumo de um mapa em duas colunas (antes e depois). Sem segredo: só se está configurado. */
function TabelaDoMapa({ antes, depois, impressaoAntes, impressaoDepois }: {
    antes: ResumoDoMapa;
    depois: ResumoDoMapa;
    impressaoAntes: string;
    impressaoDepois: string;
}) {
    const linhas: { rotulo: string; antes: ReactNode; depois: ReactNode }[] = [
        { rotulo: "Modo", antes: modoEmTexto(antes), depois: modoEmTexto(depois) },
        {
            rotulo: "Esta instalação",
            antes: antes.instanciaLocal || "não definida",
            depois: depois.instanciaLocal || "não definida",
        },
        {
            rotulo: "Instância padrão",
            antes: antes.instanciaPadrao ?? "nenhuma",
            depois: depois.instanciaPadrao ?? "nenhuma",
        },
        {
            rotulo: "Instâncias declaradas",
            antes: listaOuNenhuma(antes.instancias.map((item) => item.id)),
            depois: listaOuNenhuma(depois.instancias.map((item) => item.id)),
        },
        {
            rotulo: "Orgs roteadas",
            antes: String(antes.quantidadeDeOrgs),
            depois: String(depois.quantidadeDeOrgs),
        },
        {
            rotulo: "Segredo compartilhado",
            antes: antes.segredoConfigurado ? "configurado" : "não configurado",
            depois: depois.segredoConfigurado ? "configurado" : "não configurado",
        },
        {
            rotulo: "Impressão digital",
            antes: <code title={impressaoAntes}>{impressaoNaTela(impressaoAntes)}</code>,
            depois: <code title={impressaoDepois}>{impressaoNaTela(impressaoDepois)}</code>,
        },
    ];
    return (
        <table className="poph-operacao__tabela" aria-label="Mapa antes e depois da recarga">
            <thead>
                <tr>
                    <th scope="col">Item</th>
                    <th scope="col">Antes</th>
                    <th scope="col">Depois</th>
                </tr>
            </thead>
            <tbody>
                {linhas.map((linha) => (
                    <tr key={linha.rotulo}>
                        <th scope="row">{linha.rotulo}</th>
                        <td>{linha.antes}</td>
                        <td>{linha.depois}</td>
                    </tr>
                ))}
            </tbody>
        </table>
    );
}

/** Resultado de uma recarga que deu certo: mudou ou não, o que entrou e saiu, e as orgs que trocaram. */
function RelatorioDaRecarga({ relatorio, titulo }: { relatorio: RelatorioRecarga; titulo: string }) {
    return (
        <div className="poph-operacao__relatorio">
            <h3 className="poph-operacao__subtitulo">{titulo}</h3>
            <SofaBanner
                tone={relatorio.mudou ? "success" : "info"}
                title={relatorio.mudou ? "Mapa recarregado: houve mudança" : "Mapa recarregado: nada mudou"}
            >
                {relatorio.mudou
                    ? "O mapa novo já está em uso nesta instalação."
                    : "O arquivo tem o mesmo conteúdo do mapa que já estava em uso."}
            </SofaBanner>
            {relatorio.depois.modo === "instancia_unica" ? (
                <p className="poph-operacao__ajuda">
                    Esta instalação está em modo instância única: não há mapa de cluster configurado, e ela
                    atende todas as organizações sozinha. Não é erro.
                </p>
            ) : null}
            <TabelaDoMapa
                antes={relatorio.antes}
                depois={relatorio.depois}
                impressaoAntes={relatorio.impressaoAntes}
                impressaoDepois={relatorio.impressaoDepois}
            />
            <p className="poph-operacao__ajuda">
                Instâncias adicionadas: <strong>{listaOuNenhuma(relatorio.instanciasAdicionadas)}</strong>.
                Instâncias removidas: <strong>{listaOuNenhuma(relatorio.instanciasRemovidas)}</strong>.
            </p>
            {relatorio.orgsQueTrocaram.length > 0 ? (
                <>
                    <p className="poph-operacao__ajuda">Orgs que trocaram de instância:</p>
                    <ul className="poph-operacao__conflitos" aria-label="Orgs que trocaram de instância">
                        {relatorio.orgsQueTrocaram.map((troca) => (
                            <li key={troca.org}>
                                {troca.org}: de {troca.de ?? "sem destino"} para {troca.para ?? "sem destino"}
                            </li>
                        ))}
                    </ul>
                </>
            ) : (
                <p className="poph-operacao__ajuda">Nenhuma org trocou de instância.</p>
            )}
        </div>
    );
}

/** Aviso de uma recarga que falhou. Em 400 diz que o mapa anterior segue valendo. */
function AvisoDaRecarga({ falha, aoTentarDeNovo }: { falha: FalhaDaRecarga; aoTentarDeNovo: () => void }) {
    if (falha.tipo === "invalido") {
        return (
            <SofaBanner tone="danger" title="O arquivo de cluster tem erros">
                <p style={{ margin: 0 }}>
                    Nada mudou: a instância segue com o mapa que já estava em uso. Corrija o arquivo e recarregue.
                </p>
                {falha.arquivo ? (
                    <p className="poph-mono-mudo" style={{ margin: "6px 0 0" }}>
                        Arquivo: {falha.arquivo}
                    </p>
                ) : null}
                <p style={{ margin: "6px 0 0" }}>Problemas encontrados:</p>
                <ul className="poph-operacao__conflitos" aria-label="Problemas encontrados no arquivo de cluster">
                    {falha.problemas.map((problema, indice) => (
                        <li key={`${indice}-${problema}`}>{problema}</li>
                    ))}
                </ul>
            </SofaBanner>
        );
    }
    if (falha.tipo === "sem_permissao") {
        return (
            <SofaBanner tone="danger" title="Esta ação exige token global">
                <p style={{ margin: 0 }}>
                    Recarregar o mapa vale para a instalação inteira. Entre com o token global; o token de uma
                    organização não alcança essa ação.
                </p>
            </SofaBanner>
        );
    }
    return (
        <SofaBanner tone="danger" title="Não consegui recarregar o mapa">
            <p style={{ margin: 0 }}>{falha.mensagem}</p>
            <div style={{ marginTop: 8 }}>
                <SofaButton variant="secondary" onClick={aoTentarDeNovo}>
                    Tentar de novo
                </SofaButton>
            </div>
        </SofaBanner>
    );
}

/**
 * Seção Cluster. A recarga é ação da instalação inteira: o botão só abre a confirmação, e o pedido
 * sai uma vez (a trava `emCurso` e o botão desabilitado impedem duplo envio).
 */
function SecaoCluster() {
    const idSecao = useId();
    const [confirmando, setConfirmando] = useState(false);
    const [recarregando, setRecarregando] = useState(false);
    const [ultimoSucesso, setUltimoSucesso] = useState<RelatorioRecarga | null>(null);
    const [falha, setFalha] = useState<FalhaDaRecarga | null>(null);
    const emCurso = useRef(false);
    const vivo = useRef(true);

    useEffect(() => {
        vivo.current = true;
        return () => {
            vivo.current = false;
        };
    }, []);

    const recarregar = useCallback(async () => {
        if (emCurso.current) return;
        emCurso.current = true;
        setConfirmando(false);
        setRecarregando(true);
        setFalha(null);
        try {
            const relatorio = await recarregarCluster();
            if (!vivo.current) return;
            setUltimoSucesso(relatorio);
        } catch (erro) {
            if (!vivo.current) return;
            setFalha(falhaDaRecarga(erro));
        } finally {
            emCurso.current = false;
            if (vivo.current) setRecarregando(false);
        }
    }, []);

    return (
        <section className="poph-secao" aria-labelledby={idSecao}>
            <h2 id={idSecao} className="poph-secao__titulo">Cluster</h2>
            <p className="poph-operacao__ajuda">
                Relê o arquivo de cluster desta instalação e troca o mapa em uso, para todas as organizações.
                É uma ação da instalação inteira: só o token global pode fazer, e ela pede confirmação. Se o
                arquivo tiver erro, o mapa anterior continua valendo. A tela não mostra o mapa em uso agora;
                mostra o resultado das recargas feitas nesta sessão.
            </p>

            <div className="poph-pilha" style={{ gap: "var(--pop-spacing-sm, 6px)", maxWidth: 680 }}>
                <div>
                    <SofaButton
                        variant="secondary"
                        loading={recarregando}
                        disabled={recarregando}
                        onClick={() => setConfirmando(true)}
                    >
                        Recarregar mapa do cluster
                    </SofaButton>
                </div>

                <div aria-live="polite">
                    {recarregando ? (
                        <p className="poph-operacao__carregando" role="status">
                            Recarregando o mapa…
                        </p>
                    ) : null}
                    {falha ? (
                        <AvisoDaRecarga falha={falha} aoTentarDeNovo={() => setConfirmando(true)} />
                    ) : null}
                    {ultimoSucesso ? (
                        <RelatorioDaRecarga
                            relatorio={ultimoSucesso}
                            titulo={falha ? "Última recarga que deu certo nesta sessão" : "Resultado da recarga"}
                        />
                    ) : !falha && !recarregando ? (
                        <p className="poph-operacao__vazio">
                            Nenhuma recarga feita nesta sessão ainda. O botão relê o arquivo de cluster e mostra o
                            que mudou.
                        </p>
                    ) : null}
                </div>
            </div>

            <SofaConfirmDialog
                open={confirmando}
                onOpenChange={(aberto) => {
                    if (!aberto && !recarregando) setConfirmando(false);
                }}
                title="Recarregar o mapa do cluster?"
                description="Isto relê o arquivo de cluster desta instalação e troca o mapa em uso para todas as organizações. Se o arquivo tiver erro, nada muda e o mapa anterior segue valendo."
                confirmLabel="Recarregar agora"
                cancelLabel="Cancelar"
                closeLabel="Fechar confirmação da recarga"
                onConfirm={() => void recarregar()}
                loading={recarregando}
            />
        </section>
    );
}

export function Operacao() {
    const { orgId = "" } = useParams();
    const [saude, setSaude] = useState<Estado<SaudeDaInstancia>>(estadoInicial);
    const [metricas, setMetricas] = useState<Estado<Medicoes>>(estadoInicial);
    const [plugins, setPlugins] = useState<Estado<ItemDePlugin[]>>(estadoInicial);
    const [atualizadoEm, setAtualizadoEm] = useState<Date | null>(null);
    const [consulta, setConsulta] = useState<Consulta>({
        carregando: false,
        kind: "",
        manifesto: null,
        erro: null,
    });
    const emCurso = useRef(false);
    const vivo = useRef(true);
    const idSaude = useId();
    const idMetricas = useId();
    const idPlugin = useId();

    useEffect(() => {
        vivo.current = true;
        return () => {
            vivo.current = false;
        };
    }, []);

    const carregar = useCallback(async () => {
        if (emCurso.current) return;
        emCurso.current = true;
        setSaude((atual) => ({ ...atual, carregando: true }));
        setMetricas((atual) => ({ ...atual, carregando: true }));
        setPlugins((atual) => ({ ...atual, carregando: true }));
        const [resSaude, resMetricas, resPlugins] = await Promise.allSettled([
            consultarSaude(),
            consultarMetricas(),
            listarPlugins(orgId),
        ]);
        emCurso.current = false;
        if (!vivo.current) return;

        if (resSaude.status === "fulfilled") {
            setSaude({ carregando: false, dados: resSaude.value, erro: null });
        } else {
            setSaude((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: mensagemDeFalha(resSaude.reason, "o estado do servidor"),
            }));
        }

        if (resMetricas.status === "fulfilled") {
            setMetricas({ carregando: false, dados: resMetricas.value, erro: null });
        } else {
            setMetricas((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: mensagemDeFalha(resMetricas.reason, "as métricas"),
            }));
        }

        if (resPlugins.status === "fulfilled") {
            setPlugins({ carregando: false, dados: resPlugins.value, erro: null });
        } else {
            setPlugins((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: mensagemDeFalha(resPlugins.reason, "a lista de plugins"),
            }));
        }
        setAtualizadoEm(new Date());
    }, [orgId]);

    useEffect(() => {
        void carregar();
        const id = window.setInterval(() => {
            if (document.visibilityState === "visible") void carregar();
        }, INTERVALO_DE_ATUALIZACAO_MS);
        return () => window.clearInterval(id);
    }, [carregar]);

    const consultarTipo = useCallback(
        async (kind: string) => {
            setConsulta({ carregando: true, kind, manifesto: null, erro: null });
            try {
                const manifesto = await consultarPlugin(orgId, kind);
                if (!vivo.current) return;
                setConsulta({ carregando: false, kind, manifesto, erro: null });
            } catch (erro) {
                if (!vivo.current) return;
                const naoDeclarado = erro instanceof ErroApi && erro.status === 404;
                const naoRespondeu = erro instanceof ErroApi && erro.status === 503;
                const mensagem = naoDeclarado
                    ? "Este tipo não tem plugin declarado nesta instância."
                    : naoRespondeu
                      ? "O plugin está declarado, mas não respondeu ao teste de conexão."
                      : mensagemDeFalha(erro, "o plugin");
                setConsulta({ carregando: false, kind, manifesto: null, erro: mensagem });
            }
        },
        [orgId],
    );

    const m = metricas.dados;
    const s = saude.dados;
    const lista = plugins.dados;

    return (
        <div className="poph-pagina poph-operacao">
            <div className="poph-pagina__topo">
                <div>
                    <h1 className="poph-pagina__titulo">Saúde da instância</h1>
                    <p className="poph-pagina__descricao">
                        Se o servidor está no ar, em que versão, como estão as filas e quanto de
                        recurso ele usa. As métricas são deste processo e zeram quando ele reinicia.
                    </p>
                </div>
                <div className="poph-pagina__acoes">
                    <span className="poph-operacao__atualizacao" role="status" aria-live="polite">
                        {saude.carregando || metricas.carregando || plugins.carregando
                            ? "Atualizando…"
                            : atualizadoEm
                              ? `Atualizado às ${atualizadoEm.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })}`
                              : ""}
                    </span>
                    <SofaButton
                        variant="subtle"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void carregar()}
                        disabled={saude.carregando || metricas.carregando || plugins.carregando}
                    >
                        Atualizar
                    </SofaButton>
                </div>
            </div>

            <section className="poph-secao" aria-labelledby={idSaude}>
                <h2 id={idSaude} className="poph-secao__titulo">Servidor</h2>
                {saude.carregando && !s ? (
                    <SofaSkeleton lines={3} />
                ) : saude.erro && !s ? (
                    <SofaErrorState
                        title="Não consegui ler o estado do servidor"
                        description={saude.erro}
                        retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                    />
                ) : s ? (
                    <>
                        {saude.erro ? (
                            <p className="poph-operacao__aviso" role="alert">
                                A última atualização falhou: {saude.erro} Mostrando o último resultado.
                            </p>
                        ) : null}
                        <dl className="poph-propriedades">
                            <Propriedade
                                chave="Situação"
                                valor={
                                    s.situacao === "ok"
                                        ? "No ar, com os bancos respondendo"
                                        : "No ar, mas um dos bancos não respondeu"
                                }
                            />
                            <Propriedade chave="Versão do servidor" valor={s.saude.version} mono />
                            <Propriedade chave="Tempo no ar" valor={formatarTempoNoAr(s.saude.uptime_seconds)} />
                            <Propriedade
                                chave="Banco de conversas"
                                valor={formatarVersaoDoBanco(s.saude.databases.harness)}
                            />
                            <Propriedade
                                chave="Banco da base de busca"
                                valor={formatarVersaoDoBanco(s.saude.databases.rag)}
                            />
                        </dl>
                    </>
                ) : null}
            </section>

            <section className="poph-secao" aria-labelledby={idMetricas}>
                <h2 id={idMetricas} className="poph-secao__titulo">Filas e recursos</h2>
                {metricas.carregando && !m ? (
                    <SofaSkeleton lines={4} />
                ) : metricas.erro && !m ? (
                    <SofaErrorState
                        title="Não consegui ler as métricas"
                        description={metricas.erro}
                        retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                    />
                ) : m ? (
                    <>
                        {metricas.erro ? (
                            <p className="poph-operacao__aviso" role="alert">
                                A última atualização falhou: {metricas.erro} Mostrando o último resultado.
                            </p>
                        ) : null}
                        <h3 className="poph-operacao__subtitulo">Agora</h3>
                        <dl className="poph-propriedades">
                            <Propriedade chave="Turnos esperando na fila" valor={numero(m.filaTurnos)} />
                            <Propriedade chave="Mensagens esperando envio" valor={numero(m.filaSaida)} />
                            <Propriedade chave="Conversas com atendimento em curso" valor={numero(m.conversasAtivas)} />
                            <Propriedade chave="Agentes carregados na memória" valor={numero(m.agentesCarregados)} />
                            <Propriedade chave="Memória usada pelo processo" valor={memoria(m.memoriaProcesso)} />
                            <Propriedade chave="Tamanho do banco de conversas" valor={memoria(m.bancoPrincipal)} />
                            <Propriedade chave="Tamanho da base de busca" valor={memoria(m.bancoBusca)} />
                        </dl>
                        <h3 className="poph-operacao__subtitulo">Desde a última subida do servidor</h3>
                        <dl className="poph-propriedades">
                            <Propriedade
                                chave="Chamadas ao modelo (LLM)"
                                valor={`${numero(m.chamadasLlm)} · com erro: ${numero(m.errosLlm)}`}
                            />
                            <Propriedade
                                chave="Chamadas ao Jev"
                                valor={`${numero(m.chamadasJev)} · com erro: ${numero(m.errosJev)}`}
                            />
                            <Propriedade
                                chave="Chamadas à Evolution (WhatsApp)"
                                valor={`${numero(m.chamadasEvolution)} · com erro: ${numero(m.errosEvolution)}`}
                            />
                            <Propriedade chave="Tokens consumidos" valor={numero(m.tokens)} />
                        </dl>
                    </>
                ) : null}
            </section>

            <section className="poph-secao" aria-labelledby={idPlugin}>
                <h2 id={idPlugin} className="poph-secao__titulo">Plugins declarados</h2>
                <p className="poph-operacao__ajuda">
                    Os plugins que esta instância declara. Um plugin que não respondeu aparece marcado,
                    e os outros seguem na lista. Escolha um plugin no ar para ver os detalhes. Instalar
                    e remover plugin não estão nesta tela.
                </p>

                {plugins.carregando && !lista ? (
                    <>
                        <p className="poph-operacao__carregando" role="status">
                            Consultando plugins… a primeira consulta sobe cada plugin, então pode demorar.
                        </p>
                        <SofaSkeleton lines={3} />
                    </>
                ) : plugins.erro && !lista ? (
                    <SofaErrorState
                        title="Não consegui ler a lista de plugins"
                        description={plugins.erro}
                        retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                    />
                ) : lista ? (
                    <>
                        {plugins.erro ? (
                            <p className="poph-operacao__aviso" role="alert">
                                A última atualização falhou: {plugins.erro} Mostrando o último resultado.
                            </p>
                        ) : null}
                        {lista.length === 0 ? (
                            <p className="poph-operacao__vazio">
                                Nenhum plugin declarado nesta instância. Os plugins se declaram na seção
                                <code> [plugins].integracoes</code> da configuração do servidor; não é um erro.
                            </p>
                        ) : (
                            <ul className="poph-operacao__lista-plugins" aria-label="Plugins declarados">
                                {lista.map((item) => (
                                    <LinhaDePlugin
                                        key={item.kind}
                                        item={item}
                                        aberto={consulta.kind === item.kind && !!consulta.manifesto}
                                        onAbrir={() => void consultarTipo(item.kind)}
                                    />
                                ))}
                            </ul>
                        )}
                    </>
                ) : null}

                <div aria-live="polite">
                    {consulta.carregando ? (
                        <>
                            <p className="poph-operacao__carregando" role="status">
                                Consultando o plugin "{consulta.kind}"…
                            </p>
                            <SofaSkeleton lines={3} />
                        </>
                    ) : consulta.erro ? (
                        <SofaErrorState
                            title={`Não consegui consultar o plugin "${consulta.kind}"`}
                            description={consulta.erro}
                            retry={{
                                label: "Tentar de novo",
                                onClick: () => void consultarTipo(consulta.kind),
                            }}
                        />
                    ) : consulta.manifesto ? (
                        <dl className="poph-propriedades poph-operacao__detalhe" aria-label={`Detalhes do plugin ${consulta.kind}`}>
                            <Propriedade chave="Nome" valor={consulta.manifesto.nome} />
                            <Propriedade chave="Tipo" valor={consulta.manifesto.kind} mono />
                            <Propriedade chave="Versão" valor={consulta.manifesto.versao} mono />
                            <Propriedade
                                chave="Campos de configuração"
                                valor={String(resumoDoManifesto(consulta.manifesto).configuracao)}
                            />
                            <Propriedade
                                chave="Campos de segredo"
                                valor={String(resumoDoManifesto(consulta.manifesto).segredos)}
                            />
                        </dl>
                    ) : null}
                </div>
            </section>

            <SecaoCluster />

            <ExportarImportarOrganizacao key={orgId} org={orgId} />
        </div>
    );
}

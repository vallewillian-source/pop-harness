/**
 * Painel lateral do "por quê" de um turno do chat de teste (HR-47, UX_SPEC §3.7).
 *
 * Abre sobre a conversa (`SofaDrawer`: foco preso, `Esc` fecha, foco volta ao botão que abriu) e
 * mostra o que o turno gravou: decisões do Jev por gate, trechos e produtos com a fonte, ferramentas
 * chamadas, tempos por etapa, tokens e custo. Quando um dado não foi gravado, a tela diz isso em
 * vez de mostrar um número.
 *
 * Os dados vêm da leitura da conversa que a página já tem; a página recarrega essa leitura quando o
 * painel abre. Erro de leitura aparece no painel com "Tentar de novo" e não apaga o que já estava
 * na tela.
 */

import { SofaBanner, SofaButton, SofaDrawer, SofaStatusIndicator } from "@sofacoding/sofa-ui";
import {
    contarFerramentas,
    duracaoDoTurno,
    formatarConfianca,
    formatarConteudoDeFerramenta,
    formatarCustoMicros,
    formatarDuracaoDeFerramenta,
    formatarMargem,
    formatarMs,
    formatarPrecoCentavos,
    formatarTokens,
    lerChamadasDeFerramenta,
    lerFerramentasDoTurno,
    lerJevDoTurno,
    lerRecuperadosDoTurno,
    lerTemposDoTurno,
    rotuloDaOrigem,
    rotuloDoEstadoDoTurno,
    rotuloDoMotivoFim,
    rotuloDoMotivoRecusa,
    tituloDoGate,
    type ChamadaDeFerramenta,
    type DecisaoDoGate,
    type JevDoTurno,
    type RecuperadosDoTurno,
    type TemposDoTurno,
} from "../api/porQue";
import type { TurnoTeste } from "../api/testeChat";
import "../estilos/painel-porque.css";

type Props = {
    aberto: boolean;
    /** Posição do turno na conversa (1 é o primeiro). */
    numero: number | null;
    /** O turno, como a leitura da conversa o trouxe. `null`: ele não está na leitura atual. */
    turno: TurnoTeste | null;
    /** A leitura está sendo refeita. */
    atualizando: boolean;
    /** Motivo do erro da última leitura, quando ela falhou. */
    erroDeLeitura: string | null;
    aoFechar: () => void;
    aoTentarDeNovo: () => void;
};

function dataHora(ms: number): { texto: string; iso: string } {
    const data = new Date(ms);
    return {
        texto: data.toLocaleString("pt-BR", {
            day: "2-digit",
            month: "2-digit",
            year: "numeric",
            hour: "2-digit",
            minute: "2-digit",
            second: "2-digit",
        }),
        iso: data.toISOString(),
    };
}

function simNao(valor: boolean): string {
    return valor ? "sim" : "não";
}

export function PainelPorQue({
    aberto,
    numero,
    turno,
    atualizando,
    erroDeLeitura,
    aoFechar,
    aoTentarDeNovo,
}: Props) {
    const titulo = numero !== null ? `Por quê do turno ${numero}` : "Por quê do turno";
    return (
        <SofaDrawer
            open={aberto}
            onOpenChange={(proximo) => {
                if (!proximo) aoFechar();
            }}
            title={titulo}
            description="O que o agente usou para responder esta mensagem, como ficou gravado no turno."
            closeLabel="Fechar o painel do por quê"
            size="lg"
        >
            <div className="poph-porque">
                {erroDeLeitura && (
                    <SofaBanner tone="danger" title="Não consegui atualizar os dados deste turno">
                        <p>{erroDeLeitura}</p>
                        <SofaButton variant="secondary" onClick={aoTentarDeNovo} disabled={atualizando}>
                            Tentar de novo
                        </SofaButton>
                    </SofaBanner>
                )}
                {atualizando && (
                    <p className="poph-porque__atualizando" role="status">
                        Atualizando os dados do turno…
                    </p>
                )}
                {turno === null ? (
                    <p className="poph-porque__vazio">
                        Este turno não está na leitura atual da conversa. Atualize a conversa e abra de novo.
                    </p>
                ) : (
                    <ConteudoDoTurno turno={turno} numero={numero} />
                )}
            </div>
        </SofaDrawer>
    );
}

function ConteudoDoTurno({ turno, numero }: { turno: TurnoTeste; numero: number | null }) {
    const jev = lerJevDoTurno(turno.jev);
    const recuperados = lerRecuperadosDoTurno(turno.retrieved);
    const ferramentas = lerFerramentasDoTurno(turno.tools);
    const chamadas = lerChamadasDeFerramenta(turno.tool_calls);
    const tempos = lerTemposDoTurno(turno.timings);
    const inicio = dataHora(turno.started_at);
    const duracao = duracaoDoTurno(turno.started_at, turno.finished_at, tempos);

    return (
        <>
            <CabecalhoDoTurno
                numero={numero}
                turno={turno}
                inicio={inicio}
                duracao={duracao}
            />

            {turno.error && (
                <section className="poph-porque__secao" aria-labelledby="porque-erro">
                    <h3 id="porque-erro" className="poph-porque__titulo">
                        Erro registrado
                    </h3>
                    <p className="poph-porque__erro">{turno.error}</p>
                </section>
            )}

            <SecaoJev jev={jev} />
            <SecaoRecuperados recuperados={recuperados} />
            <SecaoFerramentas chamadas={chamadas} nomesAntigos={ferramentas} />
            <SecaoTempos tempos={tempos} />
        </>
    );
}

function CabecalhoDoTurno({
    numero,
    turno,
    inicio,
    duracao,
}: {
    numero: number | null;
    turno: TurnoTeste;
    inicio: { texto: string; iso: string };
    duracao: number | null;
}) {
    const entrada = turno.input_tokens ?? null;
    const saida = turno.output_tokens ?? null;
    // Zero nos dois lados é o padrão de quem não chegou a usar o modelo (ou não gravou o uso): não é
    // uma medição, então não entra como "0 tokens".
    const medido = (entrada ?? 0) + (saida ?? 0) > 0;
    const total = medido ? (entrada ?? 0) + (saida ?? 0) : null;
    return (
        <section className="poph-porque__secao" aria-labelledby="porque-cabecalho">
            <h3 id="porque-cabecalho" className="poph-porque__titulo">
                Resumo do turno
            </h3>
            <dl className="poph-porque__resumo">
                <div>
                    <dt>Turno</dt>
                    <dd>{numero !== null ? `#${numero}` : "não registrado"}</dd>
                </div>
                <div>
                    <dt>Estado</dt>
                    <dd>{rotuloDoEstadoDoTurno(turno.state)}</dd>
                </div>
                <div>
                    <dt>Início</dt>
                    <dd>
                        <time dateTime={inicio.iso}>{inicio.texto}</time>
                    </dd>
                </div>
                <div>
                    <dt>Duração</dt>
                    <dd>
                        {duracao !== null
                            ? formatarMs(duracao)
                            : turno.state === "running"
                              ? "em andamento"
                              : "não registrada"}
                    </dd>
                </div>
                <div>
                    <dt>Tokens do modelo</dt>
                    <dd>
                        {total === null
                            ? "não registrado"
                            : `${formatarTokens(total)} (entrada ${formatarTokens(entrada)}, saída ${formatarTokens(saida)})`}
                    </dd>
                </div>
                <div>
                    <dt>Custo</dt>
                    <dd>{formatarCustoMicros(turno.cost_micros ?? null)}</dd>
                </div>
            </dl>
        </section>
    );
}

function SecaoJev({ jev }: { jev: JevDoTurno | null }) {
    return (
        <section className="poph-porque__secao" aria-labelledby="porque-jev">
            <h3 id="porque-jev" className="poph-porque__titulo">
                Decisões do Jev por gate
            </h3>
            {jev === null ? (
                <p className="poph-porque__vazio">Nenhum gate decidiu neste turno.</p>
            ) : (
                <>
                    {jev.chamada && (
                        <dl className="poph-porque__linhas">
                            <div>
                                <dt>Chamada ao Jev</dt>
                                <dd>
                                    {jev.chamada.perguntas === null
                                        ? "não registrado"
                                        : `${jev.chamada.perguntas} pergunta(s), ${jev.chamada.decididasPorCache ?? 0} reaproveitada(s) do cache`}
                                </dd>
                            </div>
                            <div>
                                <dt>Tempo do lote</dt>
                                <dd>{formatarMs(jev.chamada.ms)}</dd>
                            </div>
                            <div>
                                <dt>Tokens do Jev</dt>
                                <dd>
                                    {jev.chamada.total !== null || jev.chamada.tokens !== null
                                        ? `${formatarTokens(jev.chamada.total ?? jev.chamada.tokens)}${
                                              jev.chamada.entrada !== null || jev.chamada.saida !== null
                                                  ? ` (entrada ${formatarTokens(jev.chamada.entrada)}, saída ${formatarTokens(jev.chamada.saida)})`
                                                  : ""
                                          }`
                                        : "não informados pelo Jev"}
                                </dd>
                            </div>
                            {jev.chamada.falha && (
                                <div>
                                    <dt>Falha da chamada</dt>
                                    <dd className="poph-porque__erro">{jev.chamada.falha}</dd>
                                </div>
                            )}
                        </dl>
                    )}

                    {jev.decisoes.length === 0 && (
                        <p className="poph-porque__vazio">A chamada não decidiu nenhum gate.</p>
                    )}
                    {jev.decisoes.map((decisao, indice) => (
                        <CartaoDoGate key={`${decisao.gate}-${indice}`} decisao={decisao} />
                    ))}

                    {jev.revisao && (
                        <article className="poph-porque__cartao" aria-labelledby="porque-g6">
                            <h4 id="porque-g6" className="poph-porque__subtitulo">
                                Revisão da resposta antes do envio (G6)
                            </h4>
                            <dl className="poph-porque__linhas">
                                <div>
                                    <dt>Escolha da revisão</dt>
                                    <dd>{jev.revisao.escolha ?? "não registrado"}</dd>
                                </div>
                                <div>
                                    <dt>Confiança</dt>
                                    <dd>{formatarConfianca(jev.revisao.confianca)}</dd>
                                </div>
                                <div>
                                    <dt>Reescreveu a resposta</dt>
                                    <dd>{simNao(jev.revisao.regenerou)}</dd>
                                </div>
                                {jev.revisao.segundaEscolha && (
                                    <div>
                                        <dt>Escolha da segunda revisão</dt>
                                        <dd>{jev.revisao.segundaEscolha}</dd>
                                    </div>
                                )}
                                <div>
                                    <dt>Enviou a resposta segura</dt>
                                    <dd>{simNao(jev.revisao.respostaSegura)}</dd>
                                </div>
                            </dl>
                        </article>
                    )}

                    <p className="poph-porque__nao-gravado">
                        Não gravado neste turno: o texto da pergunta enviada ao Jev e as probabilidades de cada
                        opção. O tempo e os tokens são do lote inteiro, então gates decididos no mesmo lote
                        mostram o mesmo valor.
                    </p>
                </>
            )}
        </section>
    );
}

function CartaoDoGate({ decisao }: { decisao: DecisaoDoGate }) {
    const id = `porque-gate-${decisao.gate}-${decisao.escolha ?? "sem"}`;
    return (
        <article className="poph-porque__cartao" aria-labelledby={id}>
            <h4 id={id} className="poph-porque__subtitulo">
                {tituloDoGate(decisao.gate)}
            </h4>
            <dl className="poph-porque__linhas">
                <div>
                    <dt>Escolha que valeu</dt>
                    <dd>{decisao.escolha ?? "nenhuma"}</dd>
                </div>
                <div>
                    <dt>Escolha do Jev</dt>
                    <dd>{decisao.escolhaDoJev ?? "não registrado"}</dd>
                </div>
                <div>
                    <dt>Origem</dt>
                    <dd>{rotuloDaOrigem(decisao.origem)}</dd>
                </div>
                <div>
                    <dt>Confiança</dt>
                    <dd>{formatarConfianca(decisao.confianca)}</dd>
                </div>
                <div>
                    <dt>Margem sobre a segunda opção</dt>
                    <dd>{formatarMargem(decisao.margem)}</dd>
                </div>
                {decisao.motivo && (
                    <div>
                        <dt>Motivo</dt>
                        <dd>{decisao.motivo}</dd>
                    </div>
                )}
                <div>
                    <dt>Tempo do lote</dt>
                    <dd>{formatarMs(decisao.ms)}</dd>
                </div>
                <div>
                    <dt>Tokens do lote</dt>
                    <dd>{formatarTokens(decisao.tokens)}</dd>
                </div>
            </dl>
        </article>
    );
}

function SecaoRecuperados({ recuperados }: { recuperados: RecuperadosDoTurno }) {
    const vazio = recuperados.trechos.length === 0 && recuperados.produtos.length === 0;
    return (
        <section className="poph-porque__secao" aria-labelledby="porque-recuperados">
            <h3 id="porque-recuperados" className="poph-porque__titulo">
                Base e catálogo usados na resposta
            </h3>
            {vazio ? (
                <p className="poph-porque__vazio">Nenhum trecho da base nem produto entrou na resposta deste turno.</p>
            ) : (
                <>
                    {recuperados.trechos.length > 0 && (
                        <>
                            <h4 className="poph-porque__subtitulo">Base de conhecimento</h4>
                            <ul className="poph-porque__lista">
                                {recuperados.trechos.map((trecho, indice) => (
                                    <li key={`${trecho.item ?? "trecho"}-${indice}`} className="poph-porque__item">
                                        <strong>{trecho.titulo}</strong>
                                        <span className="poph-porque__meta">
                                            Grupo: {trecho.grupo || "não registrado"} · Similaridade:{" "}
                                            {formatarConfianca(trecho.similaridade)} · Relevância (ranking):{" "}
                                            {trecho.relevancia === null
                                                ? "não registrada"
                                                : trecho.relevancia.toFixed(4).replace(".", ",")}
                                        </span>
                                        {trecho.texto && (
                                            <details>
                                                <summary>Ver o texto do trecho</summary>
                                                <p className="poph-porque__trecho">{trecho.texto}</p>
                                            </details>
                                        )}
                                    </li>
                                ))}
                            </ul>
                        </>
                    )}
                    {recuperados.produtos.length > 0 && (
                        <>
                            <h4 className="poph-porque__subtitulo">Catálogo de produtos</h4>
                            <ul className="poph-porque__lista">
                                {recuperados.produtos.map((produto, indice) => {
                                    const preco =
                                        produto.precoTexto ??
                                        formatarPrecoCentavos(produto.precoCentavos) ??
                                        "sem preço gravado";
                                    return (
                                        <li key={`${produto.produto ?? "produto"}-${indice}`} className="poph-porque__item">
                                            <strong>{produto.nome}</strong>
                                            <span className="poph-porque__meta">
                                                Preço que o cliente viu: {preco}
                                                {produto.unidade ? ` · ${produto.unidade}` : ""}
                                            </span>
                                            {produto.condicoes && (
                                                <span className="poph-porque__meta">Condições: {produto.condicoes}</span>
                                            )}
                                        </li>
                                    );
                                })}
                            </ul>
                        </>
                    )}
                </>
            )}
            <p className="poph-porque__nao-gravado">
                Não gravado neste turno: link para a fonte oficial e indicação de corte por trecho.
            </p>
        </section>
    );
}

function SecaoFerramentas({
    chamadas,
    nomesAntigos,
}: {
    chamadas: ChamadaDeFerramenta[];
    nomesAntigos: string[];
}) {
    // Turno gravado antes da HR-47b: tem `tools` (só os nomes) e nenhum detalhe por chamada.
    const soNomes = chamadas.length === 0 && nomesAntigos.length > 0;
    return (
        <section className="poph-porque__secao" aria-labelledby="porque-ferramentas">
            <h3 id="porque-ferramentas" className="poph-porque__titulo">
                Ferramentas chamadas
            </h3>
            {chamadas.length > 0 && (
                <ol className="poph-porque__lista" aria-label="Chamadas de ferramenta, na ordem em que aconteceram">
                    {chamadas.map((chamada, indice) => (
                        <ChamadaDaFerramenta key={`${chamada.rodada ?? "r"}-${chamada.ordem ?? indice}`} chamada={chamada} />
                    ))}
                </ol>
            )}
            {soNomes && (
                <>
                    <p className="poph-porque__nota">
                        Este turno foi gravado antes do registro detalhado: aparecem só os nomes. Estado, motivo,
                        duração, argumentos e resultado não foram gravados.
                    </p>
                    <ul className="poph-porque__lista">
                        {contarFerramentas(nomesAntigos).map((item) => (
                            <li key={item.nome} className="poph-porque__item">
                                <strong>{item.nome}</strong>
                                <span className="poph-porque__meta">
                                    {item.vezes === 1 ? "chamada 1 vez" : `chamada ${item.vezes} vezes`}
                                </span>
                            </li>
                        ))}
                    </ul>
                </>
            )}
            {chamadas.length === 0 && nomesAntigos.length === 0 && (
                <p className="poph-porque__vazio">Nenhuma ferramenta foi chamada neste turno.</p>
            )}
            <p className="poph-porque__nao-gravado">
                Não gravado neste turno: a estrutura completa do retorno de cada ferramenta. Fica o texto que voltou
                ao modelo, cortado em 1.024 caracteres quando passa disso.
            </p>
        </section>
    );
}

/** Uma chamada: executada e recusada têm estado com rótulo e cor diferentes, e a recusa mostra o motivo. */
function ChamadaDaFerramenta({ chamada }: { chamada: ChamadaDeFerramenta }) {
    const recusada = chamada.estado === "recusada";
    const rotuloDoEstado =
        chamada.estado === "executada" ? "Executada" : recusada ? "Recusada, não rodou" : "Estado não registrado";
    const tom = chamada.estado === "executada" ? "success" : recusada ? "attention" : "muted";
    const rodada = chamada.rodada === null ? "rodada não registrada" : `rodada ${chamada.rodada + 1}`;
    const ordem = chamada.ordem === null ? "" : `, chamada ${chamada.ordem}`;
    return (
        <li className={`poph-porque__item poph-porque__chamada${recusada ? " poph-porque__chamada--recusada" : ""}`}>
            <div className="poph-porque__cabecalho-chamada">
                <strong>{chamada.nome}</strong>
                <SofaStatusIndicator tone={tom}>{rotuloDoEstado}</SofaStatusIndicator>
            </div>
            <span className="poph-porque__meta">
                {rodada}
                {ordem}
            </span>
            {recusada && (
                <p className="poph-porque__motivo">
                    Motivo: {rotuloDoMotivoRecusa(chamada.motivo)}. O modelo recebeu o erro abaixo.
                </p>
            )}
            <dl className="poph-porque__linhas">
                <div>
                    <dt>Duração</dt>
                    <dd>{formatarDuracaoDeFerramenta(chamada.duracaoMs)}</dd>
                </div>
                <div>
                    <dt>Argumentos</dt>
                    <dd>
                        {chamada.argumentos === null ? (
                            "não registrados"
                        ) : (
                            <pre className="poph-porque__texto" tabIndex={0} aria-label={`Argumentos de ${chamada.nome}`}>
                                {formatarConteudoDeFerramenta(chamada.argumentos)}
                            </pre>
                        )}
                    </dd>
                </div>
                <div>
                    <dt>{recusada ? "Erro devolvido ao modelo" : "Resultado"}</dt>
                    <dd>
                        {chamada.resultado === null ? (
                            "não registrado"
                        ) : (
                            <pre className="poph-porque__texto" tabIndex={0} aria-label={`Resultado de ${chamada.nome}`}>
                                {formatarConteudoDeFerramenta(chamada.resultado)}
                            </pre>
                        )}
                    </dd>
                </div>
            </dl>
        </li>
    );
}

function SecaoTempos({ tempos }: { tempos: TemposDoTurno | null }) {
    if (tempos === null) {
        return (
            <section className="poph-porque__secao" aria-labelledby="porque-tempos">
                <h3 id="porque-tempos" className="poph-porque__titulo">
                    Tempos por etapa
                </h3>
                <p className="poph-porque__vazio">Os tempos não foram registrados neste turno.</p>
            </section>
        );
    }
    const total = tempos.totalMs;
    const etapas: Array<{ rotulo: string; ms: number | null }> = [
        { rotulo: "Fila", ms: tempos.filaMs },
        { rotulo: "Montagem do contexto", ms: tempos.montagemMs },
        { rotulo: "Até o primeiro texto", ms: tempos.primeiroTextoMs },
        { rotulo: "Total do turno", ms: total },
    ];
    return (
        <section className="poph-porque__secao" aria-labelledby="porque-tempos">
            <h3 id="porque-tempos" className="poph-porque__titulo">
                Tempos por etapa
            </h3>
            <ul className="poph-porque__lista">
                {etapas.map((etapa) => (
                    <li key={etapa.rotulo} className="poph-porque__item poph-porque__tempo">
                        <span>{etapa.rotulo}</span>
                        <span className="poph-porque__meta">{formatarMs(etapa.ms)}</span>
                        {etapa.ms !== null && total !== null && total > 0 && (
                            <progress
                                className="poph-porque__barra"
                                max={total}
                                value={Math.min(etapa.ms, total)}
                                aria-label={`${etapa.rotulo}: ${formatarMs(etapa.ms)} de ${formatarMs(total)}`}
                            />
                        )}
                    </li>
                ))}
            </ul>
            <dl className="poph-porque__linhas">
                <div>
                    <dt>Rodadas de ferramenta</dt>
                    <dd>{tempos.rodadasFerramenta ?? "não registrado"}</dd>
                </div>
                <div>
                    <dt>Modelo usado</dt>
                    <dd>{tempos.modelo ?? "não registrado"}</dd>
                </div>
                <div>
                    <dt>Motivo do fim</dt>
                    <dd>{rotuloDoMotivoFim(tempos.motivoFim)}</dd>
                </div>
            </dl>
            <p className="poph-porque__nao-gravado">
                Não gravado neste turno: o tempo da busca na base separado do resto da montagem.
            </p>
        </section>
    );
}

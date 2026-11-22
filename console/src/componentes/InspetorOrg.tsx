/**
 * Inspetor lateral da organização (PRD do re-layout §5.4, RL-06).
 *
 * Gaveta de 380 px à direita, sem modal: abre quando a linha da lista é selecionada, fecha com
 * `Esc` e com o "✕". Tudo o que aparece vem da lista já carregada (`Org`) ou de **uma** consulta
 * feita ao abrir (agentes da org). Porta, nó, banco, cota, canal de WhatsApp e gates G1–G8 não
 * entram: a API não entrega esses dados por organização (PRD §6 e §5.4).
 *
 * O comando de terminal foi conferido no binário (`poph --help` e `poph chat --help`): `--org` é
 * opção global e `chat` exige o agente como argumento posicional. Então o comando real é
 * `poph chat --org <slug-da-org> <slug-do-agente>`; enquanto os agentes não chegam (ou quando não
 * há agente ativo), o lugar do agente fica com o marcador `<agente>`, e não com um slug inventado.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { IconCheck, IconClose, IconCopy, SofaButton, SofaIconButton } from "@sofacoding/sofa-ui";
import { listarAgentes, type Agente } from "../api/agentes";
import type { Org } from "../api/cliente";
import "../estilos/inspetor.css";

export type InspetorOrgProps = {
    /** Organização selecionada na lista; o inspetor mostra sempre esta. */
    org: Org;
    /** Fecha a gaveta (o dono da tela devolve o foco à linha). */
    aoFechar: () => void;
    /** Abre o painel da organização; passa pela guarda de alterações não salvas. */
    aoAcessar: (org: Org) => void;
};

/** Estado da consulta de agentes: carrega uma vez por org aberta, sem travar o resto da gaveta. */
type Recurso =
    | { estado: "carregando" }
    | { estado: "pronto"; agentes: Agente[] }
    | { estado: "erro"; mensagem: string };

function rotuloDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return "Ativa";
        case "inactive":
            return "Inativa";
        case "archived":
            return "Arquivada";
        default:
            return situacao;
    }
}

/** Retenção vem da configuração da org, que é JSON livre (mesma regra da lista). */
function retencao(org: Org): string {
    const config = org.config as Record<string, unknown> | undefined;
    const valor = config?.["retention_days"] ?? config?.["retencao_dias"];
    if (typeof valor === "number") {
        return valor === 0 ? "nunca apaga" : `${valor} dias`;
    }
    return "padrão";
}

function dataCurta(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    return new Date(ms).toLocaleString("pt-BR", {
        day: "2-digit",
        month: "2-digit",
        year: "numeric",
    });
}

/** Falha de consulta em texto curto, para a linha de Agentes. */
function mensagemDoErro(falha: unknown): string {
    if (falha instanceof Error && falha.message) return falha.message;
    return "falha inesperada ao consultar os agentes";
}

export function InspetorOrg({ org, aoFechar, aoAcessar }: InspetorOrgProps) {
    const [recurso, setRecurso] = useState<Recurso>({ estado: "carregando" });
    const [copiado, setCopiado] = useState(false);
    /** Falha do clipboard, quando não há como copiar (degradação honesta). */
    const [falhaAoCopiar, setFalhaAoCopiar] = useState<string | null>(null);
    const relogioDoCopiado = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Uma consulta ao abrir (e ao trocar a org), isolada: o resto da gaveta aparece na hora.
    useEffect(() => {
        let cancelado = false;
        setRecurso({ estado: "carregando" });
        listarAgentes(org.id)
            .then((agentes) => {
                if (!cancelado) setRecurso({ estado: "pronto", agentes });
            })
            .catch((falha: unknown) => {
                if (!cancelado) setRecurso({ estado: "erro", mensagem: mensagemDoErro(falha) });
            });
        return () => {
            cancelado = true;
        };
    }, [org.id]);

    // A seleção mudou: o "Copiado" pertence à organização anterior.
    useEffect(() => {
        setCopiado(false);
        setFalhaAoCopiar(null);
    }, [org.id]);

    const fechar = useCallback(() => aoFechar(), [aoFechar]);

    // `Esc` fecha a gaveta, de onde o foco estiver. Menus e diálogos abertos consomem o `Esc`
    // primeiro (o deles fecha antes); se um deles já tratou a tecla, aqui não se faz nada.
    useEffect(() => {
        const aoTeclar = (evento: KeyboardEvent) => {
            if (evento.key !== "Escape" || evento.defaultPrevented) return;
            const alvo = evento.target as HTMLElement | null;
            if (alvo?.closest?.('[role="menu"], [role="dialog"], [role="listbox"]')) return;
            evento.preventDefault();
            fechar();
        };
        window.addEventListener("keydown", aoTeclar);
        return () => window.removeEventListener("keydown", aoTeclar);
    }, [fechar]);

    useEffect(
        () => () => {
            if (relogioDoCopiado.current !== null) clearTimeout(relogioDoCopiado.current);
        },
        [],
    );

    const agentes = recurso.estado === "pronto" ? recurso.agentes : [];
    const primeiroAtivo = agentes.find((agente) => agente.status === "active") ?? null;
    const comando = `poph chat --org ${org.slug} ${primeiroAtivo?.slug ?? "<agente>"}`;

    async function copiar() {
        setFalhaAoCopiar(null);
        try {
            if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
                throw new Error("clipboard indisponível");
            }
            await navigator.clipboard.writeText(comando);
        } catch {
            setCopiado(false);
            setFalhaAoCopiar("Não consegui copiar. Selecione o comando e copie à mão.");
            return;
        }
        setCopiado(true);
        if (relogioDoCopiado.current !== null) clearTimeout(relogioDoCopiado.current);
        relogioDoCopiado.current = setTimeout(() => setCopiado(false), 4000);
    }

    return (
        <aside
            className="poph-gaveta poph-inspetor"
            role="complementary"
            aria-label={`Detalhes de ${org.name}`}
        >
            <div className="poph-gaveta__topo">
                <div className="poph-inspetor__identidade">
                    <h2 className="poph-gaveta__titulo">{org.name}</h2>
                    <p className="poph-gaveta__slug">{org.slug}</p>
                </div>
                <SofaIconButton label="Fechar inspetor" size={28} onClick={fechar}>
                    <IconClose aria-hidden="true" />
                </SofaIconButton>
            </div>

            <div className="poph-gaveta__corpo">
                <div className="poph-inspetor__acoes">
                    <SofaButton variant="primary" onClick={() => aoAcessar(org)}>
                        Acessar organização →
                    </SofaButton>
                </div>

                <section className="poph-secao">
                    <h3 className="poph-secao__titulo">Propriedades do tenant</h3>
                    <dl className="poph-propriedades">
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">Identificador</dt>
                            <dd className="poph-propriedade__valor poph-propriedade__valor--mono">
                                {org.id}
                            </dd>
                        </div>
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">Situação</dt>
                            <dd className="poph-propriedade__valor">
                                {rotuloDaSituacao(org.status)}
                            </dd>
                        </div>
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">Versão</dt>
                            <dd className="poph-propriedade__valor poph-propriedade__valor--mono">
                                v{org.version}
                            </dd>
                        </div>
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">Retenção do histórico</dt>
                            <dd className="poph-propriedade__valor">{retencao(org)}</dd>
                        </div>
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">Criada em</dt>
                            <dd className="poph-propriedade__valor">{dataCurta(org.created_at)}</dd>
                        </div>
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">Atualizada em</dt>
                            <dd className="poph-propriedade__valor">{dataCurta(org.updated_at)}</dd>
                        </div>
                    </dl>
                </section>

                <section className="poph-secao">
                    <h3 className="poph-secao__titulo">Recursos ativos</h3>
                    <dl className="poph-propriedades">
                        <div className="poph-propriedade">
                            <dt className="poph-propriedade__chave">Agentes</dt>
                            <dd className="poph-propriedade__valor">
                                {recurso.estado === "carregando" && (
                                    <span className="poph-inspetor__recurso">carregando…</span>
                                )}
                                {recurso.estado === "erro" && (
                                    <span
                                        className="poph-inspetor__recurso poph-inspetor__recurso--erro"
                                        title={recurso.mensagem}
                                    >
                                        não consegui carregar
                                    </span>
                                )}
                                {recurso.estado === "pronto" && (
                                    <span className="poph-inspetor__recurso">
                                        {agentes.length === 0
                                            ? "nenhum agente"
                                            : `${agentes.length} ${
                                                  agentes.length === 1 ? "agente" : "agentes"
                                              }`}
                                        {agentes.length > 0 &&
                                            (primeiroAtivo === null
                                                ? " · nenhum ativo"
                                                : ` · ${primeiroAtivo.name}`)}
                                    </span>
                                )}
                            </dd>
                        </div>
                    </dl>
                </section>

                <section className="poph-secao">
                    <h3 className="poph-secao__titulo">Comando CLI de teste</h3>
                    <div className="poph-terminal">
                        <code className="poph-terminal__comando">{comando}</code>
                        <SofaIconButton
                            label="Copiar comando"
                            size={24}
                            className="poph-terminal__copiar"
                            onClick={() => void copiar()}
                        >
                            <IconCopy size={13} aria-hidden="true" />
                        </SofaIconButton>
                    </div>
                    <p className="poph-inspetor__dica">
                        Conversa de teste pelo terminal. Quando não há agente ativo, troque{" "}
                        <code>&lt;agente&gt;</code> pelo slug do agente.
                    </p>
                    <p
                        className={[
                            "poph-inspetor__aviso",
                            copiado || falhaAoCopiar ? "poph-inspetor__aviso--visivel" : "",
                            falhaAoCopiar ? "poph-inspetor__aviso--erro" : "",
                        ]
                            .filter(Boolean)
                            .join(" ")}
                        role="status"
                        aria-live="polite"
                    >
                        {copiado ? (
                            <>
                                <IconCheck aria-hidden="true" /> Copiado
                            </>
                        ) : (
                            falhaAoCopiar ?? ""
                        )}
                    </p>
                </section>
            </div>

            <div className="poph-gaveta__rodape">
                Pressione <kbd>Esc</kbd> para fechar o inspetor.
            </div>
        </aside>
    );
}

/**
 * Abas do agente (UX_SPEC §3.5) e cabeçalho do agente, compartilhados pelas telas de agente.
 *
 * Re-layout (RL-09): as abas são texto com sublinhado âmbar na ativa, como em `relayout.css`
 * (`.poph-abas` / `.poph-aba--ativa`), no lugar do trocador da biblioteca. A navegação continua
 * passando pela guarda de alterações não salvas: sair de uma aba com formulário sujo abre o aviso
 * em vez de descartar em silêncio.
 *
 * Teclado (WAI-ARIA, ativação manual): `Tab` entra na aba ativa; `←`/`→`/`Home`/`End` movem o foco
 * entre as abas; `Enter`/`Espaço` trocam. As setas não navegam sozinhas, para que percorrer a
 * lista não abandone uma aba com edição pendente.
 *
 * `CabecalhoDoAgente` é o cabeçalho comum das abas (nome, situação, organização) na linguagem do
 * painel da organização (RL-07). Ele recebe os dados que a aba já carregou — não faz consulta
 * própria, para não criar chamada nova à API. As telas das abas (RL-10, RL-11 e RL-12) trocam o
 * cabeçalho antigo por este.
 */

import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { IconArrowLeft, SofaIconButton } from "@sofacoding/sofa-ui";
import { useGuardaDeAlteracoes } from "./GuardaAlteracoes";
import { publicarNomeDoAgente } from "./nomeDoAgente";
import "../estilos/agentes.css";

export type IdAbaDoAgente =
    | "instructions"
    | "knowledge-base"
    | "files"
    | "routing"
    | "integrations"
    | "model-jev"
    | "test-chat";

type Aba = {
    id: IdAbaDoAgente;
    label: string;
    /** Sufixo da rota dentro de `/orgs/:org/agents/:agent/` (todas existem no `App.tsx`). */
    rota: string;
};

/** Ordem do PRD §5.5 / TASKS RL-09. A aba "Produtos" saiu: não tem rota no `App.tsx`. */
export const ABAS_DO_AGENTE: readonly Aba[] = [
    { id: "instructions", label: "Instruções", rota: "instructions" },
    { id: "knowledge-base", label: "Base de conhecimento", rota: "knowledge-base" },
    { id: "files", label: "Arquivos", rota: "files" },
    { id: "routing", label: "Roteamento", rota: "routing" },
    { id: "integrations", label: "Integrações", rota: "integrations" },
    { id: "model-jev", label: "Modelo e Jev", rota: "model-jev" },
    { id: "test-chat", label: "Chat de teste", rota: "test-chat" },
] as const;

export type AbasDoAgenteProps = {
    org: string;
    agente: string;
    ativa: IdAbaDoAgente;
    /** Nome do agente, para a trilha da barra superior (HR-49b). Cada aba já tem o agente carregado. */
    nome?: string;
};

export function AbasDoAgente({ org, agente, ativa, nome }: AbasDoAgenteProps) {
    const guarda = useGuardaDeAlteracoes();
    const refs = useRef<Array<HTMLButtonElement | null>>([]);

    useEffect(() => {
        if (nome) publicarNomeDoAgente(agente, nome);
    }, [agente, nome]);

    function trocar(id: IdAbaDoAgente) {
        const aba = ABAS_DO_AGENTE.find((candidata) => candidata.id === id);
        if (!aba || id === ativa) return;
        guarda.tentarNavegar(
            `/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/${aba.rota}`,
        );
    }

    /** Move o foco (sem trocar de aba): a troca é `Enter`/`Espaço`, como num botão. */
    function moverFoco(indice: number, evento: KeyboardEvent<HTMLButtonElement>) {
        const ultimo = ABAS_DO_AGENTE.length - 1;
        let destino: number;
        switch (evento.key) {
            case "ArrowRight":
                destino = indice === ultimo ? 0 : indice + 1;
                break;
            case "ArrowLeft":
                destino = indice === 0 ? ultimo : indice - 1;
                break;
            case "Home":
                destino = 0;
                break;
            case "End":
                destino = ultimo;
                break;
            default:
                return;
        }
        evento.preventDefault();
        refs.current[destino]?.focus();
    }

    return (
        <div className="poph-abas" role="tablist" aria-label="Seções do agente">
            {ABAS_DO_AGENTE.map((aba, indice) => {
                const selecionada = aba.id === ativa;
                return (
                    <button
                        key={aba.id}
                        type="button"
                        role="tab"
                        aria-selected={selecionada}
                        aria-current={selecionada ? "page" : undefined}
                        tabIndex={selecionada ? 0 : -1}
                        className={selecionada ? "poph-aba poph-aba--ativa" : "poph-aba"}
                        ref={(no) => {
                            refs.current[indice] = no;
                        }}
                        onClick={() => trocar(aba.id)}
                        onKeyDown={(evento) => moverFoco(indice, evento)}
                    >
                        {aba.label}
                    </button>
                );
            })}
        </div>
    );
}

function tomDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return "poph-status--ativo";
        case "inactive":
        case "archived":
            return "poph-status--inativo";
        default:
            return "poph-status--inativo";
    }
}

function rotuloDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return "Ativo";
        case "inactive":
            return "Inativo";
        case "archived":
            return "Arquivado";
        default:
            return situacao;
    }
}

export type CabecalhoDoAgenteProps = {
    org: string;
    nome: string;
    slug?: string;
    situacao?: string;
    /** Nome da organização, quando a tela já o carregou. */
    organizacao?: string | null;
    /** Linha de contexto da aba ("modelo, busca na base e portões…"), como hoje. */
    descricao?: ReactNode;
    /** Ações da aba (Salvar, Desfazer…), no mesmo lugar de hoje. */
    acoes?: ReactNode;
};

/**
 * Cabeçalho comum das abas do agente: voltar, organização, nome, situação (ponto de 6 px + texto)
 * e as ações da aba à direita. Não busca nada: quem usa passa os dados que já tem.
 */
export function CabecalhoDoAgente({
    org,
    nome,
    slug,
    situacao,
    organizacao,
    descricao,
    acoes,
}: CabecalhoDoAgenteProps) {
    const guarda = useGuardaDeAlteracoes();

    return (
        <div className="poph-pagina__topo poph-agente__topo">
            <div className="poph-agente__identidade">
                <SofaIconButton
                    label="Voltar para agentes"
                    onClick={() =>
                        guarda.tentarNavegar(
                            `/orgs/${encodeURIComponent(org)}/agents`,
                        )
                    }
                >
                    <IconArrowLeft aria-hidden="true" />
                </SofaIconButton>
                <div className="poph-agente__texto">
                    <p className="poph-agente__contexto">
                        {organizacao ? `Agente · ${organizacao}` : "Agente"}
                    </p>
                    <h1 className="poph-pagina__titulo">{nome}</h1>
                    <p className="poph-pagina__descricao poph-agente__meta">
                        {situacao && (
                            <span className={`poph-status ${tomDaSituacao(situacao)}`}>
                                <span className="poph-status__ponto" aria-hidden="true" />
                                {rotuloDaSituacao(situacao)}
                            </span>
                        )}
                        {slug && <span className="poph-mono-mudo">{slug}</span>}
                        {descricao && <span>{descricao}</span>}
                    </p>
                </div>
            </div>
            {acoes && <div className="poph-pagina__acoes">{acoes}</div>}
        </div>
    );
}

export type SecaoDoAgenteProps = {
    titulo: string;
    /** Frase curta logo abaixo do título, como nas outras telas do Console. */
    descricao?: ReactNode;
    /** Ações da seção (ex.: Atualizar, Adicionar), à direita do título. */
    acoes?: ReactNode;
    children: ReactNode;
};

/**
 * Seção plana das telas de agente (RL-10): título em caixa alta, frase curta e ações, com a linha
 * fina de 1 px separando da seção anterior. No lugar do cartão (`SofaCard`), que não é usado aqui.
 */
export function SecaoDoAgente({ titulo, descricao, acoes, children }: SecaoDoAgenteProps) {
    const idDoTitulo = useId();
    return (
        <section className="poph-secao poph-agente__secao" aria-labelledby={idDoTitulo}>
            <div className="poph-agente__secao-cabecalho">
                <div className="poph-agente__secao-texto">
                    <h2 id={idDoTitulo} className="poph-secao__titulo">
                        {titulo}
                    </h2>
                    {descricao && <p className="poph-agente__secao-ajuda">{descricao}</p>}
                </div>
                {acoes && <div className="poph-agente__secao-acoes">{acoes}</div>}
            </div>
            <div className="poph-secao__conteudo">{children}</div>
        </section>
    );
}

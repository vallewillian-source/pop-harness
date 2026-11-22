/**
 * Barra superior do Console (PRD do re-layout §5.1, RL-02 e RL-04).
 *
 * Esquerda: monograma + nome da marca e a trilha de navegação. Direita, nesta ordem: busca rápida
 * de organizações (RL-04), estado da instância em texto ("● Local :porta", ou "Sem conexão" quando
 * a consulta de saúde falha), escopo do token, botão de sair e, na extremidade, o botão de ajuda
 * que abre os atalhos (HR-61). Saíram o selo "DEV LOCAL" e o indicador de saúde duplicado; o
 * ambiente continua legível no próprio texto do estado.
 *
 * As classes próprias deste header vivem em `src/estilos/topbar.css` e as da busca em
 * `src/estilos/busca.css`, importados abaixo para a tarefa não precisar mexer em `main.tsx`.
 */

import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { useNavigate } from "react-router-dom";
import { IconLock, IconLogOut, SofaIconButton, type SofaIconProps } from "@sofacoding/sofa-ui";
import { useSessao } from "../sessao/Sessao";
import { useGuardaDeAlteracoes } from "./GuardaAlteracoes";
import { filtrarOrgs } from "./busca";
import { useSaude } from "./saude";
import { Trilha } from "./Trilha";
import "../estilos/topbar.css";
import "../estilos/busca.css";

/** Endereço desta instância para exibição: "Local :porta" quando a página está na máquina. */
export function enderecoDaInstancia(): string {
    const { hostname, port } = window.location;
    if (hostname === "127.0.0.1" || hostname === "localhost" || hostname === "") {
        return port ? `Local :${port}` : "Local";
    }
    return hostname;
}

/** Campo que recebe digitação. Nele, Ctrl/⌘+K fica com quem estiver digitando. */
function estaDigitando(alvo: EventTarget | null): boolean {
    if (!(alvo instanceof HTMLElement)) return false;
    return (
        alvo instanceof HTMLInputElement ||
        alvo instanceof HTMLTextAreaElement ||
        alvo instanceof HTMLSelectElement ||
        alvo.isContentEditable
    );
}

/**
 * Busca rápida de organizações da sessão (RL-04). Ctrl/⌘+K foca o campo; digitar filtra por nome
 * ou slug; setas escolhem, Enter abre `/orgs/:id` pela guarda de alterações; Esc limpa e fecha.
 */
function BuscaDeOrgs() {
    const { sessao } = useSessao();
    const guarda = useGuardaDeAlteracoes();
    const campo = useRef<HTMLInputElement>(null);
    const idBase = useId();
    const [termo, setTermo] = useState("");
    const [aberta, setAberta] = useState(false);
    const [indice, setIndice] = useState(0);

    const resultados = filtrarOrgs(sessao?.orgs ?? [], termo);
    const mostrarLista = aberta && termo.trim() !== "";
    const indiceAtivo = resultados.length === 0 ? -1 : Math.min(indice, resultados.length - 1);
    const idCampo = `${idBase}-campo`;
    const idLista = `${idBase}-lista`;
    const idOpcao = (posicao: number) => `${idBase}-opcao-${posicao}`;

    useEffect(() => {
        const aoTeclar = (evento: globalThis.KeyboardEvent) => {
            if (!(evento.ctrlKey || evento.metaKey) || evento.altKey || evento.shiftKey) return;
            if (evento.key.toLowerCase() !== "k") return;
            const input = campo.current;
            if (!input) return;
            // Em outro campo de texto a combinação é dele: a busca não rouba o foco.
            if (evento.target !== input && estaDigitando(evento.target)) return;
            evento.preventDefault();
            input.focus();
            input.select();
        };
        window.addEventListener("keydown", aoTeclar);
        return () => window.removeEventListener("keydown", aoTeclar);
    }, []);

    /** Abre a organização pela guarda: com alterações não salvas, o aviso aparece e nada se perde. */
    function abrirOrg(org: { id: string }) {
        const foi = guarda.tentarNavegar(`/orgs/${org.id}`);
        setAberta(false);
        if (foi) setTermo("");
    }

    function aoTeclar(evento: KeyboardEvent<HTMLInputElement>) {
        if (evento.key === "ArrowDown" || evento.key === "ArrowUp") {
            if (resultados.length === 0) return;
            evento.preventDefault();
            setAberta(true);
            const passo = evento.key === "ArrowDown" ? 1 : -1;
            const total = resultados.length;
            setIndice(((indiceAtivo + passo) % total + total) % total);
            return;
        }
        if (evento.key === "Enter") {
            const escolhida = resultados[indiceAtivo];
            if (!mostrarLista || !escolhida) return;
            evento.preventDefault();
            abrirOrg(escolhida);
            return;
        }
        if (evento.key === "Escape") {
            if (termo === "") return;
            evento.preventDefault();
            setTermo("");
            setAberta(false);
        }
    }

    return (
        <div className="poph-busca">
            <label htmlFor={idCampo} className="poph-busca__rotulo-oculto">
                Buscar organização
            </label>
            <input
                id={idCampo}
                ref={campo}
                type="text"
                role="combobox"
                className="poph-busca__campo"
                placeholder="Buscar… ⌘K"
                autoComplete="off"
                spellCheck={false}
                value={termo}
                aria-expanded={mostrarLista}
                aria-controls={mostrarLista ? idLista : undefined}
                aria-autocomplete="list"
                aria-activedescendant={
                    mostrarLista && indiceAtivo >= 0 ? idOpcao(indiceAtivo) : undefined
                }
                onChange={(evento) => {
                    setTermo(evento.target.value);
                    setIndice(0);
                    setAberta(true);
                }}
                onFocus={() => setAberta(true)}
                onBlur={() => setAberta(false)}
                onKeyDown={aoTeclar}
            />
            {mostrarLista && (
                <div
                    id={idLista}
                    className="poph-busca__lista"
                    role={resultados.length > 0 ? "listbox" : undefined}
                    aria-label={resultados.length > 0 ? "Organizações encontradas" : undefined}
                >
                    {resultados.length === 0 ? (
                        <p className="poph-busca__vazio" role="status">
                            Nenhuma organização encontrada
                        </p>
                    ) : (
                        resultados.map((org, posicao) => {
                            const ativa = posicao === indiceAtivo;
                            return (
                                <div
                                    key={org.id}
                                    id={idOpcao(posicao)}
                                    role="option"
                                    aria-selected={ativa}
                                    className={
                                        ativa
                                            ? "poph-busca__opcao poph-busca__opcao--ativa"
                                            : "poph-busca__opcao"
                                    }
                                    // Mantém o foco no campo ao clicar na opção.
                                    onMouseDown={(evento) => evento.preventDefault()}
                                    onClick={() => abrirOrg(org)}
                                >
                                    <span>{org.name}</span>
                                    <span className="poph-busca__slug">{org.slug}</span>
                                </div>
                            );
                        })
                    )}
                </div>
            )}
        </div>
    );
}

/**
 * Ícone de ajuda (interrogação num círculo). A biblioteca não tem um; ele fica aqui, no mesmo traço
 * dos ícones da SofaUI (24 px, traço de 1.8).
 */
function IconeAjuda(props: SofaIconProps) {
    return (
        <svg
            viewBox="0 0 24 24"
            width="16"
            height="16"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
            {...props}
        >
            <circle cx="12" cy="12" r="8.5" />
            <path d="M9.6 9.5a2.5 2.5 0 1 1 3.4 2.3c-.6.3-1 .8-1 1.5v.4" />
            <path d="M12 16.9h.01" />
        </svg>
    );
}

export function Topbar({
    orgs,
    aoAbrirAtalhos,
}: {
    orgs: { id: string; name: string }[];
    /** Abre o modal "Atalhos do Console" (o mesmo do botão "? atalhos" do menu). */
    aoAbrirAtalhos: () => void;
}) {
    const { sessao, sair } = useSessao();
    const navegar = useNavigate();
    const { semConexao } = useSaude();
    const escopoDoToken = sessao?.escopo === "org" ? "Token de org" : "Token global";

    return (
        <header className="poph-topbar">
            <div className="poph-topbar__marca">
                <span className="poph-topbar__monograma" aria-hidden="true">
                    P
                </span>
                <span className="poph-topbar__nome">POP HARNESS</span>
            </div>
            <span className="poph-topbar__divisor" aria-hidden="true">
                /
            </span>
            <Trilha orgs={orgs} />
            <div className="poph-topbar__acoes">
                <BuscaDeOrgs />
                <span
                    className={
                        semConexao
                            ? "poph-topbar__estado poph-topbar__estado--sem"
                            : "poph-topbar__estado"
                    }
                    aria-live="polite"
                >
                    <span className="poph-topbar__ponto" aria-hidden="true" />
                    <span className="poph-topbar__estado-texto">
                        {semConexao ? "Sem conexão" : enderecoDaInstancia()}
                    </span>
                </span>
                <span className="poph-topbar__token">
                    <IconLock aria-hidden="true" />
                    {escopoDoToken}
                </span>
                <SofaIconButton
                    label="Sair do Console"
                    onClick={() => {
                        sair();
                        navegar("/login", { replace: true });
                    }}
                >
                    <IconLogOut aria-hidden="true" />
                </SofaIconButton>
                <SofaIconButton label="Atalhos do teclado" size={28} onClick={aoAbrirAtalhos}>
                    <IconeAjuda aria-hidden="true" />
                </SofaIconButton>
            </div>
        </header>
    );
}

/**
 * Barra superior do Console (PRD do re-layout §5.1, RL-02, RL-04 e CF-04).
 *
 * Esquerda: monograma, nome da marca, versão da instância e a trilha de navegação. Centro: a busca
 * global (organizações, telas do Console e comandos do `poph`), com o atalho ⌘/Ctrl+K visível.
 * Direita, nesta ordem: estado da instância ("Engine: 200 OK · 125 MB", ou "Sem conexão"), escopo
 * do token, botão de sair e, na extremidade, o botão de ajuda que abre os atalhos (HR-61).
 *
 * O endereço da instância e o escopo do token ficam no `title` do indicador de estado; o escopo
 * também segue visível como selo, como antes.
 *
 * As classes próprias deste header vivem em `src/estilos/topbar.css` e as da busca em
 * `src/estilos/busca.css`, importados abaixo para a tarefa não precisar mexer em `main.tsx`.
 */

import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import {
    IconLock,
    IconLogOut,
    IconSearch,
    SofaIconButton,
    type SofaIconProps,
} from "@sofacoding/sofa-ui";
import { useSessao } from "../sessao/Sessao";
import { useGuardaDeAlteracoes } from "./GuardaAlteracoes";
import {
    buscarTudo,
    GRUPOS_DA_BUSCA,
    type OpcaoDeBusca,
    type TelaBuscavel,
} from "./busca";
import { ITENS_DE_NAVEGACAO, type ContextoDeNavegacao } from "./navegacao";
import { useMemoriaDoProcesso, useSaude } from "./saude";
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

/** Texto do atalho de busca no sistema da pessoa: ⌘K no Mac, Ctrl K nos demais. */
function textoDoAtalho(): string {
    if (typeof navigator === "undefined") return "Ctrl K";
    return /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘K" : "Ctrl K";
}

/**
 * Contexto da URL atual, do mesmo jeito que o menu lateral lê: a organização e o agente abertos
 * decidem quais telas a busca pode oferecer.
 */
function contextoDaUrl(pathname: string): ContextoDeNavegacao {
    const partes = pathname.split("/").filter(Boolean);
    const orgAberta = partes[0] === "orgs" ? (partes[1] ?? "") : "";
    const agenteAberto = orgAberta && partes[2] === "agents" ? (partes[3] ?? "") : "";
    return {
        orgId: orgAberta || null,
        agentId: agenteAberto || null,
        totalDeOrgs: null,
        totalDeAgentes: null,
    };
}

/** Telas do menu que existem para o contexto da URL (as que estão ativas e têm rota resolvida). */
function telasDisponiveis(contexto: ContextoDeNavegacao): TelaBuscavel[] {
    return ITENS_DE_NAVEGACAO.flatMap((item) => {
        if (item.estado !== "ativo") return [];
        const caminho = item.rota(contexto);
        return caminho ? [{ rotulo: item.rotulo, caminho }] : [];
    });
}

/** Tempo que o aviso de "comando copiado" fica na tela. */
const DURACAO_DO_AVISO_MS = 3000;

/**
 * Busca global do header (RL-04, CF-04). Ctrl/⌘+K foca o campo; digitar procura em organizações,
 * telas e comandos do `poph`; setas percorrem as opções de todos os grupos; Enter abre a
 * organização ou a tela pela guarda de alterações, ou copia o comando; Esc limpa e fecha.
 */
function BuscaGlobal({ telas }: { telas: TelaBuscavel[] }) {
    const { sessao } = useSessao();
    const guarda = useGuardaDeAlteracoes();
    const campo = useRef<HTMLInputElement>(null);
    const idBase = useId();
    const [termo, setTermo] = useState("");
    const [aberta, setAberta] = useState(false);
    const [indice, setIndice] = useState(0);
    const [aviso, setAviso] = useState("");
    const atalho = textoDoAtalho();

    const opcoes: OpcaoDeBusca[] = buscarTudo(termo, sessao?.orgs ?? [], telas);
    const mostrarLista = aberta && termo.trim() !== "";
    const indiceAtivo = opcoes.length === 0 ? -1 : Math.min(indice, opcoes.length - 1);
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

    // O aviso de comando copiado some sozinho; ao trocar de aviso, o prazo recomeça.
    useEffect(() => {
        if (aviso === "") return;
        const id = window.setTimeout(() => setAviso(""), DURACAO_DO_AVISO_MS);
        return () => window.clearTimeout(id);
    }, [aviso]);

    /** Caminho pela guarda: com alterações não salvas, o aviso aparece e nada se perde. */
    function irPara(caminho: string) {
        const foi = guarda.tentarNavegar(caminho);
        setAberta(false);
        if (foi) setTermo("");
    }

    /** Copia o comando do `poph` para a área de transferência, com aviso de sucesso ou de falha. */
    function copiarComando(texto: string) {
        setAberta(false);
        setTermo("");
        if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
            setAviso(`Não consegui copiar. Comando: ${texto}`);
            return;
        }
        navigator.clipboard.writeText(texto).then(
            () => setAviso(`Comando copiado: ${texto}`),
            () => setAviso(`Não consegui copiar. Comando: ${texto}`),
        );
    }

    function executar(opcao: OpcaoDeBusca) {
        if (opcao.destino.tipo === "caminho") {
            irPara(opcao.destino.caminho);
        } else {
            copiarComando(opcao.destino.texto);
        }
    }

    function aoTeclar(evento: KeyboardEvent<HTMLInputElement>) {
        if (evento.key === "ArrowDown" || evento.key === "ArrowUp") {
            if (opcoes.length === 0) return;
            evento.preventDefault();
            setAberta(true);
            const passo = evento.key === "ArrowDown" ? 1 : -1;
            const total = opcoes.length;
            setIndice(((indiceAtivo + passo) % total + total) % total);
            return;
        }
        if (evento.key === "Enter") {
            const escolhida = opcoes[indiceAtivo];
            if (!mostrarLista || !escolhida) return;
            evento.preventDefault();
            executar(escolhida);
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
                Buscar organizações, telas e comandos
            </label>
            <span className="poph-busca__lupa" aria-hidden="true">
                <IconSearch />
            </span>
            <input
                id={idCampo}
                ref={campo}
                type="text"
                role="combobox"
                className="poph-busca__campo"
                placeholder="Buscar organizações, rotas ou comandos globais…"
                autoComplete="off"
                spellCheck={false}
                value={termo}
                aria-expanded={mostrarLista}
                aria-controls={mostrarLista ? idLista : undefined}
                aria-autocomplete="list"
                aria-keyshortcuts="Control+K Meta+K"
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
            <kbd className="poph-busca__atalho" aria-hidden="true">
                {atalho}
            </kbd>
            {mostrarLista && (
                <div
                    id={idLista}
                    className="poph-busca__lista"
                    role={opcoes.length > 0 ? "listbox" : undefined}
                    aria-label={opcoes.length > 0 ? "Resultados da busca" : undefined}
                >
                    {opcoes.length === 0 ? (
                        <p className="poph-busca__vazio" role="status">
                            Nada encontrado para “{termo.trim()}”
                        </p>
                    ) : (
                        GRUPOS_DA_BUSCA.map((grupo) => {
                            const doGrupo = opcoes
                                .map((opcao, posicao) => ({ opcao, posicao }))
                                .filter(({ opcao }) => opcao.grupo === grupo);
                            if (doGrupo.length === 0) return null;
                            const idTitulo = `${idBase}-grupo-${GRUPOS_DA_BUSCA.indexOf(grupo)}`;
                            return (
                                <div key={grupo} role="group" aria-labelledby={idTitulo}>
                                    <div id={idTitulo} className="poph-busca__grupo">
                                        {grupo}
                                    </div>
                                    {doGrupo.map(({ opcao, posicao }) => {
                                        const ativa = posicao === indiceAtivo;
                                        return (
                                            <div
                                                key={opcao.chave}
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
                                                onClick={() => executar(opcao)}
                                            >
                                                <span className="poph-busca__rotulo-opcao">
                                                    {opcao.rotulo}
                                                </span>
                                                {opcao.detalhe && (
                                                    <span className="poph-busca__slug">
                                                        {opcao.detalhe}
                                                    </span>
                                                )}
                                            </div>
                                        );
                                    })}
                                </div>
                            );
                        })
                    )}
                </div>
            )}
            <div role="status" className={aviso ? "poph-busca__aviso" : "poph-busca__rotulo-oculto"}>
                {aviso}
            </div>
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

/** Texto do estado da instância e se ele é de alerta (sem conexão ou degradada). */
function estadoDaInstancia(
    dados: { status: string } | null,
    semConexao: boolean,
    degradada: boolean,
    memoria: number | null,
): { texto: string; alerta: boolean } {
    if (semConexao) return { texto: "Sem conexão", alerta: true };
    if (degradada) return { texto: "Engine: 503 · degradada", alerta: true };
    if (!dados) return { texto: "Engine: verificando…", alerta: false };
    const situacao = dados.status === "ok" ? "200 OK" : "degradada";
    const mb = memoria == null ? null : Math.round(memoria / 1048576);
    const texto = mb == null ? `Engine: ${situacao}` : `Engine: ${situacao} · ${mb} MB`;
    return { texto, alerta: dados.status !== "ok" };
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
    const { pathname } = useLocation();
    const { saude: dados, semConexao, degradada } = useSaude();
    const memoria = useMemoriaDoProcesso();
    const escopoDoToken = sessao?.escopo === "org" ? "Token de org" : "Token global";
    const telas = telasDisponiveis(contextoDaUrl(pathname));
    const estado = estadoDaInstancia(dados, semConexao, degradada, memoria);

    return (
        <header className="poph-topbar">
            <div className="poph-topbar__esquerda">
                <div className="poph-topbar__marca">
                    <span className="poph-topbar__monograma" aria-hidden="true">
                        P
                    </span>
                    <span className="poph-topbar__nome">POP HARNESS</span>
                    {dados?.version && (
                        <span className="poph-topbar__versao" title="Versão da instância">
                            v{dados.version}
                        </span>
                    )}
                </div>
                <span className="poph-topbar__divisor" aria-hidden="true">
                    /
                </span>
                <Trilha orgs={orgs} />
            </div>
            <div className="poph-topbar__centro">
                <BuscaGlobal telas={telas} />
            </div>
            <div className="poph-topbar__acoes">
                <span
                    className={
                        estado.alerta
                            ? "poph-topbar__estado poph-topbar__estado--sem"
                            : "poph-topbar__estado"
                    }
                    title={`${enderecoDaInstancia()} · ${escopoDoToken} · consulta a cada 15 s`}
                >
                    <span className="poph-topbar__ponto" aria-hidden="true" />
                    <span className="poph-topbar__estado-texto">{estado.texto}</span>
                </span>
                {/* Leitura para quem usa leitor de tela: só anuncia quando a instância cai. */}
                <span role="status" className="poph-topbar__so-leitor">
                    {semConexao ? "Instância sem conexão" : ""}
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

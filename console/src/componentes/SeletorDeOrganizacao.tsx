/**
 * Seletor de organização no topo do rail (CF-06, desenho `OrgWorkspaceSwitcherCombobox`).
 *
 * Um único combobox de navegação: mostra a organização aberta na URL ("Administrar Organizações" na
 * lista, "Selecionar organização" fora dela e "Nenhuma organização" sem nenhuma). Ao abrir, lista as organizações que o
 * token enxerga, com busca por nome ou slug. O primeiro item leva à tela de administração das
 * organizações (`/orgs`), que não é recurso de dentro de uma organização. Escolher passa pela guarda
 * de alterações, como o clique na linha da lista. Token de organização vê só a própria e não abre a lista.
 *
 * A SofaUI não tem combobox de navegação com popover, então o padrão é feito aqui: botão com
 * `role="combobox"`, lista com `role="listbox"` e `aria-activedescendant` no campo de busca. O
 * popover vai para o `body`, fixo na tela, para não ser cortado pela área de rolagem do rail.
 */

import {
    useEffect,
    useId,
    useRef,
    useState,
    type FocusEvent,
    type KeyboardEvent,
    type MouseEvent,
} from "react";
import { createPortal } from "react-dom";
import { IconBuilding, IconCheck, IconGrid, IconSearch } from "@sofacoding/sofa-ui";
import type { Org } from "../api/cliente";
import { filtrarOrgs } from "./busca";
import { useGuardaDeAlteracoes } from "./GuardaAlteracoes";
import "../estilos/seletorDeOrganizacao.css";

/** Largura do rail no modo foco: abaixo disso o gatilho está recolhido e o popover abre ao lado. */
const LARGURA_DO_GATILHO_RECOLHIDO = 64;
const LARGURA_DO_POPOVER_AO_LADO = 264;
const ESPACO_DA_JANELA = 16;

export type Posicao = { top: number; left: number; width: number; maxHeight: number };

/**
 * Ancora o popover sob o gatilho (rail expandido) ou ao lado dele (rail recolhido).
 * Exportada para o seletor de agente do CF-07 usar a mesma geometria.
 */
export function posicaoDoGatilho(gatilho: HTMLElement | null): Posicao {
    const alvo = gatilho?.getBoundingClientRect();
    if (!alvo) return { top: 0, left: 0, width: 240, maxHeight: 400 };
    if (alvo.width <= LARGURA_DO_GATILHO_RECOLHIDO) {
        return {
            top: alvo.top,
            left: alvo.right + 8,
            width: LARGURA_DO_POPOVER_AO_LADO,
            maxHeight: Math.max(160, window.innerHeight - alvo.top - ESPACO_DA_JANELA),
        };
    }
    return {
        top: alvo.bottom + 4,
        left: alvo.left,
        width: alvo.width,
        maxHeight: Math.max(160, window.innerHeight - alvo.bottom - ESPACO_DA_JANELA),
    };
}

/** Uma linha da lista: "Administrar Organizações" (org `null`) ou uma organização. */
type Opcao = { chave: string; org: Org | null };

/** Ponto de situação da organização: ativa e inativa têm tom próprio; outros estados não mostram ponto. */
function Situacao({ status }: { status: string | undefined }) {
    if (status !== "active" && status !== "inactive") return null;
    const ativa = status === "active";
    const rotulo = ativa ? "Ativa" : "Inativa";
    return (
        <span
            className={
                ativa
                    ? "poph-orgsel__situacao poph-orgsel__situacao--ativa"
                    : "poph-orgsel__situacao poph-orgsel__situacao--inativa"
            }
        >
            <span className="poph-orgsel__ponto" aria-hidden="true" />
            <span className="poph-orgsel__so-leitor">{rotulo}</span>
        </span>
    );
}

/** Setas para cima e para baixo: o indicador de combobox do desenho (`chevrons_up_down`). */
function IconeSetas() {
    return (
        <svg
            className="poph-orgsel__setas"
            viewBox="0 0 12 12"
            width="11"
            height="11"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.3"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
        >
            <path d="M1 3.5 6 .5l5 3" />
            <path d="M1 8.5 6 11.5l5-3" />
        </svg>
    );
}

export function SeletorDeOrganizacao({
    orgs,
    escopo,
    selecionada,
    naListaDeOrgs,
    compacta,
}: {
    orgs: Org[];
    escopo: "global" | "org";
    /** Id da organização aberta na URL (ou a própria, com token de organização). */
    selecionada: string;
    /** `true` quando a URL é a lista (`/orgs`), sem organização aberta. */
    naListaDeOrgs: boolean;
    /** Modo foco: o gatilho vira só o tile do ícone. */
    compacta: boolean;
}) {
    const guarda = useGuardaDeAlteracoes();
    const idBase = useId();
    const gatilho = useRef<HTMLButtonElement>(null);
    const painel = useRef<HTMLDivElement>(null);
    const campo = useRef<HTMLInputElement>(null);
    const [aberto, setAberto] = useState(false);
    const [termo, setTermo] = useState("");
    const [ativo, setAtivo] = useState(0);
    const [posicao, setPosicao] = useState<Posicao>({ top: 0, left: 0, width: 240, maxHeight: 400 });

    // Token de organização não troca de organização: o gatilho mostra a própria e não abre.
    const bloqueado = escopo === "org";
    const atual = orgs.find((org) => org.id === selecionada) ?? null;
    const rotulo = atual
        ? atual.name
        : orgs.length === 0
          ? "Nenhuma organização"
          : naListaDeOrgs
            ? "Administrar Organizações"
            : "Selecionar organização";

    const termoLimpo = termo.trim();
    const itens: Opcao[] = [];
    // "Todas" só aparece com organizações para escolher; sem nenhuma, o popover explica.
    if (termoLimpo === "" && escopo === "global" && orgs.length > 0) {
        itens.push({ chave: "todas", org: null });
    }
    for (const org of termoLimpo === "" ? orgs : filtrarOrgs(orgs, termo)) {
        itens.push({ chave: org.id, org });
    }
    const indiceAtivo = itens.length === 0 ? -1 : Math.min(ativo, itens.length - 1);

    const idLista = `${idBase}-lista`;
    const idOpcao = (posicaoNaLista: number) => `${idBase}-opcao-${posicaoNaLista}`;

    // Abrir põe o foco no campo de busca; a opção ativa rola para dentro da lista.
    useEffect(() => {
        if (aberto) campo.current?.focus();
    }, [aberto]);

    useEffect(() => {
        if (!aberto || indiceAtivo < 0) return;
        const elemento = document.getElementById(`${idBase}-opcao-${indiceAtivo}`);
        elemento?.scrollIntoView?.({ block: "nearest" });
    }, [aberto, indiceAtivo, idBase]);

    // Com o popover aberto, a posição acompanha a janela e a rolagem (a rolagem da própria lista não conta).
    useEffect(() => {
        if (!aberto) return;
        const atualizar = (evento: Event) => {
            if (evento.target instanceof Node && painel.current?.contains(evento.target)) return;
            setPosicao(posicaoDoGatilho(gatilho.current));
        };
        window.addEventListener("resize", atualizar);
        window.addEventListener("scroll", atualizar, true);
        return () => {
            window.removeEventListener("resize", atualizar);
            window.removeEventListener("scroll", atualizar, true);
        };
    }, [aberto]);

    function abrir() {
        if (bloqueado) return;
        setPosicao(posicaoDoGatilho(gatilho.current));
        setTermo("");
        setAtivo(0);
        setAberto(true);
    }

    /** Fecha o popover. Com `devolverFoco`, o foco volta ao gatilho (Esc, escolha, clique no gatilho). */
    function fechar(devolverFoco: boolean) {
        setAberto(false);
        setTermo("");
        if (devolverFoco) gatilho.current?.focus();
    }

    function escolher(opcao: Opcao) {
        if (opcao.org === null) {
            fechar(true);
            if (!naListaDeOrgs) guarda.tentarNavegar("/orgs");
            return;
        }
        if (opcao.org.id !== selecionada) guarda.tentarNavegar(`/orgs/${opcao.org.id}`);
        fechar(true);
    }

    /** Clique fora (o foco sai do popover e do gatilho) fecha sem devolver o foco. */
    function aoSairDoFoco(evento: FocusEvent<HTMLDivElement>) {
        const destino = evento.relatedTarget;
        if (
            destino instanceof Node &&
            (painel.current?.contains(destino) || gatilho.current?.contains(destino))
        ) {
            return;
        }
        fechar(false);
    }

    function teclarNoPopover(evento: KeyboardEvent<HTMLDivElement>) {
        const ultimo = Math.max(itens.length - 1, 0);
        switch (evento.key) {
            case "ArrowDown":
                evento.preventDefault();
                setAtivo(Math.min((indiceAtivo < 0 ? 0 : indiceAtivo) + 1, ultimo));
                return;
            case "ArrowUp":
                evento.preventDefault();
                setAtivo(Math.max((indiceAtivo < 0 ? 0 : indiceAtivo) - 1, 0));
                return;
            case "Home":
                evento.preventDefault();
                setAtivo(0);
                return;
            case "End":
                evento.preventDefault();
                setAtivo(ultimo);
                return;
            case "Enter": {
                const escolhida = indiceAtivo >= 0 ? itens[indiceAtivo] : undefined;
                if (!escolhida) return;
                evento.preventDefault();
                escolher(escolhida);
                return;
            }
            case "Escape":
                evento.preventDefault();
                evento.stopPropagation();
                fechar(true);
                return;
            default:
                return;
        }
    }

    function aoClicarNoGatilho() {
        if (aberto) fechar(true);
        else abrir();
    }

    // Com o popover aberto, o clique no gatilho não tira o foco do campo (o fechamento fica no clique).
    function aoPressionarNoGatilho(evento: MouseEvent<HTMLButtonElement>) {
        if (aberto) evento.preventDefault();
    }

    const popover =
        aberto && !bloqueado
            ? createPortal(
                  <div
                      ref={painel}
                      className="poph-orgsel__painel"
                      style={{
                          top: posicao.top,
                          left: posicao.left,
                          width: posicao.width,
                          maxHeight: posicao.maxHeight,
                      }}
                      onKeyDown={teclarNoPopover}
                      onBlur={aoSairDoFoco}
                  >
                      <div className="poph-orgsel__busca">
                          <IconSearch aria-hidden="true" />
                          <input
                              ref={campo}
                              id={`${idBase}-filtro`}
                              type="text"
                              className="poph-orgsel__campo"
                              placeholder="Filtrar organizações…"
                              aria-label="Filtrar organizações por nome ou slug"
                              autoComplete="off"
                              spellCheck={false}
                              value={termo}
                              aria-controls={idLista}
                              aria-autocomplete="list"
                              aria-activedescendant={
                                  indiceAtivo >= 0 ? idOpcao(indiceAtivo) : undefined
                              }
                              onChange={(evento) => {
                                  setTermo(evento.target.value);
                                  setAtivo(0);
                              }}
                          />
                      </div>
                      <div className="poph-orgsel__grupo" aria-hidden="true">
                          Organizações disponíveis
                      </div>
                      {itens.length === 0 ? (
                          <p className="poph-orgsel__vazio" role="status">
                              {orgs.length === 0
                                  ? "Nenhuma organização"
                                  : `Nenhuma organização encontrada para “${termoLimpo}”`}
                          </p>
                      ) : (
                          <ul
                              id={idLista}
                              role="listbox"
                              aria-label="Organizações disponíveis"
                              className="poph-orgsel__lista"
                          >
                              {itens.map((opcao, posicaoNaLista) => {
                                  const ativa = posicaoNaLista === indiceAtivo;
                                  const corrente = opcao.org
                                      ? opcao.org.id === selecionada
                                      : naListaDeOrgs;
                                  return (
                                      <li
                                          key={opcao.chave}
                                          id={idOpcao(posicaoNaLista)}
                                          role="option"
                                          aria-selected={ativa}
                                          aria-current={corrente ? "true" : undefined}
                                          className={
                                              ativa
                                                  ? "poph-orgsel__opcao poph-orgsel__opcao--ativa"
                                                  : "poph-orgsel__opcao"
                                          }
                                          // Mantém o foco no campo ao clicar numa opção.
                                          onMouseDown={(evento) => evento.preventDefault()}
                                          onMouseEnter={() => setAtivo(posicaoNaLista)}
                                          onClick={() => escolher(opcao)}
                                      >
                                          {opcao.org ? (
                                              <IconBuilding aria-hidden="true" />
                                          ) : (
                                              <IconGrid aria-hidden="true" />
                                          )}
                                          <span className="poph-orgsel__texto">
                                              <span className="poph-orgsel__nome">
                                                  {opcao.org
                                                      ? opcao.org.name
                                                      : "Administrar Organizações"}
                                              </span>
                                              {opcao.org && (
                                                  <span className="poph-orgsel__slug">
                                                      {opcao.org.slug}
                                                  </span>
                                              )}
                                          </span>
                                          {opcao.org && <Situacao status={opcao.org.status} />}
                                          {corrente && (
                                              <IconCheck
                                                  className="poph-orgsel__marca"
                                                  aria-hidden="true"
                                              />
                                          )}
                                      </li>
                                  );
                              })}
                          </ul>
                      )}
                  </div>,
                  document.body,
              )
            : null;

    return (
        <div className={compacta ? "poph-orgsel poph-orgsel--compacta" : "poph-orgsel"}>
            <button
                ref={gatilho}
                type="button"
                role="combobox"
                aria-haspopup="listbox"
                aria-expanded={aberto}
                aria-controls={aberto ? idLista : undefined}
                aria-label={`Organização: ${rotulo}`}
                title={rotulo}
                disabled={bloqueado}
                className="poph-orgsel__gatilho"
                onClick={aoClicarNoGatilho}
                onMouseDown={aoPressionarNoGatilho}
                onKeyDown={(evento) => {
                    if (evento.key === "ArrowDown" && !aberto) {
                        evento.preventDefault();
                        abrir();
                    }
                }}
            >
                <IconBuilding aria-hidden="true" className="poph-orgsel__icone" />
                <span className="poph-orgsel__rotulo-gatilho">{rotulo}</span>
                <IconeSetas />
            </button>
            {popover}
        </div>
    );
}

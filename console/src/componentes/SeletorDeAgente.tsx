/**
 * Topo do rail na visão do agente (CF-07, desenho `AgentSwitcherCombobox` da prancha 03).
 *
 * Duas peças, nesta ordem: o link de volta "← Agentes da Org" (22 px, texto de 11 px em tom mudo) e
 * o combobox do agente (36 px no desenho: ícone de bot em âmbar, nome em 12,5 px Medium, subtítulo
 * de 10,5 px com modelo e situação, e as setas up/down). O clique no link passa pela guarda de
 * alterações como qualquer link interno (a guarda intercepta o clique em `a[href]`).
 *
 * Escolher outro agente navega para a aba de Instruções dele, também pela guarda. `Esc` fecha sem
 * navegar e devolve o foco ao gatilho; clicar fora fecha; escolher o agente já aberto só fecha.
 *
 * Dados: **uma** consulta por organização (`GET …/agents`), guardada no componente enquanto o rail
 * viver, e uma consulta extra (`GET …/agents/{id}`) só no caso de o agente da URL não estar na
 * lista. Toda leitura termina em `pronto` ou `erro`, inclusive por prazo: nunca fica carregando
 * para sempre. A SofaUI não tem combobox de navegação, então o padrão (`combobox` + `listbox` com
 * `aria-activedescendant`) é feito à mão, como no seletor de organização do CF-06.
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
import { Link } from "react-router-dom";
import { createPortal } from "react-dom";
import { IconArrowLeft, IconBot, IconCheck, IconSearch } from "@sofacoding/sofa-ui";
import { listAgents, getAgent, type Agente } from "../api/agents";
import { normalizarBusca } from "./busca";
import { useGuardaDeAlteracoes } from "./GuardaAlteracoes";
import { posicaoDoGatilho, type Posicao } from "./SeletorDeOrganizacao";
import { translate, useI18n } from "../i18n";
import "../estilos/seletorDeAgente.css";

/** Prazo de cada leitura: passado isso, o estado vira erro em vez de "carregando" eterno. */
const PRAZO_DA_LEITURA_MS = 12_000;

/** Situação do agente em caixa de frase, como nas outras telas. */
function rotuloDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return translate("agents.agentStatusActive");
        case "inactive":
            return translate("agents.agentStatusInactive");
        case "archived":
            return translate("agents.agentStatusArchived");
        default:
            return situacao;
    }
}

/** Modelo real do agente; sem modelo próprio, ele herda o padrão do servidor (a API não devolve qual). */
function rotuloDoModelo(agente: Agente): string {
    const modelo = agente.model?.trim();
    return modelo ? modelo : translate("agents.defaultServerModel");
}

/** Texto do subtítulo do gatilho e da opção: "modelo · situação", só com o que existe. */
function subtituloDoAgente(agente: Agente): string {
    const partes = [rotuloDoModelo(agente), rotuloDaSituacao(agente.status)].filter(Boolean);
    return partes.join(" · ");
}

function mensagemDoErro(falha: unknown): string {
    const texto = falha instanceof Error ? falha.message : String(falha);
    return texto.length > 120 ? `${texto.slice(0, 117)}…` : texto;
}

type Lista =
    | { estado: "carregando" }
    | { estado: "pronto"; agentes: Agente[] }
    | { estado: "erro"; mensagem: string };

/** Setas para cima e para baixo do desenho (`chevrons_up_down`). */
function IconeSetas() {
    return (
        <svg
            className="poph-agsel__setas"
            viewBox="0 0 12 12"
            width="12"
            height="12"
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

/** Link de volta para a lista de agentes da organização (primeira linha do desenho). */
export function VoltarParaAgentes({
    orgId,
    compacta,
}: {
    orgId: string;
    /** Modo foco: só a seta, com o nome no `title` e no rótulo acessível. */
    compacta: boolean;
}) {
    const { translate: t } = useI18n();
    return (
        <Link
            to={`/orgs/${orgId}/agents`}
            className={
                compacta ? "poph-agsel__voltar poph-agsel__voltar--compacta" : "poph-agsel__voltar"
            }
            aria-label={t("agents.backToAgents")}
            title={t("agents.backToAgents")}
        >
            <IconArrowLeft aria-hidden="true" className="poph-agsel__voltar-seta" />
            <span className="poph-agsel__voltar-texto">{t("agents.backToAgents")}</span>
        </Link>
    );
}

export function SeletorDeAgente({
    orgId,
    agentId,
    compacta,
    aoMudarNome,
}: {
    /** Organização aberta na URL. */
    orgId: string;
    /** Agente aberto na URL. */
    agentId: string;
    /** Modo foco: o gatilho vira o tile de 32×32 com o ícone. */
    compacta: boolean;
    /**
     * Avisa o rótulo resolvido do agente (nome, ou o estado real da leitura) para quem mostra o
     * mesmo nome em outro lugar do rail — o rodapé "Agente: <nome>" do cockpit (CF-09). Não faz
     * leitura nova: reaproveita a que este componente já fez.
     */
    aoMudarNome?: (nome: string) => void;
}) {
    const { translate: t } = useI18n();
    const guarda = useGuardaDeAlteracoes();
    const idBase = useId();
    const gatilho = useRef<HTMLButtonElement>(null);
    const painel = useRef<HTMLDivElement>(null);
    const campo = useRef<HTMLInputElement>(null);
    const [aberto, setAberto] = useState(false);
    const [termo, setTermo] = useState("");
    const [ativo, setAtivo] = useState(0);
    const [posicao, setPosicao] = useState<Posicao>({ top: 0, left: 0, width: 240, maxHeight: 400 });

    // Uma leitura por organização, guardada enquanto o rail viver (trocar de agente não relê).
    const lidos = useRef(new Map<string, Agente[]>());
    const [lista, setLista] = useState<Lista>(() =>
        lidos.current.has(orgId)
            ? { estado: "pronto", agentes: lidos.current.get(orgId) ?? [] }
            : { estado: "carregando" },
    );

    useEffect(() => {
        const guardados = lidos.current.get(orgId);
        if (guardados) {
            setLista({ estado: "pronto", agentes: guardados });
            return;
        }
        let encerrado = false;
        setLista({ estado: "carregando" });
        const prazo = setTimeout(() => {
            if (encerrado) return;
            encerrado = true;
            setLista({ estado: "erro", mensagem: translate("agents.agentsTimeout") });
        }, PRAZO_DA_LEITURA_MS);
        listAgents(orgId)
            .then((agentes) => {
                if (encerrado) return;
                encerrado = true;
                clearTimeout(prazo);
                lidos.current.set(orgId, agentes);
                setLista({ estado: "pronto", agentes });
            })
            .catch((falha: unknown) => {
                if (encerrado) return;
                encerrado = true;
                clearTimeout(prazo);
                setLista({ estado: "erro", mensagem: mensagemDoErro(falha) });
            });
        return () => {
            encerrado = true;
            clearTimeout(prazo);
        };
    }, [orgId]);

    const agentes = lista.estado === "pronto" ? lista.agentes : [];
    const daLista = agentes.find((agente) => agente.id === agentId) ?? null;
    // A lista traz até 200 agentes; quem ficou de fora (outra org, agente removido) é lido sozinho.
    const precisarLerAvulso = lista.estado === "pronto" && !daLista && agentId !== "";
    const chaveAvulsa = `${orgId}/${agentId}`;
    const [avulso, setAvulso] = useState<{ chave: string; agente: Agente | null } | null>(null);

    useEffect(() => {
        if (!precisarLerAvulso) return;
        let encerrado = false;
        getAgent(orgId, agentId)
            .then((agente) => {
                if (!encerrado) setAvulso({ chave: chaveAvulsa, agente });
            })
            .catch(() => {
                if (!encerrado) setAvulso({ chave: chaveAvulsa, agente: null });
            });
        return () => {
            encerrado = true;
        };
    }, [precisarLerAvulso, chaveAvulsa, orgId, agentId]);

    const avulsoDaChave = avulso?.chave === chaveAvulsa ? avulso : null;
    const atual: Agente | null = daLista ?? avulsoDaChave?.agente ?? null;
    // Rótulo do gatilho: o nome do agente quando já se sabe; senão, o estado real da leitura.
    let nome: string;
    if (atual) nome = atual.name;
    else if (lista.estado === "erro")
        nome = agentId ? t("agents.agentsUnavailable") : t("agents.noAgents");
    else if (lista.estado === "carregando") nome = t("agents.loading");
    else if (!agentId) nome = t("agents.noAgentOpen");
    else if (!avulsoDaChave) nome = t("agents.loading"); // o agente saiu da lista: leitura dele em curso
    else nome = t("agents.agentNotFound");
    const subtitulo = atual ? subtituloDoAgente(atual) : "";

    // O rodapé do cockpit mostra o mesmo nome. O callback vai numa ref para o efeito não rodar de
    // novo a cada renderização do pai (o rail o recria a cada mudança de estado).
    const avisarNome = useRef(aoMudarNome);
    avisarNome.current = aoMudarNome;
    useEffect(() => {
        avisarNome.current?.(nome);
    }, [nome]);

    // Opções do popover: os agentes da organização, filtrados por nome ou slug.
    const termoLimpo = termo.trim();
    const alvo = normalizarBusca(termoLimpo);
    const visiveis =
        alvo === ""
            ? agentes
            : agentes.filter(
                  (agente) =>
                      normalizarBusca(agente.name).includes(alvo) ||
                      normalizarBusca(agente.slug).includes(alvo),
              );
    const indiceAtivo = visiveis.length === 0 ? -1 : Math.min(ativo, visiveis.length - 1);
    const idLista = `${idBase}-lista`;
    const idOpcao = (posicaoNaLista: number) => `${idBase}-opcao-${posicaoNaLista}`;

    useEffect(() => {
        if (aberto) campo.current?.focus();
    }, [aberto]);

    useEffect(() => {
        if (!aberto || indiceAtivo < 0) return;
        document.getElementById(`${idBase}-opcao-${indiceAtivo}`)?.scrollIntoView?.({ block: "nearest" });
    }, [aberto, indiceAtivo, idBase]);

    // Com o popover aberto, a posição acompanha a janela e a rolagem (a da própria lista não conta).
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
        setPosicao(posicaoDoGatilho(gatilho.current));
        setTermo("");
        setAtivo(0);
        setAberto(true);
    }

    /** Fecha o popover; com `devolverFoco`, o foco volta ao gatilho (Esc, escolha). */
    function fechar(devolverFoco: boolean) {
        setAberto(false);
        setTermo("");
        if (devolverFoco) gatilho.current?.focus();
    }

    /** Escolher outro agente fecha o popover e navega pela guarda; o atual só fecha. */
    function escolher(agente: Agente) {
        fechar(true);
        if (agente.id === agentId) return;
        guarda.tentarNavegar(`/orgs/${orgId}/agents/${agente.id}/instructions`);
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
        const ultimo = Math.max(visiveis.length - 1, 0);
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
                const escolhido = indiceAtivo >= 0 ? visiveis[indiceAtivo] : undefined;
                if (!escolhido) return;
                evento.preventDefault();
                escolher(escolhido);
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

    // Com o popover aberto, o clique no gatilho não tira o foco do campo (o fechamento fica no clique).
    function aoPressionarNoGatilho(evento: MouseEvent<HTMLButtonElement>) {
        if (aberto) evento.preventDefault();
    }

    const popover = aberto
        ? createPortal(
              <div
                  ref={painel}
                  className="poph-agsel__painel"
                  style={{
                      top: posicao.top,
                      left: posicao.left,
                      width: posicao.width,
                      maxHeight: posicao.maxHeight,
                  }}
                  onKeyDown={teclarNoPopover}
                  onBlur={aoSairDoFoco}
              >
                  <div className="poph-agsel__busca">
                      <IconSearch aria-hidden="true" />
                      <input
                          ref={campo}
                          id={`${idBase}-filtro`}
                          type="text"
                          className="poph-agsel__campo"
                          placeholder={t("agents.filterAgentsPlaceholder")}
                          aria-label={t("agents.filterAgentsLabel")}
                          autoComplete="off"
                          spellCheck={false}
                          value={termo}
                          aria-controls={idLista}
                          aria-autocomplete="list"
                          aria-activedescendant={indiceAtivo >= 0 ? idOpcao(indiceAtivo) : undefined}
                          onChange={(evento) => {
                              setTermo(evento.target.value);
                              setAtivo(0);
                          }}
                      />
                  </div>
                  <div className="poph-agsel__grupo" aria-hidden="true">
                      {t("agents.agentsOfOrg")}
                  </div>
                  {lista.estado === "carregando" ? (
                      <p className="poph-agsel__aviso" role="status">
                          {t("agents.loadingAgents")}
                      </p>
                  ) : lista.estado === "erro" ? (
                      <p className="poph-agsel__aviso" role="status">
                          {t("agents.agentsLoadFailed", { message: lista.mensagem })}
                      </p>
                  ) : visiveis.length === 0 ? (
                      <p className="poph-agsel__aviso" role="status">
                          {agentes.length === 0
                              ? t("agents.noAgentsInOrg")
                              : t("agents.noAgentsFoundFor", { term: termoLimpo })}
                      </p>
                  ) : (
                      <ul
                          id={idLista}
                          role="listbox"
                          aria-label={t("agents.agentsOfOrg")}
                          className="poph-agsel__lista"
                      >
                          {visiveis.map((agente, posicaoNaLista) => {
                              const destacado = posicaoNaLista === indiceAtivo;
                              const corrente = agente.id === agentId;
                              return (
                                  <li
                                      key={agente.id}
                                      id={idOpcao(posicaoNaLista)}
                                      role="option"
                                      aria-selected={destacado}
                                      aria-current={corrente ? "true" : undefined}
                                      className={
                                          destacado
                                              ? "poph-agsel__opcao poph-agsel__opcao--ativa"
                                              : "poph-agsel__opcao"
                                      }
                                      // Mantém o foco no campo ao clicar numa opção.
                                      onMouseDown={(evento) => evento.preventDefault()}
                                      onMouseEnter={() => setAtivo(posicaoNaLista)}
                                      onClick={() => escolher(agente)}
                                  >
                                      <IconBot aria-hidden="true" />
                                      <span className="poph-agsel__texto">
                                          <span className="poph-agsel__nome">{agente.name}</span>
                                          <span className="poph-agsel__slug">
                                              {subtituloDoAgente(agente)}
                                          </span>
                                      </span>
                                      {corrente && (
                                          <IconCheck
                                              className="poph-agsel__marca"
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
        <div className={compacta ? "poph-agsel poph-agsel--compacta" : "poph-agsel"}>
            <button
                ref={gatilho}
                type="button"
                role="combobox"
                aria-haspopup="listbox"
                aria-expanded={aberto}
                aria-controls={aberto ? idLista : undefined}
                aria-label={t("agents.agentSelectorLabel", { name: nome })}
                title={nome}
                className="poph-agsel__gatilho"
                onClick={() => (aberto ? fechar(true) : abrir())}
                onMouseDown={aoPressionarNoGatilho}
                onKeyDown={(evento) => {
                    if (evento.key === "ArrowDown" && !aberto) {
                        evento.preventDefault();
                        abrir();
                    }
                }}
            >
                <IconBot aria-hidden="true" className="poph-agsel__icone" />
                <span className="poph-agsel__coluna">
                    <span className="poph-agsel__rotulo">{nome}</span>
                    {subtitulo !== "" && <span className="poph-agsel__sub">{subtitulo}</span>}
                </span>
                <IconeSetas />
            </button>
            {popover}
        </div>
    );
}

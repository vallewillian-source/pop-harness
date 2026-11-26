
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
import { normaliseSearch } from "../components/search";
import { useGuardChanges } from "../components/GuardChanges";
import { positionTrigger, type Position } from "../components/SelectorOrganization";
import { translate, useI18n } from "../i18n";
import "../styles/agent-selector.css";

const DEADLINE_READ_MS = 12_000;

function labelSituation(situacao: string): string {
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

function labelModel(agente: Agente): string {
    const modelo = agente.model?.trim();
    return modelo ? modelo : translate("agents.defaultServerModel");
}

function subtitleAgent(agente: Agente): string {
    const parts = [labelModel(agente), labelSituation(agente.status)].filter(Boolean);
    return parts.join(" · ");
}

function messageError(failure: unknown): string {
    const texto = failure instanceof Error ? failure.message : String(failure);
    return texto.length > 120 ? `${texto.slice(0, 117)}…` : texto;
}

type List =
    | { estado: "carregando" }
    | { estado: "pronto"; agentes: Agente[] }
    | { estado: "erro"; mensagem: string };

function IconArrows() {
    return (
        <svg
            className="poph-agent-select__arrows"
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

export function ReturnAgents({
    orgId,
    compact,
}: {
    orgId: string;
    compact: boolean;
}) {
    const { translate: t } = useI18n();
    return (
        <Link
            to={`/orgs/${orgId}/agents`}
            className={
                compact ? "poph-agent-select__return poph-agent-select__return--compact" : "poph-agent-select__return"
            }
            aria-label={t("agents.backToAgents")}
            title={t("agents.backToAgents")}
        >
            <IconArrowLeft aria-hidden="true" className="poph-agent-select__return-arrow" />
            <span className="poph-agent-select__return-text">{t("agents.backToAgents")}</span>
        </Link>
    );
}

export function SelectorAgent({
    orgId,
    agentId,
    compact,
    onChangeName,
}: {
    orgId: string;
    agentId: string;
    compact: boolean;
    onChangeName?: (nome: string) => void;
}) {
    const { translate: t } = useI18n();
    const guard = useGuardChanges();
    const idBase = useId();
    const gatilho = useRef<HTMLButtonElement>(null);
    const panel = useRef<HTMLDivElement>(null);
    const campo = useRef<HTMLInputElement>(null);
    const [isOpen, setOpen] = useState(false);
    const [term, setTerm] = useState("");
    const [ativo, setActive] = useState(0);
    const [position, setPosition] = useState<Position>({ top: 0, left: 0, width: 240, maxHeight: 400 });

    const read = useRef(new Map<string, Agente[]>());
    const [list, setList] = useState<List>(() =>
        read.current.has(orgId)
            ? { estado: "pronto", agentes: read.current.get(orgId) ?? [] }
            : { estado: "carregando" },
    );

    useEffect(() => {
        const guarded = read.current.get(orgId);
        if (guarded) {
            setList({ estado: "pronto", agentes: guarded });
            return;
        }
        let ended = false;
        setList({ estado: "carregando" });
        const deadline = setTimeout(() => {
            if (ended) return;
            ended = true;
            setList({ estado: "erro", mensagem: translate("agents.agentsTimeout") });
        }, DEADLINE_READ_MS);
        listAgents(orgId)
            .then((agents) => {
                if (ended) return;
                ended = true;
                clearTimeout(deadline);
                read.current.set(orgId, agents);
                setList({ estado: "pronto", agentes: agents });
            })
            .catch((failure: unknown) => {
                if (ended) return;
                ended = true;
                clearTimeout(deadline);
                setList({ estado: "erro", mensagem: messageError(failure) });
            });
        return () => {
            ended = true;
            clearTimeout(deadline);
        };
    }, [orgId]);

    const agents = list.estado === "pronto" ? list.agentes : [];
    const fromList = agents.find((agente) => agente.id === agentId) ?? null;
    const needReadStandalone = list.estado === "pronto" && !fromList && agentId !== "";
    const keyOne_off = `${orgId}/${agentId}`;
    const [standalone, setStandalone] = useState<{ chave: string; agente: Agente | null } | null>(null);

    useEffect(() => {
        if (!needReadStandalone) return;
        let ended = false;
        getAgent(orgId, agentId)
            .then((agente) => {
                if (!ended) setStandalone({ chave: keyOne_off, agente });
            })
            .catch(() => {
                if (!ended) setStandalone({ chave: keyOne_off, agente: null });
            });
        return () => {
            ended = true;
        };
    }, [needReadStandalone, keyOne_off, orgId, agentId]);

    const standaloneKey = standalone?.chave === keyOne_off ? standalone : null;
    const current: Agente | null = fromList ?? standaloneKey?.agente ?? null;
    let nome: string;
    if (current) nome = current.name;
    else if (list.estado === "erro")
        nome = agentId ? t("agents.agentsUnavailable") : t("agents.noAgents");
    else if (list.estado === "carregando") nome = t("agents.loading");
    else if (!agentId) nome = t("agents.noAgentOpen");
    else if (!standaloneKey) nome = t("agents.loading");
    else nome = t("agents.agentNotFound");
    const subtitulo = current ? subtitleAgent(current) : "";

    const notifyName = useRef(onChangeName);
    notifyName.current = onChangeName;
    useEffect(() => {
        notifyName.current?.(nome);
    }, [nome]);

    const termClean = term.trim();
    const target = normaliseSearch(termClean);
    const visible =
        target === ""
            ? agents
            : agents.filter(
                  (agente) =>
                      normaliseSearch(agente.name).includes(target) ||
                      normaliseSearch(agente.slug).includes(target),
              );
    const indexActive = visible.length === 0 ? -1 : Math.min(ativo, visible.length - 1);
    const idList = `${idBase}-lista`;
    const idOption = (positionList: number) => `${idBase}-opcao-${positionList}`;

    useEffect(() => {
        if (isOpen) campo.current?.focus();
    }, [isOpen]);

    useEffect(() => {
        if (!isOpen || indexActive < 0) return;
        document.getElementById(`${idBase}-opcao-${indexActive}`)?.scrollIntoView?.({ block: "nearest" });
    }, [isOpen, indexActive, idBase]);

    useEffect(() => {
        if (!isOpen) return;
        const atualizar = (evento: Event) => {
            if (evento.target instanceof Node && panel.current?.contains(evento.target)) return;
            setPosition(positionTrigger(gatilho.current));
        };
        window.addEventListener("resize", atualizar);
        window.addEventListener("scroll", atualizar, true);
        return () => {
            window.removeEventListener("resize", atualizar);
            window.removeEventListener("scroll", atualizar, true);
        };
    }, [isOpen]);

    function open() {
        setPosition(positionTrigger(gatilho.current));
        setTerm("");
        setActive(0);
        setOpen(true);
    }

    function close(returnFocus: boolean) {
        setOpen(false);
        setTerm("");
        if (returnFocus) gatilho.current?.focus();
    }

    function choose(agente: Agente) {
        close(true);
        if (agente.id === agentId) return;
        guard.tryNavigate(`/orgs/${orgId}/agents/${agente.id}/instructions`);
    }

    function onExitFocus(evento: FocusEvent<HTMLDivElement>) {
        const destination = evento.relatedTarget;
        if (
            destination instanceof Node &&
            (panel.current?.contains(destination) || gatilho.current?.contains(destination))
        ) {
            return;
        }
        close(false);
    }

    function pressOnPopover(evento: KeyboardEvent<HTMLDivElement>) {
        const last = Math.max(visible.length - 1, 0);
        switch (evento.key) {
            case "ArrowDown":
                evento.preventDefault();
                setActive(Math.min((indexActive < 0 ? 0 : indexActive) + 1, last));
                return;
            case "ArrowUp":
                evento.preventDefault();
                setActive(Math.max((indexActive < 0 ? 0 : indexActive) - 1, 0));
                return;
            case "Home":
                evento.preventDefault();
                setActive(0);
                return;
            case "End":
                evento.preventDefault();
                setActive(last);
                return;
            case "Enter": {
                const chosen = indexActive >= 0 ? visible[indexActive] : undefined;
                if (!chosen) return;
                evento.preventDefault();
                choose(chosen);
                return;
            }
            case "Escape":
                evento.preventDefault();
                evento.stopPropagation();
                close(true);
                return;
            default:
                return;
        }
    }

    function onPressTrigger(evento: MouseEvent<HTMLButtonElement>) {
        if (isOpen) evento.preventDefault();
    }

    const popover = isOpen
        ? createPortal(
              <div
                  ref={panel}
                  className="poph-agent-select__panel"
                  style={{
                      top: position.top,
                      left: position.left,
                      width: position.width,
                      maxHeight: position.maxHeight,
                  }}
                  onKeyDown={pressOnPopover}
                  onBlur={onExitFocus}
              >
                  <div className="poph-agent-select__search">
                      <IconSearch aria-hidden="true" />
                      <input
                          ref={campo}
                          id={`${idBase}-filtro`}
                          type="text"
                          className="poph-agent-select__field"
                          placeholder={t("agents.filterAgentsPlaceholder")}
                          aria-label={t("agents.filterAgentsLabel")}
                          autoComplete="off"
                          spellCheck={false}
                          value={term}
                          aria-controls={idList}
                          aria-autocomplete="list"
                          aria-activedescendant={indexActive >= 0 ? idOption(indexActive) : undefined}
                          onChange={(evento) => {
                              setTerm(evento.target.value);
                              setActive(0);
                          }}
                      />
                  </div>
                  <div className="poph-agent-select__group" aria-hidden="true">
                      {t("agents.agentsOfOrg")}
                  </div>
                  {list.estado === "carregando" ? (
                      <p className="poph-agent-select__notice" role="status">
                          {t("agents.loadingAgents")}
                      </p>
                  ) : list.estado === "erro" ? (
                      <p className="poph-agent-select__notice" role="status">
                          {t("agents.agentsLoadFailed", { message: list.mensagem })}
                      </p>
                  ) : visible.length === 0 ? (
                      <p className="poph-agent-select__notice" role="status">
                          {agents.length === 0
                              ? t("agents.noAgentsInOrg")
                              : t("agents.noAgentsFoundFor", { term: termClean })}
                      </p>
                  ) : (
                      <ul
                          id={idList}
                          role="listbox"
                          aria-label={t("agents.agentsOfOrg")}
                          className="poph-agent-select__list"
                      >
                          {visible.map((agente, positionList) => {
                              const destacado = positionList === indexActive;
                              const corrente = agente.id === agentId;
                              return (
                                  <li
                                      key={agente.id}
                                      id={idOption(positionList)}
                                      role="option"
                                      aria-selected={destacado}
                                      aria-current={corrente ? "true" : undefined}
                                      className={
                                          destacado
                                              ? "poph-agent-select__option poph-agent-select__option--active"
                                              : "poph-agent-select__option"
                                      }
                                      onMouseDown={(evento) => evento.preventDefault()}
                                      onMouseEnter={() => setActive(positionList)}
                                      onClick={() => choose(agente)}
                                  >
                                      <IconBot aria-hidden="true" />
                                      <span className="poph-agent-select__text">
                                          <span className="poph-agent-select__name">{agente.name}</span>
                                          <span className="poph-agent-select__slug">
                                              {subtitleAgent(agente)}
                                          </span>
                                      </span>
                                      {corrente && (
                                          <IconCheck
                                              className="poph-agent-select__mark"
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
        <div className={compact ? "poph-agent-select poph-agent-select--compact" : "poph-agent-select"}>
            <button
                ref={gatilho}
                type="button"
                role="combobox"
                aria-haspopup="listbox"
                aria-expanded={isOpen}
                aria-controls={isOpen ? idList : undefined}
                aria-label={t("agents.agentSelectorLabel", { name: nome })}
                title={nome}
                className="poph-agent-select__trigger"
                onClick={() => (isOpen ? close(true) : open())}
                onMouseDown={onPressTrigger}
                onKeyDown={(evento) => {
                    if (evento.key === "ArrowDown" && !isOpen) {
                        evento.preventDefault();
                        open();
                    }
                }}
            >
                <IconBot aria-hidden="true" className="poph-agent-select__icon" />
                <span className="poph-agent-select__column">
                    <span className="poph-agent-select__label">{nome}</span>
                    {subtitulo !== "" && <span className="poph-agent-select__sub">{subtitulo}</span>}
                </span>
                <IconArrows />
            </button>
            {popover}
        </div>
    );
}

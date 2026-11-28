
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
import type { Org } from "../api/client";
import { filterOrgs } from "../components/search";
import { useGuardChanges } from "../components/GuardChanges";
import { useI18n } from "../i18n";
import "../styles/org-selector.css";

const WIDTH_TRIGGER_COLLAPSED = 64;
const POPOVER_WIDTH_SIDE = 264;
const SPACE_WINDOW = 16;

export type Position = { top: number; left: number; width: number; maxHeight: number };

export function positionTrigger(gatilho: HTMLElement | null): Position {
    const target = gatilho?.getBoundingClientRect();
    if (!target) return { top: 0, left: 0, width: 240, maxHeight: 400 };
    if (target.width <= WIDTH_TRIGGER_COLLAPSED) {
        return {
            top: target.top,
            left: target.right + 8,
            width: POPOVER_WIDTH_SIDE,
            maxHeight: Math.max(160, window.innerHeight - target.top - SPACE_WINDOW),
        };
    }
    return {
        top: target.bottom + 4,
        left: target.left,
        width: target.width,
        maxHeight: Math.max(160, window.innerHeight - target.bottom - SPACE_WINDOW),
    };
}

type Option = { chave: string; org: Org | null };

function Situacao({ status }: { status: string | undefined }) {
    const { translate: t } = useI18n();
    if (status !== "active" && status !== "inactive") return null;
    const active = status === "active";
    const label = active ? t("navigation.orgStatusActive") : t("navigation.orgStatusInactive");
    return (
        <span
            className={
                active
                    ? "poph-org-select__situation poph-org-select__situation--active"
                    : "poph-org-select__situation poph-orgsel__situacao--inativa"
            }
        >
            <span className="poph-org-select__point" aria-hidden="true" />
            <span className="poph-org-select__only-reader">{label}</span>
        </span>
    );
}

function IconArrows() {
    return (
        <svg
            className="poph-org-select__arrows"
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

export function SelectorOrganization({
    orgs,
    scope,
    selected,
    listOrgs,
    compact,
}: {
    orgs: Org[];
    scope: "global" | "org";
    selected: string;
    listOrgs: boolean;
    compact: boolean;
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

    const bloqueado = scope === "org";
    const current = orgs.find((org) => org.id === selected) ?? null;
    const label = current
        ? current.name
        : orgs.length === 0
          ? t("navigation.orgNone")
          : listOrgs
            ? t("navigation.orgManage")
            : t("navigation.orgSelect");

    const termClean = term.trim();
    const items: Option[] = [];
    if (termClean === "" && scope === "global" && orgs.length > 0) {
        items.push({ chave: "todas", org: null });
    }
    for (const org of termClean === "" ? orgs : filterOrgs(orgs, term)) {
        items.push({ chave: org.id, org });
    }
    const indexActive = items.length === 0 ? -1 : Math.min(ativo, items.length - 1);

    const idList = `${idBase}-lista`;
    const idOption = (positionList: number) => `${idBase}-opcao-${positionList}`;

    useEffect(() => {
        if (isOpen) campo.current?.focus();
    }, [isOpen]);

    useEffect(() => {
        if (!isOpen || indexActive < 0) return;
        const elemento = document.getElementById(`${idBase}-opcao-${indexActive}`);
        elemento?.scrollIntoView?.({ block: "nearest" });
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
        if (bloqueado) return;
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

    function choose(option: Option) {
        if (option.org === null) {
            close(true);
            if (!listOrgs) guard.tryNavigate("/orgs");
            return;
        }
        if (option.org.id !== selected) guard.tryNavigate(`/orgs/${option.org.id}`);
        close(true);
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
        const last = Math.max(items.length - 1, 0);
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
                const chosen = indexActive >= 0 ? items[indexActive] : undefined;
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

    function onClickTrigger() {
        if (isOpen) close(true);
        else open();
    }

    function onPressTrigger(evento: MouseEvent<HTMLButtonElement>) {
        if (isOpen) evento.preventDefault();
    }

    const popover =
        isOpen && !bloqueado
            ? createPortal(
                  <div
                      ref={panel}
                      className="poph-org-select__panel"
                      style={{
                          top: position.top,
                          left: position.left,
                          width: position.width,
                          maxHeight: position.maxHeight,
                      }}
                      onKeyDown={pressOnPopover}
                      onBlur={onExitFocus}
                  >
                      <div className="poph-org-select__search">
                          <IconSearch aria-hidden="true" />
                          <input
                              ref={campo}
                              id={`${idBase}-filtro`}
                              type="text"
                              className="poph-org-select__field"
                              placeholder={t("navigation.orgFilterPlaceholder")}
                              aria-label={t("navigation.orgFilterLabel")}
                              autoComplete="off"
                              spellCheck={false}
                              value={term}
                              aria-controls={idList}
                              aria-autocomplete="list"
                              aria-activedescendant={
                                  indexActive >= 0 ? idOption(indexActive) : undefined
                              }
                              onChange={(evento) => {
                                  setTerm(evento.target.value);
                                  setActive(0);
                              }}
                          />
                      </div>
                      <div className="poph-org-select__group" aria-hidden="true">
                          {t("navigation.orgsAvailable")}
                      </div>
                      {items.length === 0 ? (
                          <p className="poph-org-select__empty" role="status">
                              {orgs.length === 0
                                  ? t("navigation.orgNone")
                                  : t("navigation.orgsEmptyFor", { term: termClean })}
                          </p>
                      ) : (
                          <ul
                              id={idList}
                              role="listbox"
                              aria-label={t("navigation.orgsAvailable")}
                              className="poph-org-select__list"
                          >
                              {items.map((option, positionList) => {
                                  const active = positionList === indexActive;
                                  const corrente = option.org
                                      ? option.org.id === selected
                                      : listOrgs;
                                  return (
                                      <li
                                          key={option.chave}
                                          id={idOption(positionList)}
                                          role="option"
                                          aria-selected={active}
                                          aria-current={corrente ? "true" : undefined}
                                          className={
                                              active
                                                  ? "poph-org-select__option poph-org-select__option--active"
                                                  : "poph-org-select__option"
                                          }
                                          onMouseDown={(evento) => evento.preventDefault()}
                                          onMouseEnter={() => setActive(positionList)}
                                          onClick={() => choose(option)}
                                      >
                                          {option.org ? (
                                              <IconBuilding aria-hidden="true" />
                                          ) : (
                                              <IconGrid aria-hidden="true" />
                                          )}
                                          <span className="poph-org-select__text">
                                              <span className="poph-org-select__name">
                                                  {option.org
                                                      ? option.org.name
                                                      : t("navigation.orgManage")}
                                              </span>
                                              {option.org && (
                                                  <span className="poph-org-select__slug">
                                                      {option.org.slug}
                                                  </span>
                                              )}
                                          </span>
                                          {option.org && <Situacao status={option.org.status} />}
                                          {corrente && (
                                              <IconCheck
                                                  className="poph-org-select__mark"
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
        <div className={compact ? "poph-org-select poph-org-select--compact" : "poph-org-select"}>
            <button
                ref={gatilho}
                type="button"
                role="combobox"
                aria-haspopup="listbox"
                aria-expanded={isOpen}
                aria-controls={isOpen ? idList : undefined}
                aria-label={t("navigation.orgSelectorLabel", { name: label })}
                title={label}
                disabled={bloqueado}
                className="poph-org-select__trigger"
                onClick={onClickTrigger}
                onMouseDown={onPressTrigger}
                onKeyDown={(evento) => {
                    if (evento.key === "ArrowDown" && !isOpen) {
                        evento.preventDefault();
                        open();
                    }
                }}
            >
                <IconBuilding aria-hidden="true" className="poph-org-select__icon" />
                <span className="poph-org-select__label-trigger">{label}</span>
                <IconArrows />
            </button>
            {popover}
        </div>
    );
}

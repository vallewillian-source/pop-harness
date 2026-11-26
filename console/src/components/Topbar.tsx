
import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import {
    IconLock,
    IconLogOut,
    IconSearch,
    SofaButton,
    SofaIconButton,
    type SofaIconProps,
} from "@sofacoding/sofa-ui";
import { translate, useI18n } from "../i18n";
import { useSession } from "../session/Session";
import { useGuardChanges } from "../components/GuardChanges";
import { LanguageSwitcher } from "../components/LanguageSwitcher";
import {
    searchEverything,
    GROUPS_SEARCH,
    SEARCH_GROUP_KEYS,
    type OptionSearch,
    type ScreenSearchable,
} from "../components/search";
import {
    ITEMS_NAVIGATION,
    ITEMS_COCKPIT_AGENT,
    type ContextNavigation,
} from "../components/navigation";
import { useAgentModel } from "../components/agentModel";
import { useMemoryProcess, useHealth } from "../components/health";
import { Trail } from "../components/Trail";
import "../styles/topbar.css";
import "../styles/search.css";

export function addressInstance(): string {
    const { hostname, port } = window.location;
    if (hostname === "127.0.0.1" || hostname === "localhost" || hostname === "") {
        return port ? translate("shell.instanceLocalPort", { port }) : translate("shell.instanceLocal");
    }
    return hostname;
}

function isTyping(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) return false;
    return (
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        target.isContentEditable
    );
}

function textShortcut(): string {
    if (typeof navigator === "undefined") return "Ctrl K";
    return /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘K" : "Ctrl K";
}

function contextUrl(pathname: string): ContextNavigation {
    const parts = pathname.split("/").filter(Boolean);
    const orgOpen = parts[0] === "orgs" ? (parts[1] ?? "") : "";
    const agentOpen = orgOpen && parts[2] === "agents" ? (parts[3] ?? "") : "";
    return {
        orgId: orgOpen || null,
        agentId: agentOpen || null,
        totalOrgs: null,
        totalAgents: null,
    };
}

function screensAvailable(context: ContextNavigation): ScreenSearchable[] {
    const items = context.agentId
        ? [
              ...ITEMS_NAVIGATION.filter((item) => !item.requiresAgent),
              ...ITEMS_COCKPIT_AGENT,
          ]
        : ITEMS_NAVIGATION;
    return items.flatMap((item) => {
        if (item.estado !== "ativo") return [];
        const path = item.route(context);
        return path ? [{ rotulo: item.caption, caminho: path }] : [];
    });
}

const DURATION_NOTICE_MS = 3000;

function SearchGlobal({ screens, placeholder }: { screens: ScreenSearchable[]; placeholder: string }) {
    const { translate } = useI18n();
    const { sessao: session } = useSession();
    const guard = useGuardChanges();
    const campo = useRef<HTMLInputElement>(null);
    const idBase = useId();
    const [term, setTerm] = useState("");
    const [opened, setOpen] = useState(false);
    const [index, setIndex] = useState(0);
    const [notice, setNotice] = useState("");
    const shortcut = textShortcut();

    const options: OptionSearch[] = searchEverything(term, session?.orgs ?? [], screens);
    const showList = opened && term.trim() !== "";
    const indexActive = options.length === 0 ? -1 : Math.min(index, options.length - 1);
    const idField = `${idBase}-campo`;
    const idList = `${idBase}-lista`;
    const idOption = (position: number) => `${idBase}-opcao-${position}`;

    useEffect(() => {
        const onPress = (evento: globalThis.KeyboardEvent) => {
            if (!(evento.ctrlKey || evento.metaKey) || evento.altKey || evento.shiftKey) return;
            if (evento.key.toLowerCase() !== "k") return;
            const input = campo.current;
            if (!input) return;
            if (evento.target !== input && isTyping(evento.target)) return;
            evento.preventDefault();
            input.focus();
            input.select();
        };
        window.addEventListener("keydown", onPress);
        return () => window.removeEventListener("keydown", onPress);
    }, []);

    useEffect(() => {
        if (notice === "") return;
        const id = window.setTimeout(() => setNotice(""), DURATION_NOTICE_MS);
        return () => window.clearTimeout(id);
    }, [notice]);

    function goTo(path: string) {
        const foi = guard.tryNavigate(path);
        setOpen(false);
        if (foi) setTerm("");
    }

    function copyCommand(texto: string) {
        setOpen(false);
        setTerm("");
        if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
            setNotice(translate("shell.copyFailure", { command: texto }));
            return;
        }
        navigator.clipboard.writeText(texto).then(
            () => setNotice(translate("shell.copySuccess", { command: texto })),
            () => setNotice(translate("shell.copyFailure", { command: texto })),
        );
    }

    function run(option: OptionSearch) {
        if (option.destination.tipo === "caminho") {
            goTo(option.destination.caminho);
        } else {
            copyCommand(option.destination.texto);
        }
    }

    function onPress(evento: KeyboardEvent<HTMLInputElement>) {
        if (evento.key === "ArrowDown" || evento.key === "ArrowUp") {
            if (options.length === 0) return;
            evento.preventDefault();
            setOpen(true);
            const step = evento.key === "ArrowDown" ? 1 : -1;
            const total = options.length;
            setIndex(((indexActive + step) % total + total) % total);
            return;
        }
        if (evento.key === "Enter") {
            const chosen = options[indexActive];
            if (!showList || !chosen) return;
            evento.preventDefault();
            run(chosen);
            return;
        }
        if (evento.key === "Escape") {
            if (term === "") return;
            evento.preventDefault();
            setTerm("");
            setOpen(false);
        }
    }

    return (
        <div className="poph-search">
            <label htmlFor={idField} className="poph-search__label-hidden">
                {translate("shell.searchLabel")}
            </label>
            <span className="poph-search__magnifier" aria-hidden="true">
                <IconSearch />
            </span>
            <input
                id={idField}
                ref={campo}
                type="text"
                role="combobox"
                className="poph-search__field"
                placeholder={placeholder}
                autoComplete="off"
                spellCheck={false}
                value={term}
                aria-expanded={showList}
                aria-controls={showList ? idList : undefined}
                aria-autocomplete="list"
                aria-keyshortcuts="Control+K Meta+K"
                aria-activedescendant={
                    showList && indexActive >= 0 ? idOption(indexActive) : undefined
                }
                onChange={(evento) => {
                    setTerm(evento.target.value);
                    setIndex(0);
                    setOpen(true);
                }}
                onFocus={() => setOpen(true)}
                onBlur={() => setOpen(false)}
                onKeyDown={onPress}
            />
            <kbd className="poph-search__shortcut" aria-hidden="true">
                {shortcut}
            </kbd>
            {showList && (
                <div
                    id={idList}
                    className="poph-search__list"
                    role={options.length > 0 ? "listbox" : undefined}
                    aria-label={options.length > 0 ? translate("shell.searchResultsLabel") : undefined}
                >
                    {options.length === 0 ? (
                        <p className="poph-search__empty" role="status">
                            {translate("shell.searchEmpty", { query: term.trim() })}
                        </p>
                    ) : (
                        GROUPS_SEARCH.map((group) => {
                            const ofGroup = options
                                .map((option, position) => ({ option, position }))
                                .filter(({ option }) => option.grupo === group);
                            if (ofGroup.length === 0) return null;
                            const idTitle = `${idBase}-grupo-${GROUPS_SEARCH.indexOf(group)}`;
                            return (
                                <div key={group} role="group" aria-labelledby={idTitle}>
                                    <div id={idTitle} className="poph-search__group">
                                        {translate(SEARCH_GROUP_KEYS[group])}
                                    </div>
                                    {ofGroup.map(({ option, position }) => {
                                        const active = position === indexActive;
                                        return (
                                            <div
                                                key={option.chave}
                                                id={idOption(position)}
                                                role="option"
                                                aria-selected={active}
                                                className={
                                                    active
                                                        ? "poph-search__option poph-search__option--active"
                                                        : "poph-search__option"
                                                }
                                                onMouseDown={(evento) => evento.preventDefault()}
                                                onClick={() => run(option)}
                                            >
                                                <span className="poph-search__label-option">
                                                    {option.rotulo}
                                                </span>
                                                {option.detail && (
                                                    <span className="poph-search__slug">
                                                        {option.detail}
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
            <div role="status" className={notice ? "poph-search__notice" : "poph-search__label-hidden"}>
                {notice}
            </div>
        </div>
    );
}

function IconHelp(props: SofaIconProps) {
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

function stateInstance(
    dados: { status: string } | null,
    withoutConnection: boolean,
    degraded: boolean,
    memory: number | null,
): { texto: string; alert: boolean } {
    if (withoutConnection) return { texto: translate("shell.statusOffline"), alert: true };
    if (degraded) return { texto: translate("shell.statusDegraded503"), alert: true };
    if (!dados) return { texto: translate("shell.statusChecking"), alert: false };
    const situacao =
        dados.status === "ok" ? "200 OK" : translate("shell.statusDegradedWord");
    const mb = memory == null ? null : Math.round(memory / 1048576);
    const texto =
        mb == null
            ? translate("shell.statusEngine", { state: situacao })
            : translate("shell.statusEngineWithMemory", { state: situacao, mb });
    return { texto, alert: dados.status !== "ok" };
}

function healthChip(
    dados: { status: string } | null,
    withoutConnection: boolean,
    degraded: boolean,
): string {
    if (withoutConnection) return translate("shell.chipOffline");
    if (degraded) return translate("shell.chipDegraded503");
    if (!dados) return translate("shell.chipChecking");
    return dados.status === "ok" ? "200 OK" : translate("shell.statusDegradedWord");
}

function stateAgent(
    model: string | null,
    dados: { status: string } | null,
    withoutConnection: boolean,
    degraded: boolean,
): { modelo: string; health: string; alert: boolean } {
    return {
        modelo: translate("shell.agentModel", { model: model ?? "—" }),
        health: healthChip(dados, withoutConnection, degraded),
        alert: withoutConnection || degraded || (dados !== null && dados.status !== "ok"),
    };
}

export function Topbar({
    orgs,
    onOpenShortcuts,
}: {
    orgs: { id: string; name: string }[];
    onOpenShortcuts: () => void;
}) {
    const { translate } = useI18n();
    const { sessao: session, signOut: exit } = useSession();
    const navigate = useNavigate();
    const { pathname } = useLocation();
    const { health: dados, disconnected: withoutConnection, degradada: degraded } = useHealth();
    const memory = useMemoryProcess();
    const tokenScope =
        session?.escopo === "org"
            ? translate("shell.tokenScopeOrg")
            : translate("shell.tokenScopeGlobal");
    const context = contextUrl(pathname);
    const routeAgent = context.agentId !== null;
    const modelAgent = useAgentModel(context.agentId ?? undefined);
    const screens = screensAvailable(context);
    const estado = stateInstance(dados, withoutConnection, degraded, memory);
    const chipAgent = stateAgent(modelAgent, dados, withoutConnection, degraded);
    const titleChip = translate("shell.chipTitle", {
        address: addressInstance(),
        scope: tokenScope,
    });
    const exitConsole = () => {
        exit();
        navigate("/login", { replace: true });
    };

    return (
        <header className={routeAgent ? "poph-topbar poph-topbar--agent" : "poph-topbar"}>
            <div className="poph-topbar__left">
                <div className="poph-topbar__mark">
                    <span className="poph-topbar__name">POP HARNESS</span>
                    {dados?.version && (
                        <span
                            className="poph-topbar__version"
                            title={translate("shell.instanceVersionTitle")}
                        >
                            v{dados.version}
                        </span>
                    )}
                </div>
                <span className="poph-topbar__divider" aria-hidden="true">
                    /
                </span>
                <Trail orgs={orgs} />
            </div>
            <div className="poph-topbar__center">
                <SearchGlobal
                    screens={screens}
                    placeholder={
                        routeAgent
                            ? translate("shell.searchAgentPlaceholder")
                            : translate("shell.searchGlobalPlaceholder")
                    }
                />
            </div>
            <div className="poph-topbar__actions">
                {routeAgent ? (
                    <span
                        className={
                            chipAgent.alert
                                ? "poph-topbar__state poph-topbar__state--agent poph-topbar__state--none"
                                : "poph-topbar__state poph-topbar__state--agent"
                        }
                        title={titleChip}
                    >
                        <span className="poph-topbar__point" aria-hidden="true" />
                        <span className="poph-topbar__state-text">{chipAgent.modelo}</span>
                        <span className="poph-topbar__health">{chipAgent.health}</span>
                    </span>
                ) : (
                    <span
                        className={
                            estado.alert
                                ? "poph-topbar__state poph-topbar__state--none"
                                : "poph-topbar__state"
                        }
                        title={titleChip}
                    >
                        <span className="poph-topbar__point" aria-hidden="true" />
                        <span className="poph-topbar__state-text">{estado.texto}</span>
                    </span>
                )}
                {}
                <span role="status" className="poph-topbar__only-reader">
                    {withoutConnection ? translate("shell.instanceOfflineSr") : ""}
                </span>
                {!routeAgent && (
                    <span className="poph-topbar__token">
                        <IconLock aria-hidden="true" />
                        {tokenScope}
                    </span>
                )}
                {routeAgent ? (
                    <SofaButton
                        variant="subtle"
                        size="sm"
                        className="poph-topbar__exit"
                        aria-label={translate("shell.signOutLabel")}
                        icon={<IconLogOut aria-hidden="true" />}
                        onClick={exitConsole}
                    >
                        {translate("shell.signOut")}
                    </SofaButton>
                ) : (
                    <SofaIconButton label={translate("shell.signOutLabel")} onClick={exitConsole}>
                        <IconLogOut aria-hidden="true" />
                    </SofaIconButton>
                )}
                <LanguageSwitcher />
                <SofaIconButton
                    label={translate("shell.keyboardShortcuts")}
                    size={28}
                    onClick={onOpenShortcuts}
                >
                    <IconHelp aria-hidden="true" />
                </SofaIconButton>
            </div>
        </header>
    );
}

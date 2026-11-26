
import {
    createContext,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useState,
    type ReactNode,
} from "react";
import {
    ApiError,
    setToken,
    request,
    whenExpired,
    type OrgList,
    type Org,
} from "../api/client";
import { translate } from "../i18n";

const CHAVE = "poph-console.sessao";

export type Scope = "global" | "org";

export type Session = {
    token: string;
    "escopo": Scope;
    orgs: Org[];
};

type Context = {
    "sessao": Session | null;
    "expired": boolean;
    "signIn": (token: string) => Promise<Session>;
    "signOut": () => void;
    "reloadOrgs": () => Promise<void>;
    "clearExpiryNotice": () => void;
};

const SessionContext = createContext<Context | null>(null);

function tokenScope(token: string, orgs: Org[]): Scope {
    if (token.startsWith("pophg_")) return "global";
    if (token.startsWith("poph_")) return "org";
    return orgs.length > 1 ? "global" : "org";
}

function readFromTab(): Session | null {
    try {
        const texto = window.sessionStorage.getItem(CHAVE);
        if (!texto) return null;
        const loaded = JSON.parse(texto) as Partial<Session>;
        if (typeof loaded.token !== "string" || !Array.isArray(loaded.orgs)) return null;
        return {
            token: loaded.token,
            "escopo": loaded["escopo"] === "org" ? "org" : "global",
            orgs: loaded.orgs as Org[],
        };
    } catch {
        return null;
    }
}

function saveToTab(session: Session | null): void {
    try {
        if (session) {
            window.sessionStorage.setItem(CHAVE, JSON.stringify(session));
        } else {
            window.sessionStorage.removeItem(CHAVE);
        }
    } catch {
    }
}

export const LAST_ORG_STORAGE_KEY = "poph-console.lastOrgId";

function readLastOrgId(): string | null {
    try {
        return window.localStorage.getItem(LAST_ORG_STORAGE_KEY);
    } catch {
        return null;
    }
}

export function writeLastOrgId(orgId: string): void {
    try {
        window.localStorage.setItem(LAST_ORG_STORAGE_KEY, orgId);
    } catch {
        return;
    }
}

export function resolveDefaultOrgPath(orgs: Org[]): string {
    const saved = readLastOrgId();
    if (saved !== null && orgs.some((org) => org.id === saved)) return `/orgs/${saved}`;
    const first = orgs[0];
    return first ? `/orgs/${first.id}` : "/orgs";
}

export function SessionProvider({ children }: { children: ReactNode }) {
    const [session, setSession] = useState<Session | null>(() => {
        const initial = readFromTab();
        setToken(initial?.token ?? null);
        return initial;
    });
    const [expired, setExpired] = useState(false);

    const signOut = useCallback(() => {
        setToken(null);
        saveToTab(null);
        setSession(null);
    }, []);

    useEffect(() => {
        whenExpired(() => {
            setExpired(true);
            signOut();
        });
        return () => whenExpired(null);
    }, [signOut]);

    const signIn = useCallback(async (token: string) => {
        const clean = token.trim();
        if (!clean) {
            throw new ApiError(400, "token_vazio", translate("login.emptyToken"));
        }
        setToken(clean);
        try {
            const list = await request<OrgList>("/admin/v1/orgs?limit=200", {}, { "encerraSessao": false });
            const copy: Session = {
                token: clean,
                "escopo": tokenScope(clean, list.items),
                orgs: list.items,
            };
            saveToTab(copy);
            setSession(copy);
            setExpired(false);
            return copy;
        } catch (error) {
            setToken(null);
            throw error;
        }
    }, []);

    const reloadOrgs = useCallback(async () => {
        const list = await request<OrgList>("/admin/v1/orgs?limit=200");
        setSession((current) => {
            if (!current) return current;
            const copy: Session = { ...current, orgs: list.items };
            saveToTab(copy);
            return copy;
        });
    }, []);

    const clearExpiryNotice = useCallback(() => setExpired(false), []);

    const value = useMemo<Context>(
        () => ({ "sessao": session, "expired": expired, "signIn": signIn, "signOut": signOut, "reloadOrgs": reloadOrgs, "clearExpiryNotice": clearExpiryNotice }),
        [session, expired, signIn, signOut, reloadOrgs, clearExpiryNotice],
    );

    return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}

export function useSession(): Context {
    const context = useContext(SessionContext);
    if (!context) {
        throw new Error("useSessao precisa estar dentro de <SessaoProvider>");
    }
    return context;
}

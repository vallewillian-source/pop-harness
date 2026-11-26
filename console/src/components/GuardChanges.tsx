
import {
    createContext,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useRef,
    useState,
    type ReactNode,
} from "react";
import { useNavigate } from "react-router-dom";
import { SofaButton, SofaModal } from "@sofacoding/sofa-ui";
import { useI18n } from "../i18n";

export type RecordGuard = {
    dirty: boolean;
    save: () => Promise<boolean>;
};

type Context = {
    register: (registro: RecordGuard | null) => void;
    tryNavigate: (para: string) => boolean;
};

const GuardContext = createContext<Context | null>(null);

export function useGuardChanges(): Context {
    const context = useContext(GuardContext);
    if (!context) {
        throw new Error("useGuardaDeAlteracoes exige GuardaAlteracoesProvider");
    }
    return context;
}

function targetLink(evento: MouseEvent): string | null {
    if (evento.defaultPrevented || evento.button !== 0) return null;
    if (evento.metaKey || evento.ctrlKey || evento.shiftKey || evento.altKey) return null;

    const target = evento.target as HTMLElement | null;
    const link = target?.closest?.("a[href]") as HTMLAnchorElement | null;
    if (!link || link.target === "_blank" || link.hasAttribute("download")) return null;

    let url: URL;
    try {
        url = new URL(link.href, window.location.origin);
    } catch {
        return null;
    }
    if (url.origin !== window.location.origin) return null;

    const current = `${window.location.pathname}${window.location.search}`;
    const destination = `${url.pathname}${url.search}`;
    if (destination === current) return null;
    return `${destination}${url.hash}`;
}

export function GuardChangesProvider({ children }: { children: ReactNode }) {
    const { translate } = useI18n();
    const navigate = useNavigate();
    const registro = useRef<RecordGuard | null>(null);
    const [destination, setTarget] = useState<string | null>(null);
    const [saving, setSaving] = useState(false);

    const register = useCallback((fresh: RecordGuard | null) => {
        registro.current = fresh;
    }, []);

    const dirty = destination === null ? (registro.current?.dirty ?? false) : true;

    useEffect(() => {
        const onExit = (evento: BeforeUnloadEvent) => {
            if (!registro.current?.dirty) return;
            evento.preventDefault();
            evento.returnValue = "";
        };
        window.addEventListener("beforeunload", onExit);
        return () => window.removeEventListener("beforeunload", onExit);
    }, []);

    useEffect(() => {
        const onClick = (evento: MouseEvent) => {
            if (!registro.current?.dirty) return;
            const para = targetLink(evento);
            if (!para) return;
            evento.preventDefault();
            evento.stopPropagation();
            setTarget(para);
        };
        document.addEventListener("click", onClick, true);
        return () => document.removeEventListener("click", onClick, true);
    }, []);

    const tryNavigate = useCallback(
        (para: string) => {
            if (registro.current?.dirty) {
                setTarget(para);
                return false;
            }
            navigate(para);
            return true;
        },
        [navigate],
    );

    const value = useMemo(() => ({ register, tryNavigate }), [register, tryNavigate]);

    async function saveExit() {
        const target = destination;
        if (!target || !registro.current) return;
        setSaving(true);
        try {
            const canExit = await registro.current.save();
            if (!canExit) {
                setTarget(null);
                return;
            }
            registro.current = null;
            setTarget(null);
            navigate(target);
        } finally {
            setSaving(false);
        }
    }

    return (
        <GuardContext.Provider value={value}>
            {children}
            <SofaModal
                open={destination !== null}
                onOpenChange={(isOpen) => {
                    if (!isOpen && !saving) setTarget(null);
                }}
                title={translate("shell.unsavedTitle")}
                description={
                    dirty
                        ? translate("shell.unsavedDirtyDescription")
                        : translate("shell.unsavedPendingDescription")
                }
                closeLabel={translate("shell.unsavedClose")}
                size="sm"
                id="poph-alteracoes-nao-salvas"
                footer={
                    <>
                        <SofaButton variant="secondary" disabled={saving} onClick={() => setTarget(null)}>
                            {translate("common.cancel")}
                        </SofaButton>
                        <SofaButton
                            variant="danger"
                            disabled={saving}
                            onClick={() => {
                                registro.current = null;
                                const target = destination;
                                setTarget(null);
                                if (target) navigate(target);
                            }}
                        >
                            {translate("shell.unsavedDiscard")}
                        </SofaButton>
                        <SofaButton
                            variant="primary"
                            loading={saving}
                            onClick={() => void saveExit()}
                        >
                            {translate("shell.unsavedSaveAndLeave")}
                        </SofaButton>
                    </>
                }
            >
                <p style={{ margin: 0, color: "var(--pop-text-secondary)" }}>
                    {translate("shell.unsavedBody")}
                </p>
            </SofaModal>
        </GuardContext.Provider>
    );
}

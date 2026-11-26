
import { useEffect, useState, type ReactNode } from "react";
import { useLocation } from "react-router-dom";
import { IconClose, SofaAdminShell, SofaModal } from "@sofacoding/sofa-ui";
import "../styles/shell.css";
import { useI18n } from "../i18n";
import { useSession } from "../session/Session";
import { Rail } from "../components/Rail";
import { Topbar } from "../components/Topbar";
import { useOrgAgents } from "../components/totalAgents";

export function Shell({ children }: { children: ReactNode }) {
    const { translate } = useI18n();
    const { sessao: session } = useSession();
    const { pathname } = useLocation();
    const [shortcuts, setShortcuts] = useState(false);
    const [railCompact, setRailCompact] = useState(false);
    const parts = pathname.split("/").filter(Boolean);
    const orgOpen = parts[0] === "orgs" ? (parts[1] ?? "") : "";
    const agents = useOrgAgents(orgOpen);
    const totalAgents = agents === null ? null : agents.length;

    useEffect(() => {
        const onPress = (evento: KeyboardEvent) => {
            const target = evento.target as HTMLElement | null;
            const typing =
                target instanceof HTMLInputElement ||
                target instanceof HTMLTextAreaElement ||
                target instanceof HTMLSelectElement ||
                target?.isContentEditable === true;
            if (evento.key === "?" && !typing) {
                evento.preventDefault();
                setShortcuts(true);
            }
        };
        window.addEventListener("keydown", onPress);
        return () => window.removeEventListener("keydown", onPress);
    }, []);

    const orgs = session?.orgs ?? [];
    const openShortcuts = () => setShortcuts(true);

    return (
        <>
            <div className="poph-shell">
                <Topbar orgs={orgs} onOpenShortcuts={openShortcuts} />
                <div className="poph-shell__body">
                    <SofaAdminShell
                        identity="settings"
                        rail={
                            <Rail
                                onOpenShortcuts={openShortcuts}
                                totalAgents={totalAgents}
                                agents={agents ?? []}
                                compact={railCompact}
                                onToggleCompact={() => setRailCompact((current) => !current)}
                            />
                        }
                    >
                        {children}
                    </SofaAdminShell>
                </div>
            </div>
            <SofaModal
                open={shortcuts}
                onOpenChange={setShortcuts}
                title={translate("shell.shortcutsTitle")}
                description={translate("shell.shortcutsDescription")}
                closeLabel={translate("shell.shortcutsClose")}
            >
                <div>
                    <div className="poph-shortcut">
                        <span>{translate("shell.shortcutNavigate")}</span>
                        <kbd>Tab</kbd> <kbd>Shift+Tab</kbd>
                    </div>
                    <div className="poph-shortcut">
                        <span>{translate("shell.shortcutActivate")}</span>
                        <kbd>Enter</kbd> <kbd>{translate("shell.shortcutSpaceKey")}</kbd>
                    </div>
                    <div className="poph-shortcut">
                        <span>{translate("shell.shortcutClose")}</span>
                        <kbd>Esc</kbd>
                    </div>
                    <div className="poph-shortcut">
                        <span>{translate("shell.shortcutOpen")}</span>
                        <kbd>?</kbd>
                    </div>
                </div>
            </SofaModal>
        </>
    );
}

export function CloseMark() {
    return <IconClose aria-hidden="true" />;
}

/**
 * Casca do Console (UX_SPEC §2.2): barra superior de largura total, menu lateral abaixo dela e o
 * modal de atalhos. O rail, a barra superior e a trilha vivem em arquivos próprios desde a RL-01.
 *
 * Desenho da HR-61: a barra atravessa a tela e o menu começa abaixo dela. A biblioteca põe menu e
 * barra lado a lado, então a coluna `poph-casca` fica aqui, no Console (`estilos/casca.css`).
 */

import { useEffect, useState, type ReactNode } from "react";
import { useLocation } from "react-router-dom";
import { IconClose, SofaAdminShell, SofaModal } from "@sofacoding/sofa-ui";
import "../estilos/casca.css";
import { useSessao } from "../sessao/Sessao";
import { Rail } from "./Rail";
import { Topbar } from "./Topbar";
import { useOrgAgents } from "./totalDeAgentes";

/** Casca com barra superior e menu; o conteúdo da rota entra em `children`. */
export function Shell({ children }: { children: ReactNode }) {
    const { sessao } = useSessao();
    const { pathname } = useLocation();
    const [atalhos, setAtalhos] = useState(false);
    const [railCompacta, setRailCompacta] = useState(false);
    // Organização aberta na URL (`/orgs/:id/...`): só ela tem contador de agentes.
    const partes = pathname.split("/").filter(Boolean);
    const orgAberta = partes[0] === "orgs" ? (partes[1] ?? "") : "";
    const agents = useOrgAgents(orgAberta);
    const totalDeAgentes = agents === null ? null : agents.length;

    useEffect(() => {
        const aoTeclar = (evento: KeyboardEvent) => {
            const alvo = evento.target as HTMLElement | null;
            const digitando =
                alvo instanceof HTMLInputElement ||
                alvo instanceof HTMLTextAreaElement ||
                alvo instanceof HTMLSelectElement ||
                alvo?.isContentEditable === true;
            if (evento.key === "?" && !digitando) {
                evento.preventDefault();
                setAtalhos(true);
            }
        };
        window.addEventListener("keydown", aoTeclar);
        return () => window.removeEventListener("keydown", aoTeclar);
    }, []);

    const orgs = sessao?.orgs ?? [];
    const abrirAtalhos = () => setAtalhos(true);

    return (
        <>
            <div className="poph-casca">
                <Topbar orgs={orgs} aoAbrirAtalhos={abrirAtalhos} />
                <div className="poph-casca__corpo">
                    <SofaAdminShell
                        identity="settings"
                        rail={
                            <Rail
                                aoAbrirAtalhos={abrirAtalhos}
                                totalDeAgentes={totalDeAgentes}
                                agents={agents ?? []}
                                compacta={railCompacta}
                                aoAlternarCompacta={() => setRailCompacta((atual) => !atual)}
                            />
                        }
                    >
                        {children}
                    </SofaAdminShell>
                </div>
            </div>
            <SofaModal
                open={atalhos}
                onOpenChange={setAtalhos}
                title="Atalhos do Console"
                description="Vale em qualquer tela. Atalhos de formulário entram com as telas de edição."
                closeLabel="Fechar atalhos"
            >
                <div>
                    <div className="poph-atalho">
                        <span>Navegar entre controles</span>
                        <kbd>Tab</kbd> <kbd>Shift+Tab</kbd>
                    </div>
                    <div className="poph-atalho">
                        <span>Acionar botão ou opção em foco</span>
                        <kbd>Enter</kbd> <kbd>Espaço</kbd>
                    </div>
                    <div className="poph-atalho">
                        <span>Fechar diálogo ou menu</span>
                        <kbd>Esc</kbd>
                    </div>
                    <div className="poph-atalho">
                        <span>Abrir esta lista</span>
                        <kbd>?</kbd>
                    </div>
                </div>
            </SofaModal>
        </>
    );
}

/** Botão de fechar reutilizável (o mesmo ícone do modal da biblioteca). */
export function IconeFechar() {
    return <IconClose aria-hidden="true" />;
}

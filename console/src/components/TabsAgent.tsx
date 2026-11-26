
import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { IconArrowLeft, SofaIconButton } from "@sofacoding/sofa-ui";
import { useGuardChanges } from "../components/GuardChanges";
import { publishNameAgent } from "../components/nameAgent";
import { publishAgentModel } from "../components/agentModel";
import { translate, useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import "../styles/agents.css";

export type IdTabAgent =
    | "instructions"
    | "knowledge-base"
    | "files"
    | "routing"
    | "integrations"
    | "model-jev"
    | "test-chat";

type Tab = {
    id: IdTabAgent;
    labelKey: TranslationKey;
    rota: string;
};

export const TABS_AGENT: readonly Tab[] = [
    { id: "instructions", labelKey: "agents.tabInstructions", rota: "instructions" },
    { id: "knowledge-base", labelKey: "agents.tabKnowledge", rota: "knowledge-base" },
    { id: "files", labelKey: "agents.tabFiles", rota: "files" },
    { id: "routing", labelKey: "agents.tabRouting", rota: "routing" },
    { id: "integrations", labelKey: "agents.tabIntegrations", rota: "integrations" },
    { id: "model-jev", labelKey: "agents.tabModelJev", rota: "model-jev" },
    { id: "test-chat", labelKey: "agents.tabTestChat", rota: "test-chat" },
] as const;

export type TabsAgentProps = {
    org: string;
    agente: string;
    active: IdTabAgent;
    nome?: string;
    model?: string | null;
};

export function TabsAgent({ org, agente, active, nome, model }: TabsAgentProps) {
    const { translate: t } = useI18n();
    const guard = useGuardChanges();
    const refs = useRef<Array<HTMLButtonElement | null>>([]);

    useEffect(() => {
        if (nome) publishNameAgent(agente, nome);
        publishAgentModel(agente, model ?? null);
    }, [agente, nome, model]);

    function trocar(id: IdTabAgent) {
        const tab = TABS_AGENT.find((candidate) => candidate.id === id);
        if (!tab || id === active) return;
        guard.tentarNavegar(
            `/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/${tab.rota}`,
        );
    }

    function moveFocus(index: number, evento: KeyboardEvent<HTMLButtonElement>) {
        const last = TABS_AGENT.length - 1;
        let destination: number;
        switch (evento.key) {
            case "ArrowRight":
                destination = index === last ? 0 : index + 1;
                break;
            case "ArrowLeft":
                destination = index === 0 ? last : index - 1;
                break;
            case "Home":
                destination = 0;
                break;
            case "End":
                destination = last;
                break;
            default:
                return;
        }
        evento.preventDefault();
        refs.current[destination]?.focus();
    }

    return (
        <div className="poph-tabs" role="tablist" aria-label={t("agents.tabsLabel")}>
            {TABS_AGENT.map((tab, index) => {
                const selected = tab.id === active;
                return (
                    <button
                        key={tab.id}
                        type="button"
                        role="tab"
                        aria-selected={selected}
                        aria-current={selected ? "page" : undefined}
                        tabIndex={selected ? 0 : -1}
                        className={selected ? "poph-tab poph-tab--active" : "poph-tab"}
                        ref={(no) => {
                            refs.current[index] = no;
                        }}
                        onClick={() => trocar(tab.id)}
                        onKeyDown={(evento) => moveFocus(index, evento)}
                    >
                        {t(tab.labelKey)}
                    </button>
                );
            })}
        </div>
    );
}

function toneSituation(situacao: string): string {
    switch (situacao) {
        case "active":
            return "poph-status--active";
        case "inactive":
        case "archived":
            return "poph-status--inactive";
        default:
            return "poph-status--inactive";
    }
}

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

export type HeaderAgentProps = {
    org: string;
    nome: string;
    slug?: string;
    situacao?: string;
    organizacao?: string | null;
    descricao?: ReactNode;
    acoes?: ReactNode;
};

export function HeaderAgent({
    org,
    nome,
    slug,
    situacao,
    organizacao: organization,
    descricao,
    acoes: actions,
}: HeaderAgentProps) {
    const { translate: t } = useI18n();
    const guard = useGuardChanges();

    return (
        <div className="poph-page__top poph-agent__top">
            <div className="poph-agent__identity">
                <SofaIconButton
                    label={t("agents.backToAgentsLabel")}
                    onClick={() =>
                        guard.tentarNavegar(
                            `/orgs/${encodeURIComponent(org)}/agents`,
                        )
                    }
                >
                    <IconArrowLeft aria-hidden="true" />
                </SofaIconButton>
                <div className="poph-agent__text">
                    <p className="poph-agent__context">
                        {organization
                            ? t("agents.agentContextWithOrg", { org: organization })
                            : t("agents.agentContext")}
                    </p>
                    <h1 className="poph-page__title">{nome}</h1>
                    <p className="poph-page__description poph-agent__meta">
                        {situacao && (
                            <span className={`poph-status ${toneSituation(situacao)}`}>
                                <span className="poph-status__point" aria-hidden="true" />
                                {labelSituation(situacao)}
                            </span>
                        )}
                        {slug && <span className="poph-mono-muted">{slug}</span>}
                        {descricao && <span>{descricao}</span>}
                    </p>
                </div>
            </div>
            {actions && <div className="poph-page__actions">{actions}</div>}
        </div>
    );
}

export type SectionAgentProps = {
    titulo: string;
    descricao?: ReactNode;
    acoes?: ReactNode;
    children: ReactNode;
};

export function SectionAgent({ titulo: title, descricao, acoes: actions, children }: SectionAgentProps) {
    const idTitle = useId();
    return (
        <section className="poph-section poph-agent__section" aria-labelledby={idTitle}>
            <div className="poph-agent__section-header">
                <div className="poph-agent__section-text">
                    <h2 id={idTitle} className="poph-section__title">
                        {title}
                    </h2>
                    {descricao && <p className="poph-agent__section-help">{descricao}</p>}
                </div>
                {actions && <div className="poph-agent__section-actions">{actions}</div>}
            </div>
            <div className="poph-section__content">{children}</div>
        </section>
    );
}
